// The bar above the prompt: one button per phrase, then + and ✎.
// Pressing a phrase types it as the person would (see `press`). ✎ turns on edit mode, where
// pressing a phrase selects it and ◀ ▶ ✎ ✕ act on the selection; + and ✎ open the edit pane.

import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register } from 'claude-code'

import type { Form, Phrase } from '../types'

/** The phrases session.start loaded; null before it ran and again after /clear. */
const phrases = atom({ plugin: 'quickbar', key: 'phrases' } as const, null)
const isEditing = atom({ plugin: 'quickbar', key: 'isEditing' } as const, false)
/** The phrase ◀ ▶ ✎ ✕ act on, by its place in the list. */
const selected = atom({ plugin: 'quickbar', key: 'selected' } as const, null)
const form = atom({ plugin: 'quickbar', key: 'form' } as const, null)
/** The phrase whose send is still running, by its place in the list. */
const running = atom({ plugin: 'quickbar', key: 'running' } as const, null)
/** True while the main conversation compacts, however it was asked for. */
const isCompacting = atom({ plugin: 'quickbar', key: 'isCompacting' } as const, false)

const PANE = 'quickbar-edit'

const STORE_KEY = 'phrases'

/** The two ways a press can go, as the pane offers them, worded for someone who has not met them. */
const MODES: { value: Phrase['mode']; label: string; hint: string }[] = [
  { value: 'send', label: 'Send right away', hint: 'Sends at once; your draft stays' },
  { value: 'fill', label: 'Put in the box only', hint: 'Goes before your draft, unsent' },
]

const DEFAULTS: Phrase[] = [
  { text: '/compact', mode: 'send' },
  { text: '/clear', mode: 'send' },
  { text: 'continue', mode: 'send' },
]

/** The saved phrases; undefined when none are saved, or what is saved is not a list. */
async function stored($: EngineInterface): Promise<Phrase[] | undefined> {
  const value = await $.store.get(STORE_KEY)
  return Array.isArray(value) ? (value as Phrase[]) : undefined
}

/** The list the bar draws; after /clear the session holds none and session.start does not run again. */
async function current($: EngineInterface): Promise<Phrase[]> {
  return (await read($, phrases)) ?? (await stored($)) ?? DEFAULTS
}

/** Changes the saved list by `change`: the store first, then what the bar draws. */
async function save($: EngineInterface, change: (list: Phrase[]) => Phrase[]) {
  // Changed as drawn, so a selection by place means the phrase the person saw.
  const list = change(await current($))
  await $.store.set(STORE_KEY, list)
  await update($, phrases, () => list)
}

/** Moves the selected phrase `by` places, keeping it selected. */
async function move($: EngineInterface, by: -1 | 1) {
  const from = await read($, selected)
  if (from === null) return
  let to = from
  await save($, list => {
    if (from >= list.length) return list
    to =Math.min(Math.max(from + by, 0), list.length - 1)
    const next = [...list]
    const [phrase] = next.splice(from, 1)
    next.splice(to, 0, phrase)
    return next
  })
  await update($, selected, () => to)
}

/** Opens the pane on `filled`: a new phrase, or one of the list to change. */
async function openPane($: EngineInterface, filled: Form) {
  await update($, form, () => filled)
  await $.ui.open({
    id: PANE,
    title: filled.index === null ? 'Add a phrase' : 'Edit the phrase',
    focus: true,
    closeOnEscape: true,
    holdToasts: true,
    // A desktop draws a field and a button taller than a terminal row: ask for room for every row.
    rows: 20,
  })
}

/** Keeps what the person typed into one field of the pane. */
function typeInto($: EngineInterface, field: 'text' | 'label', value: string) {
  return update($, form, was => (was === null ? was : { ...was, [field]: value }))
}

/** Saves what the pane holds and closes it; a phrase with no text is not saved. */
async function submit($: EngineInterface) {
  const held = await read($, form)
  if (held === null) return
  const text = held.text.trim()
  if (text === '') {
    $.ui.toast('A phrase needs some text')
    return
  }
  const label = held.label.trim()
  const phrase: Phrase = label === '' ? { text, mode: held.mode } : { text, label, mode: held.mode }
  await save($, list =>
    held.index === null ? [...list, phrase] : list.map((one, i) => (i === held.index ? phrase : one)),
  )
  await update($, form, () => null)
  await $.ui.close({ id: PANE })
}

