// The bar above the prompt: a bolt that folds the row, one button per phrase, then + and ✎.
// Pressing a phrase types it as the person would (see `press`). ✎ turns on edit mode, where
// pressing a phrase selects it and ◀ ▶ ✎ ✕ act on the selection; + and ✎ open the edit pane.

import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register } from 'claude-code'

import type { Form, Phrase } from '../types'

/** The phrases session.start loaded; null before it ran and again after /clear. */
const phrases = atom({ plugin: 'quickbar', key: 'phrases' } as const, null)
const isClosed = atom({ plugin: 'quickbar', key: 'isClosed' } as const, false)
const isEditing = atom({ plugin: 'quickbar', key: 'isEditing' } as const, false)
/** The phrase ◀ ▶ ✎ ✕ act on, by its place in the list. */
const selected = atom({ plugin: 'quickbar', key: 'selected' } as const, null)
const form = atom({ plugin: 'quickbar', key: 'form' } as const, null)

const PANE = 'quickbar-edit'

const STORE_KEY = 'phrases'

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
    rows: 6,
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

/** Types the phrase as the person would: a slash command runs, words are sent, a fill phrase goes before the draft. */
async function press($: EngineInterface, phrase: Phrase) {
  if (phrase.mode === 'fill') {
    const { text: draft } = await $.prompt.read()
    await $.prompt.fill({ text: `${phrase.text} ${draft}`, mode: 'replace' })
    return
  }
  const slash = /^\/(\S+)\s*([\s\S]*)$/.exec(phrase.text)
  if (slash !== null) {
    await $.command.run({ command: slash[1], args: slash[2] })
    return
  }
  await $.prompt.submit({ text: phrase.text, asUser: true })
}

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    const out = await next(e)
    // Only an empty store gets the defaults: whatever else is there stays for the person to fix.
    if ((await $.store.get(STORE_KEY)) === undefined) {
      await $.store.set(STORE_KEY, DEFAULTS)
    }
    const loaded = (await stored($)) ?? DEFAULTS
    await update($, phrases, () => loaded)
    return out
  })

  on('ui.render', { component: 'AbovePrompt' }, async ($, e, next) => {
    if (e.props.hasSurvey) {
      return next(e)
    }

    const list = await current($)
    const closed = await read($, isClosed)
    const editing = await read($, isEditing)
    const chosen = editing ? await read($, selected) : null
    const { Box, Button } = $.ui.resolve(e)

    return (
      <Box flexDirection="row" flexWrap="wrap" gap={1}>
        <Button
          key="toggle"
          label="⚡"
          plain
          onPress={async () => {
            // Folding leaves edit mode, so the row comes back typing phrases.
            await update($, isEditing, () => false)
            await update($, selected, () => null)
            await update($, isClosed, was => !was)
          }}
        />
        {!closed &&
          list.map((phrase, i) => (
            <Button
              key={`phrase-${i}`}
              label={phrase.label ?? phrase.text}
              variant={i === chosen ? 'primary' : undefined}
              onPress={() => (editing ? update($, selected, () => i) : press($, phrase))}
            />
          ))}
        {!closed && chosen !== null && chosen < list.length && [
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
        {!closed && (
          <Button
            key="add"
            label="+"
            plain
            dimColor
            onPress={() => openPane($, { index: null, text: '', label: '', mode: 'send' })}
          />
        )}
        {!closed && (
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
        )}
      </Box>
    )
  })

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const held = await read($, form)
    const { Box, Button, Input, Text } = $.ui.resolve(e)
    if (held === null) {
      return <Text dimColor>Nothing to edit.</Text>
    }

    return (
      <Box flexDirection="column" gap={1}>
        <Input
          key="text"
          label="Phrase"
          placeholder="continue, /compact, or a /skill and what to ask"
          value={held.text}
          autoFocus
          onInput={value => typeInto($, 'text', value)}
          onSubmit={async value => {
            await typeInto($, 'text', value)
            await submit($)
          }}
        />
        <Input
          key="label"
          label="Label"
          placeholder="optional; the phrase when empty"
          value={held.label}
          onInput={value => typeInto($, 'label', value)}
          onSubmit={async value => {
            await typeInto($, 'label', value)
            await submit($)
          }}
        />
        <Box flexDirection="row" gap={1}>
          <Button
            key="mode"
            label={held.mode === 'send' ? 'Click sends' : 'Fill only, no send'}
            onPress={() =>
              update($, form, was =>
                was === null ? was : { ...was, mode: was.mode === 'send' ? 'fill' : 'send' },
              )
            }
          />
          <Button key="save" label="Save" variant="primary" onPress={() => submit($)} />
        </Box>
      </Box>
    )
  })
}
