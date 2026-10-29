# Selora IDE

Selora IDE is a Windows desktop code editor with a workspace explorer, Git tools, an embedded terminal, language intelligence, and an AI agent workbench.

## Download

<https://github.com/ToxicAngelX/Selora-IDE/releases/latest>

Run `Selora-IDE-Setup-0.1.0.exe`. The installer is per-user and does not need
administrator access. It installs WebView2 when Windows does not already have it.

## First run

1. Open Selora IDE.
2. Choose **Open Folder** and select a project.
3. Open a file from the **Files** sidebar.
4. Press `Ctrl+S` to save, or `Ctrl+L` to open the agent dock.

AI features need a provider key. Open **Settings > AI Gateway** or **Connect**,
choose a provider, enter the gateway URL and key, then test the connection.
Presets cover the Selora gateway, Anthropic, OpenAI, DeepSeek, OpenRouter, GLM,
Ollama, and custom OpenAI-compatible gateways.

## Features

### Editor and workspace

- CodeMirror 6 editor with multi-file tabs, dirty-state indicators, save, and
  optional Auto Save.
- Syntax highlighting for Rust, JavaScript, TypeScript, JSX/TSX, CSS, JSON, and
  Markdown, plus breadcrumbs, a minimap, and editor preferences.
- Find and Replace in the editor search panel, with Replace All.
- Workspace search with case-sensitive, whole-word, and regular-expression
  options. Results open at the matching line.
- Explorer actions to create files and folders, rename, and delete. **Delete is
  permanent** and does not use the Recycle Bin.
- Command palette, File/Edit/View/Help menus, and a searchable shortcut
  reference. Open files restore after restart; unsaved contents do not.

### Terminal

Embedded terminal tabs run through Windows ConPTY, using PowerShell 7 when
installed and Windows PowerShell otherwise. Shells start in the workspace root,
text can be copied and pasted, and file paths in output open in the editor.

### Git

Stage, unstage, discard, and commit changes. Create, switch, merge, and delete
local branches; fetch, pull, and push; save, apply, pop, and drop stashes.
Browse commit history and open commit details. Changed files open as
syntax-highlighted unified or split diffs. Merge conflicts can be resolved with
**Ours** or **Theirs**, or opened in the editor.

### Agent workbench

- Multi-session chat with persisted transcripts and session switching.
- Session permission modes: **Ask**, **Agent**, and **Plan**. Agent mode applies
  file writes immediately; commands still require approval. Plan is read-only.
- Approval cards for file changes, commands, plans, and workflow scripts.
- Plan cards, live todos, reasoning details, markdown replies, syntax-highlighted
  code blocks, copy, retry, and prompt rewind/edit.
- Attach workspace files to a prompt by drag and drop.
- Subagent sessions with live transcripts and artifacts. Running children appear
  as clickable pets in the title bar; clicking one opens its session.

### Workflows

Agent-requested workflow scripts run as multi-phase jobs with named agents,
per-step status, approvals, artifacts, and a run log. The Workflows panel shows
the phase graph, step progress, run list, and token and cost accounting. Runs can
be stopped or resumed; resume warns when earlier side effects may exist.

### Language intelligence and diagnostics

Rust and TypeScript language servers provide hover, go-to-definition, and
completion. Diagnostics appear inline in the editor and in the Problems panel,
which filters by severity and text. Clicking a problem opens the file at its line.

### Settings and themes

Settings sections: AI Gateway, Editor, Theme, and Keybindings. Themes are Selora
Cosmic Dark, Selora Moonlight Light, and System. Settings also cover reduced
motion, automatic update checks, editor preferences, and LSP restarts. Four
shortcuts are rebindable, with conflict warnings and a searchable shortcut table.

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl+Shift+P` | Command palette |
| `Ctrl+P` | Find in Files |
| `Ctrl+B` | Toggle sidebar |
| `Ctrl+J` | Toggle bottom dock |
| `Ctrl+L` | Agent dock |
| `Ctrl+S` | Save |
| `Ctrl+F` | Find in editor |
| `F12` | Go to definition |

## Build from source

Requirements: Windows 10 or 11 (64-bit) with WebView2, Rust stable with the MSVC
toolchain, Node.js 22, and Inno Setup 6.3 or newer for the installer.

```powershell
npm --prefix apps/selora-desktop/frontend ci
npm --prefix apps/selora-desktop/frontend run build
cargo build --release -p selora-desktop --locked
```

Run the built app with `start.bat`. Build the installer with
`installer\prebuild.bat`; the output is written to `installer\output\`.

## Privacy and credentials

Do not commit API keys or `.env` files. Selora stores provider credentials in
Windows Credential Manager when available.

## License

MIT. See [LICENSE](LICENSE).