/** Closes the pane and drops what it held, saving nothing. */
async function cancel($: EngineInterface) {
  await update($, form, () => null)
  await $.ui.close({ id: PANE })
}

/** The desktop app's own /clear, offered to the session as a tool. */
const DESKTOP_CLEAR = 'mcp__ccd_session_mgmt__clear_session'

/** A command of ours that does nothing: its run is the turn ending that a queued desktop clear waits for. */
const CLEAR_COMMAND = 'quickbar-clear'

/** Long enough to stay up while a compaction usually runs; a click takes it off sooner. */
const COMPACT_TOAST_MS = 15000

async function hasTool($: EngineInterface, name: string) {
  return (await $.tool.list()).some(tool => tool.name === name)
}

/** Types the phrase as the person would: a slash command runs, words are sent, a fill phrase goes before the draft. */
async function press($: EngineInterface, phrase: Phrase) {
  if (phrase.mode === 'fill') {
    const { text: draft } = await $.prompt.read()
    await $.prompt.fill({ text: `${phrase.text} ${draft}`, mode: 'replace' })
    return
  }
  const slash = /^\/(\S+)\s*([\s\S]*)$/.exec(phrase.text)
  if (slash !== null && slash[1] === 'clear' && (await hasTool($, DESKTOP_CLEAR))) {
    // The desktop handles a typed /clear itself; run through the engine it empties the context
    // but leaves the old conversation on screen. The desktop's own clear waits for a turn to end,
    // and between turns none will, so an empty command of ours ends one.
    await $.tool.call({ tool: DESKTOP_CLEAR, session_id: 'self', consent: 'The user pressed "/clear" on the quickbar' })
    await $.command.run({ command: CLEAR_COMMAND })
    return
  }
  if (slash !== null && slash[1] === 'compact') {
    // Run through the engine, the desktop shows none of its own compacting notice, only the one
    // when it is done, and the desktop offers no compact of its own to call instead.
    $.ui.toast('Compacting the conversation…', { timeoutMs: COMPACT_TOAST_MS })
  }
  if (slash !== null) {
    await $.command.run({ command: slash[1], args: slash[2] })
    return
  }
  await $.prompt.submit({ text: phrase.text, asUser: true })
}

/** A slash command such as /compact shows nothing until it ends, so the button stays busy till then and a second press is not sent. */
async function pressOnce($: EngineInterface, phrase: Phrase, i: number) {
  if (phrase.mode === 'send' && ((await read($, running)) !== null || (await read($, isCompacting)))) {
    $.ui.toast('Still working on the last one')
    return
  }
  if (phrase.mode === 'fill') return press($, phrase)
  await update($, running, () => i)
  try {
    await press($, phrase)
  } finally {
    await update($, running, () => null)
  }
  // The click left the focus on the button; writing nothing to the box hands it back there,
  // as a fill phrase's own write does, so the person types on without clicking the box.
  await $.prompt.fill({ text: '', mode: 'append' })
}

