import { describe, expect, mock, test } from 'claude-code/testing'
import type { On } from 'claude-code'

const props = { hasSurvey: false, isWorking: false, maxRows: 4, bodyColumns: 80 }

/** Records what reaches the engine beneath the plugin: runs, submits and fills, with `draft` in the box and `tools` offered. */
function engine(on: On, draft = '', tools: string[] = []) {
  const seen: { runs: unknown[]; submits: unknown[]; fills: unknown[] } = { runs: [], submits: [], fills: [] }
  on('command.run', ($, e) => {
    seen.runs.push({ command: e.command, args: e.args })
    return { text: '' }
  })
  on('prompt.submit', ($, e) => {
    seen.submits.push({ text: e.text })
    return { text: e.text }
  })
  on('prompt.read', () => ({ value: { text: draft, cursor: draft.length } }))
  on('tool.list', () => ({ value: tools.map(name => ({ name, description: name, isMcp: true })) }))
  on('prompt.fill', ($, e) => {
    seen.fills.push({ text: e.text, mode: e.mode })
    return { isFilled: true, text: e.text, cursor: e.text.length }
  })
  return seen
}

/** A store in memory beneath the plugin, returning every value it was set to. */
function store(on: On, entries: Record<string, unknown> = {}) {
  const held = new Map(Object.entries(entries))
  const saved: unknown[] = []
  on('store.get', ($, e) => ({ value: held.get(e.key) }))
  on('store.set', ($, e) => {
    held.set(e.key, e.value)
    saved.push(e.value)
    return { value: undefined }
  })
  return saved
}

const pane = {
  title: 'Add a phrase',
  isFocused: true,
  bodyColumns: 80,
  placement: 'inline' as const,
  scroll: { offset: 0, bodyRows: 6 },
  view: {},
}

/** Answers the pane calls beneath the plugin, returning the ids opened and closed. */
function panes(on: On) {
  const seen: { opened: string[]; closed: string[] } = { opened: [], closed: [] }
  on('ui.open', ($, e) => {
    seen.opened.push(e.id)
    return { value: { isPlaced: true } }
  })
  on('ui.close', ($, e) => {
    seen.closed.push(e.id)
    return { value: undefined }
  })
  return seen
}

