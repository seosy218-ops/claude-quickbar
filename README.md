# quickbar

A row of phrase buttons above the prompt box in the Claude desktop app's **Code** tab. One click types a phrase you use often into the session in front of you, as if you had typed it yourself: plain words such as `continue`, a slash command such as `/compact`, or a `/skill` followed by a sentence of what you want.

quickbar is a Claude Code plugin: Claude Code draws the buttons with its own parts, so they look like the rest of the Code tab in light and dark themes.

> [!IMPORTANT]
> **Unofficial.** quickbar is a third-party plugin, not made, endorsed or supported by Anthropic. It uses Claude Code's plugin hooks, which are still young, so an update to Claude may break it at any time.

## Requirements

- The Claude desktop app, Code tab. The terminal version of Claude Code may work but is not supported.
- Claude Code 2.1.286 or later. Below 2.1.287 plugin hooks are off by default: add this to `%USERPROFILE%\.claude\settings.json` (keep whatever else is there), then start a new session.

  ```json
  {
    "env": {
      "CLAUDE_CODE_ENABLE_FUNCTION_HOOKS": "1"
    }
  }
  ```

  The desktop app's Claude Code version is the folder name under `%APPDATA%\Claude\claude-code\`.

## Install

Run these two commands with the `claude` CLI. If you only have the desktop app, use the copy it ships: `%APPDATA%\Claude\claude-code\<version>\<hash>\claude.exe`.

```bash
claude plugin marketplace add seosy218-ops/claude-quickbar
```

```bash
claude plugin install quickbar@quickbar
```

Then open a new session in the Code tab. The bar shows above the prompt box.

## Use

- **⚡** folds the phrase buttons out and back in.
- **Click a phrase** to type it into the session. A slash command runs as if typed; plain words are sent. Whatever you had half-typed in the prompt box stays there.
- **`+`** adds a phrase: the text to type (anything you would type yourself, e.g. `continue` or `/compact keep the plan`), an optional label for the button, and a switch between **Click sends** and **Fill only, no send**. A fill-only phrase is put in the prompt box ahead of your draft, for you to finish and send.
- **✎** turns on edit mode. There, clicking a phrase selects it instead of sending it, and four buttons act on the selection: **◀ ▶** move it, **✎** edits it, **✕** deletes it. **Done** or ⚡ leaves edit mode.

The first session starts with `/compact`, `/clear` and `continue`. Your phrases are kept in `%USERPROFILE%\.claude\plugins\store\` and are the same in every session and project.

## Upgrading from the exe version

quickbar 0.2 and earlier was a separate `quickbar.exe`. The plugin replaces it and does not read its settings, so add your phrases again with `+`. To remove the exe:

1. In its tray icon (a ⚡ in the notification area), untick **Start with Windows**, then **Quit**.
2. Delete `quickbar.exe` and `%APPDATA%\quickbar\`.

## Turn off or uninstall

Disable or remove it in `/plugin`, or with the CLI:

```bash
claude plugin uninstall quickbar@quickbar
```

Uninstalling leaves your saved phrases in `%USERPROFILE%\.claude\plugins\store\`; delete quickbar's file there to forget them.

## License

[MIT](LICENSE)