const isCompact = (phrase: Phrase) => /^\/compact(\s|$)/.test(phrase.text)

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    const out = await next(e)
    // Only an empty store gets the defaults: whatever else is there stays for the person to fix.
    if ((await $.store.get(STORE_KEY)) === undefined) {
      await $.store.set(STORE_KEY, DEFAULTS)
    }
    const loaded = (await stored($)) ?? DEFAULTS
    await update($, phrases, () => loaded)
    await $.command.register({ name: CLEAR_COMMAND, description: 'Ends the turn a quickbar /clear waits for; does nothing on its own' })
    return out
  })

  on('command.run', { command: CLEAR_COMMAND }, async $ => {
    return { text: '' }
  })

  on('session.compact', async ($, e, next) => {
    if (e.trigger === 'precompute' || e.agentId !== undefined) return next(e)
    await update($, isCompacting, () => true)
    try {
      return await next(e)
    } finally {
      await update($, isCompacting, () => false)
    }
  })

  on('ui.render', { component: 'AbovePrompt' }, async ($, e, next) => {
    if (e.props.hasSurvey) {
      return next(e)
    }

    const list = await current($)
    const editing = await read($, isEditing)
    const chosen = editing ? await read($, selected) : null
    const busy = await read($, running)
    const compacting = await read($, isCompacting)
    const { Box, Button } = $.ui.resolve(e)

    return (
      <Box flexDirection="row" flexWrap="wrap" gap={1}>
        {list.map((phrase, i) => (
          <Button
            key={`phrase-${i}`}
            label={
              i === busy || (compacting && isCompact(phrase))
                ? `${phrase.label ?? phrase.text} …`
                : (phrase.label ?? phrase.text)
            }
            variant={i === chosen ? 'primary' : undefined}
            onPress={() => (editing ? update($, selected, () => i) : pressOnce($, phrase, i))}
          />
        ))}
        {chosen !== null && chosen < list.length && [
          <Button key="left" label="◀" plain dimColor onPress={() => move($, -1)} />,
          <Button key="right" label="▶" plain dimColor onPress={() => move($, 1)} />,
          <Button
            key="change"
            label="✎"
            plain
            dimColor
            onPress={() => {
              const phrase = list[chosen]
              return openPane($, { index: chosen, text: phrase.text, label: phrase.label ?? '', mode: phrase.mode })
            }}
          />,
          <Button
            key="delete"
            label="✕"
            plain
            dimColor
            onPress={async () => {
              await save($, all => all.filter((_, i) => i !== chosen))
              await update($, selected, () => null)
            }}
          />,
        ]}
        <Button
          key="add"
          label="+"
          plain
          dimColor
          onPress={() => openPane($, { index: null, text: '', label: '', mode: 'send' })}
        />
        <Button
          key="edit"
          label={editing ? 'Done' : '✎'}
          plain
          dimColor
          onPress={async () => {
            await update($, selected, () => null)
            await update($, isEditing, was => !was)
          }}
        />
      </Box>
    )
  })

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const held = await read($, form)
    const { Box, Button, Input, Text } = $.ui.resolve(e)
    if (held === null) {
      return <Text dimColor>Nothing to edit.</Text>
    }

    // Laid out as the desktop's own menus are: a dim heading over each part, and the two ways a
    // press can go as rows of a name and a dim line on what it does, a check on the one picked.
    // The whole form sits in the middle of the rows the pane shows, not at its top. The body is only
    // as tall as its tree, so the height is asked for in rows; a taller form still grows and scrolls.
    return (
      <Box flexDirection="column" minHeight={e.props.scroll.bodyRows} justifyContent="center" paddingY={1}>
        <Box flexDirection="column" gap={2}>
          <Box flexDirection="column" gap={1}>
            <Text dimColor>Phrase</Text>
            <Input
              key="text"
              placeholder="continue, /compact, /skill …"
              value={held.text}
              autoFocus
              onInput={value => typeInto($, 'text', value)}
              onSubmit={async value => {
                await typeInto($, 'text', value)
                await submit($)
              }}
            />
          </Box>
          <Box flexDirection="column" gap={1}>
            <Text dimColor>Label</Text>
            <Input
              key="label"
              placeholder="Optional; shows the phrase"
              value={held.label}
              onInput={value => typeInto($, 'label', value)}
              onSubmit={async value => {
                await typeInto($, 'label', value)
                await submit($)
              }}
            />
          </Box>
          <Box flexDirection="column" gap={1}>
            <Text dimColor>When pressed</Text>
            {MODES.map(mode => (
              <Box key={`row-${mode.value}`} flexDirection="row">
                <Box width={2}>
                  <Text>{held.mode === mode.value ? '✓' : ''}</Text>
                </Box>
                <Box flexDirection="column">
                  <Button
                    key={`mode-${mode.value}`}
                    label={mode.label}
                    plain
                    onPress={() => update($, form, was => (was === null ? was : { ...was, mode: mode.value }))}
                  />
                  <Text dimColor>{mode.hint}</Text>
                </Box>
              </Box>
            ))}
          </Box>
          <Box flexDirection="row" gap={1}>
            <Button key="cancel" label="Cancel" onPress={() => cancel($)} />
            <Button key="save" label="Save" variant="primary" onPress={() => submit($)} />
          </Box>
        </Box>
      </Box>
    )
  })
}
