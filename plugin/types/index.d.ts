/** One button of the bar: what it types, what it shows, and whether it sends. */
export type Phrase = {
  /** The text typed, as is: plain words, or a slash command with its arguments. */
  text: string
  /** The button's label; the text when absent. */
  label?: string
  /** `send` submits the phrase; `fill` leaves `phrase draft` in the box unsent. */
  mode: 'send' | 'fill'
}

/** The add / edit pane's fields as typed so far; `index` is the phrase edited, null when adding. */
export type Form = { index: number | null; text: string; label: string; mode: Phrase['mode'] }

declare module 'claude-code' {
  interface PluginState {
    quickbar: {
      phrases: Phrase[] | null
      isClosed: boolean
      isEditing: boolean
      selected: number | null
      form: Form | null
    }
  }
}
