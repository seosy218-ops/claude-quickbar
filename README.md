# quickbar

A small floating bar for the Claude desktop app on Windows that types the phrases you use often into the Code session in front of you, one click each, as if you had typed them yourself: plain words such as `continue`, a slash command such as `/compact`, or a `/skill` followed by a sentence of what you want.

![The bar folded out beside Claude's Local and folder chips: /clear, yes, /compact, /wait-what and +](assets/expanded.png)

> [!IMPORTANT]
> **Unofficial.** quickbar is a third-party tool, not made, endorsed or supported by Anthropic. It works by finding Claude's prompt box on screen and typing into it, so an update to the Claude app may break it at any time.

## Install

1. Download `quickbar.exe` from the [latest release](https://github.com/seosy218-ops/claude-quickbar/releases/latest) and put it somewhere it can stay, such as `%LOCALAPPDATA%\quickbar\`. There is no installer.
2. Run it. The exe is not code-signed, so Windows may say **"Windows protected your PC"**: click **More info**, then **Run anyway**.
3. Open Claude. The bar shows on Claude's window whenever Claude is in front.

Only one copy runs at a time; starting it again does nothing.

### Build from source

With [Rust](https://rustup.rs) 1.88 or later:

```bash
cd desktop
cargo build --release
```

The exe lands in `desktop/target/release/quickbar.exe`.

## Use

- **⚡** folds the phrase buttons out and back in.
- **Click a phrase** to send it to Claude's Code prompt box. Whatever you had half-typed there is put back afterwards, and your clipboard is left as it was (the phrase does not show up in Win+V history either).
- **`+`** adds a phrase. Each phrase has the text to type (anything you would type yourself, e.g. `continue` or `/compact keep the plan`), an optional label for the button, and a switch **"Fill in only, don't send"** that leaves the phrase in the box, ahead of your draft, for you to finish and send.
- **Right-click a phrase** to edit or delete it. Right-clicking any button, ⚡ and + included, also offers **Quit**. The gaps between buttons are see-through: a click there lands on Claude.
- **Drag a phrase** to move it among the others.
- **Drag the ⚡** to move the bar. It stays at that spot relative to the nearest corner of Claude's window, so it follows Claude when Claude is moved or resized.
- **Tray icon** (a ⚡ in the notification area): **Start with Windows** and **Quit**. Start with Windows starts this very exe at sign-in, so move the exe first, then tick it.

![The box for adding or editing a phrase: the phrase, an optional label, and the fill-in-only switch](assets/add-command.png)

The bar follows Claude's light or dark theme, and hides while Claude is minimized or behind other windows.

## Configure

Everything the bar does is saved in `%APPDATA%\quickbar\config.json`, written the first time quickbar runs. You can edit it by hand; quickbar reads it at start, so quit from the tray and start it again afterwards.

```json
{
  "position": {
    "corner": "top_right",
    "x": -160,
    "y": 4
  },
  "commands": [
    {
      "command": "/compact",
      "mode": "send"
    },
    {
      "command": "continue",
      "mode": "send"
    },
    {
      "command": "/review",
      "label": "Review",
      "mode": "fill"
    }
  ]
}
```

| Field | Meaning | If left out |
|---|---|---|
| `position.corner` | The corner of Claude's window the bar keeps its distance from: `top_left`, `top_right`, `bottom_left` or `bottom_right`. | `top_right` |
| `position.x`, `position.y` | From that corner of Claude's window to the same corner of the bar, in pixels at 100% scaling. Negative is left or up. | `-160`, `4`: in Claude's title bar, left of the window buttons |
| `commands[].command` | The phrase: the text typed into the prompt box, exactly as you would type it (plain words or a slash command). Required. | — |
| `commands[].label` | The button's text. | The phrase text |
| `commands[].mode` | `send` submits the phrase; `fill` leaves it in the box without submitting. | `send` |

Each entry of `commands` is one phrase; the names `commands` and `command` stay as they are so older files keep working. With no `commands` at all, the bar starts with `/compact`, `/clear` and `continue`.

Field names only ever get added, never renamed or removed, so a config file written for an older version keeps working. If the file cannot be read, the bar says **"Config file is invalid"**, runs with the defaults, and leaves your file untouched until you fix it.

## Limits

- Windows only, and only the Claude desktop app's **Code** page. On a chat or on settings the bar says **"No prompt box to type in"** and types nothing.
- It is a separate program rather than a Claude Code plugin because the desktop app does not let plugins draw anything in its window.

## Uninstall

Untick **Start with Windows** in the tray menu, **Quit**, then delete the exe and `%APPDATA%\quickbar\`.

## License

[MIT](LICENSE)