for (const surface of ['terminal', 'desktop'] as const) {
  describe(surface, () => {
    test('first start saves the default phrases and draws them', async ($, on) => {
      const saved = store(on)
      on('session.start', ($, e) => ({ cwd: e.cwd }))
      await $.session.start({ cwd: '.', surface, isInteractive: true })
      expect(saved).toEqual([
        [
          { text: '/compact', mode: 'send' },
          { text: '/clear', mode: 'send' },
          { text: 'continue', mode: 'send' },
        ],
      ])
      const ui = await $.ui.mount({ plugin: 'quickbar', surface, component: 'AbovePrompt', props })
      expect(await ui.find({ key: 'phrase-0', text: '/compact' })).toBeDefined()
      expect(await ui.find({ key: 'phrase-2', text: 'continue' })).toBeDefined()
      await ui.unmount()
    })

    test('a saved list is kept, not replaced by the defaults', async ($, on) => {
      const saved = store(on, { phrases: [{ text: 'go on', label: 'Go', mode: 'send' }] })
      on('session.start', ($, e) => ({ cwd: e.cwd }))
      await $.session.start({ cwd: '.', surface, isInteractive: true })
      expect(saved).toEqual([])
      const ui = await $.ui.mount({ plugin: 'quickbar', surface, component: 'AbovePrompt', props })
      expect(await ui.find({ key: 'phrase-0', text: 'Go' })).toBeDefined()
      expect(await ui.find({ key: 'phrase-1' })).toBeUndefined()
      await ui.unmount()
    })

    test('a store holding something else is left alone and the defaults are drawn', async ($, on) => {
      const saved = store(on, { phrases: 'hand-edited' })
      on('session.start', ($, e) => ({ cwd: e.cwd }))
      await $.session.start({ cwd: '.', surface, isInteractive: true })
      expect(saved).toEqual([])
      const ui = await $.ui.mount({ plugin: 'quickbar', surface, component: 'AbovePrompt', props })
      expect(await ui.find({ key: 'phrase-0', text: '/compact' })).toBeDefined()
      await ui.unmount()
    })

    test('the bolt hides and shows the phrases', async ($, on) => {
      mock.store(on)
      const ui = await $.ui.mount({ plugin: 'quickbar', surface, component: 'AbovePrompt', props })
      await ui.press({ key: 'toggle' })
      expect(await ui.find({ key: 'phrase-0' })).toBeUndefined()
      expect(await ui.find({ key: 'toggle' })).toBeDefined()
      await ui.press({ key: 'toggle' })
      expect(await ui.find({ key: 'phrase-0' })).toBeDefined()
      await ui.unmount()
    })

    test('a slash phrase runs the command, args and all', async ($, on) => {
      mock.store(on, { phrases: [{ text: '/compact keep the plan', mode: 'send' }, { text: '/clear', mode: 'send' }] })
      const seen = engine(on, 'half a draft')
      const ui = await $.ui.mount({ plugin: 'quickbar', surface, component: 'AbovePrompt', props })
      await ui.press({ key: 'phrase-0' })
      await ui.press({ key: 'phrase-1' })
      expect(seen.runs).toEqual([
        { command: 'compact', args: 'keep the plan' },
        { command: 'clear', args: '' },
      ])
      expect(seen.submits).toEqual([])
      expect(seen.fills).toEqual([])
      await ui.unmount()
    })

    test('/clear asks the desktop app to clear, then ends a turn for it', async ($, on) => {
      mock.store(on, { phrases: [{ text: '/clear', mode: 'send' }] })
      const seen = engine(on, 'half a draft', ['mcp__ccd_session_mgmt__clear_session'])
      const calls: unknown[] = []
      on('tool.call', ($, e) => {
        calls.push({ tool: e.tool, session_id: (e as { session_id?: string }).session_id })
        return { result: 'queued' }
      })
      const ui = await $.ui.mount({ plugin: 'quickbar', surface, component: 'AbovePrompt', props })
      await ui.press({ key: 'phrase-0' })
      expect(calls).toEqual([{ tool: 'mcp__ccd_session_mgmt__clear_session', session_id: 'self' }])
      expect(seen.runs).toEqual([{ command: 'quickbar-clear', args: '' }])
      expect(seen.fills).toEqual([])
      await ui.unmount()
    })

    test('a send still running shows on its button and a second press is not sent', async ($, on) => {
      mock.store(on, { phrases: [{ text: '/compact', mode: 'send' }, { text: 'continue', mode: 'send' }] })
      let ui: Awaited<ReturnType<typeof $.ui.mount>> | undefined
      const runs: string[] = []
      const during: (string | undefined)[] = []
      on('command.run', async ($, e) => {
        runs.push(e.command)
        during.push((await ui?.find({ key: 'phrase-0' }))?.text)
        await ui?.press({ key: 'phrase-0' })
        await ui?.press({ key: 'phrase-1' })
        return { text: '' }
      })
      const submits: string[] = []
      on('prompt.submit', ($, e) => {
        submits.push(e.text)
        return { text: e.text }
      })
      ui = await $.ui.mount({ plugin: 'quickbar', surface, component: 'AbovePrompt', props })
      await ui.press({ key: 'phrase-0' })
      expect(runs).toEqual(['compact'])
      expect(submits).toEqual([])
      expect(during).toEqual(['/compact …'])
      expect((await ui.find({ key: 'phrase-0' }))?.text).toBe('/compact')
      await ui.unmount()
    })

    test('a plain phrase is submitted and the draft is left alone', async ($, on) => {
      mock.store(on, { phrases: [{ text: 'continue', mode: 'send' }] })
      const seen = engine(on, 'half a draft')
      const ui = await $.ui.mount({ plugin: 'quickbar', surface, component: 'AbovePrompt', props })
      await ui.press({ key: 'phrase-0' })
      expect(seen.submits).toEqual([{ text: 'continue' }])
      expect(seen.runs).toEqual([])
      expect(seen.fills).toEqual([])
      await ui.unmount()
    })

    test('a fill phrase goes before the draft, unsent', async ($, on) => {
      mock.store(on, { phrases: [{ text: 'review:', mode: 'fill' }, { text: '/skill grill', mode: 'fill' }] })
      const seen = engine(on, 'half a draft')
      const ui = await $.ui.mount({ plugin: 'quickbar', surface, component: 'AbovePrompt', props })
      await ui.press({ key: 'phrase-0' })
      await ui.press({ key: 'phrase-1' })
      expect(seen.fills).toEqual([
        { text: 'review: half a draft', mode: 'replace' },
        { text: '/skill grill half a draft', mode: 'replace' },
      ])
      expect(seen.submits).toEqual([])
      expect(seen.runs).toEqual([])
      await ui.unmount()
    })

    test('a fill phrase into an empty box leaves the cursor after a space', async ($, on) => {
      mock.store(on, { phrases: [{ text: 'review:', mode: 'fill' }] })
      const seen = engine(on)
      const ui = await $.ui.mount({ plugin: 'quickbar', surface, component: 'AbovePrompt', props })
      await ui.press({ key: 'phrase-0' })
      expect(seen.fills).toEqual([{ text: 'review: ', mode: 'replace' }])
      await ui.unmount()
    })

    test('in edit mode a press selects the phrase instead of sending it', async ($, on) => {
      store(on, { phrases: [{ text: 'continue', mode: 'send' }] })
      const seen = engine(on)
      const ui = await $.ui.mount({ plugin: 'quickbar', surface, component: 'AbovePrompt', props })
      expect(await ui.find({ key: 'left' })).toBeUndefined()
      await ui.press({ key: 'edit' })
      expect(await ui.find({ key: 'edit', text: 'Done' })).toBeDefined()
      await ui.press({ key: 'phrase-0' })
      expect(seen.submits).toEqual([])
      expect(await ui.find({ key: 'left' })).toBeDefined()
      expect(await ui.find({ key: 'delete' })).toBeDefined()
      await ui.press({ key: 'edit' })
      expect(await ui.find({ key: 'left' })).toBeUndefined()
      await ui.press({ key: 'phrase-0' })
      expect(seen.submits).toEqual([{ text: 'continue' }])
      await ui.unmount()
    })

    test('◀ and ▶ move the selected phrase and save the order', async ($, on) => {
      const saved = store(on, { phrases: [{ text: 'a', mode: 'send' }, { text: 'b', mode: 'send' }, { text: 'c', mode: 'send' }] })
      const ui = await $.ui.mount({ plugin: 'quickbar', surface, component: 'AbovePrompt', props })
      await ui.press({ key: 'edit' })
      await ui.press({ key: 'phrase-2' })
      await ui.press({ key: 'left' })
      expect(await ui.find({ key: 'phrase-1', text: 'c' })).toBeDefined()
      await ui.press({ key: 'left' })
      await ui.press({ key: 'left' })
      expect(await ui.find({ key: 'phrase-0', text: 'c' })).toBeDefined()
      await ui.press({ key: 'right' })
      expect(saved.at(-1)).toEqual([
        { text: 'a', mode: 'send' },
        { text: 'c', mode: 'send' },
        { text: 'b', mode: 'send' },
      ])
      await ui.unmount()
    })

    test('✕ deletes the selected phrase at once', async ($, on) => {
      const saved = store(on, { phrases: [{ text: 'a', mode: 'send' }, { text: 'b', mode: 'send' }] })
      const ui = await $.ui.mount({ plugin: 'quickbar', surface, component: 'AbovePrompt', props })
      await ui.press({ key: 'edit' })
      await ui.press({ key: 'phrase-0' })
      await ui.press({ key: 'delete' })
      expect(saved.at(-1)).toEqual([{ text: 'b', mode: 'send' }])
      expect(await ui.find({ key: 'phrase-0', text: 'b' })).toBeDefined()
      expect(await ui.find({ key: 'phrase-1' })).toBeUndefined()
      expect(await ui.find({ key: 'delete' })).toBeUndefined()
      await ui.unmount()
    })

    test('+ opens the pane and saving adds a fill phrase with a label', async ($, on) => {
      const saved = store(on, { phrases: [{ text: 'a', mode: 'send' }] })
      const seen = panes(on)
      const bar = await $.ui.mount({ plugin: 'quickbar', surface, component: 'AbovePrompt', props })
      await bar.press({ key: 'add' })
      expect(seen.opened).toEqual(['quickbar-edit'])
      const editor = await $.ui.mount({ plugin: 'quickbar', surface, component: 'Pane', props: pane, requestId: 'quickbar-edit' })
      await editor.input({ key: 'text', text: '/skill grill', kind: 'change' })
      await editor.input({ key: 'label', text: 'Grill', kind: 'change' })
      expect(await editor.find({ key: 'mode', text: 'Click sends' })).toBeDefined()
      await editor.press({ key: 'mode' })
      expect(await editor.find({ key: 'mode', text: 'Fill only, no send' })).toBeDefined()
      await editor.press({ key: 'save' })
      expect(saved.at(-1)).toEqual([
        { text: 'a', mode: 'send' },
        { text: '/skill grill', label: 'Grill', mode: 'fill' },
      ])
      expect(seen.closed).toEqual(['quickbar-edit'])
      expect(await bar.find({ key: 'phrase-1', text: 'Grill' })).toBeDefined()
      await editor.unmount()
      await bar.unmount()
    })

    test('✎ opens the pane on the selected phrase and saving changes it in place', async ($, on) => {
      const saved = store(on, { phrases: [{ text: 'a', mode: 'send' }, { text: 'b', label: 'B', mode: 'fill' }] })
      panes(on)
      const bar = await $.ui.mount({ plugin: 'quickbar', surface, component: 'AbovePrompt', props })
      await bar.press({ key: 'edit' })
      await bar.press({ key: 'phrase-1' })
      await bar.press({ key: 'change' })
      const editor = await $.ui.mount({ plugin: 'quickbar', surface, component: 'Pane', props: pane, requestId: 'quickbar-edit' })
      expect(await editor.find({ key: 'text', text: 'b' })).toBeDefined()
      expect(await editor.find({ key: 'label', text: 'B' })).toBeDefined()
      expect(await editor.find({ key: 'mode', text: 'Fill only, no send' })).toBeDefined()
      await editor.input({ key: 'label', text: '', kind: 'change' })
      await editor.input({ key: 'text', text: 'bee' })
      expect(saved.at(-1)).toEqual([
        { text: 'a', mode: 'send' },
        { text: 'bee', mode: 'fill' },
      ])
      await editor.unmount()
      await bar.unmount()
    })

    test('folding leaves edit mode', async ($, on) => {
      store(on, { phrases: [{ text: 'continue', mode: 'send' }] })
      const seen = engine(on)
      const ui = await $.ui.mount({ plugin: 'quickbar', surface, component: 'AbovePrompt', props })
      await ui.press({ key: 'edit' })
      await ui.press({ key: 'phrase-0' })
      await ui.press({ key: 'toggle' })
      await ui.press({ key: 'toggle' })
      expect(await ui.find({ key: 'edit', text: 'Done' })).toBeUndefined()
      expect(await ui.find({ key: 'left' })).toBeUndefined()
      await ui.press({ key: 'phrase-0' })
      expect(seen.submits).toEqual([{ text: 'continue' }])
      await ui.unmount()
    })

    test('a phrase with no text is not saved', async ($, on) => {
      const saved = store(on, { phrases: [] })
      const seen = panes(on)
      const bar = await $.ui.mount({ plugin: 'quickbar', surface, component: 'AbovePrompt', props })
      await bar.press({ key: 'add' })
      const editor = await $.ui.mount({ plugin: 'quickbar', surface, component: 'Pane', props: pane, requestId: 'quickbar-edit' })
      await editor.input({ key: 'text', text: '   ', kind: 'change' })
      await editor.press({ key: 'save' })
      expect(saved).toEqual([])
      expect(seen.closed).toEqual([])
      await editor.unmount()
      await bar.unmount()
    })

    test('a survey takes the place of the bar', async ($, on) => {
      mock.store(on)
      on('ui.render', ($, e) => {
        const { Box } = $.ui.resolve(e)
        return h(Box, { key: 'engine' })
      })
      const ui = await $.ui.mount({ plugin: 'quickbar', surface, component: 'AbovePrompt', props: { ...props, hasSurvey: true } })
      expect(await ui.find({ key: 'toggle' })).toBeUndefined()
      await ui.unmount()
    })
  })
}
