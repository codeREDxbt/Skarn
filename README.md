# Skarn

Skarn is a Windows desktop code editor and AI agent workbench built with Rust, WebView2, and CodeMirror 6. It combines a workspace explorer, Git tools, an embedded ConPTY terminal, Rust/TypeScript language intelligence, and a deterministic, checkpointed AI agent runtime.

This project began as Selora IDE and is currently on the `skarn/migration-baseline` branch (Stages 0–3 implemented).

---

## Why Skarn?

Most AI coding tools either bolt chat onto an editor without real execution boundaries, or run autonomous loops where a bad edit or command is hard to audit and harder to roll back. Skarn is built around a few core ideas:

- **Deterministic decision kernel (`skarn-decision`)**: Permission checks, trust levels, and mode rules live in a standalone Rust kernel with property-based (`proptest`) proofs and shadow replay rather than scattered UI conditionals.
- **Content-addressed filesystem mutations & turn undo**: Agent file writes go through a CAS-backed checkpoint layer so any turn can be cleanly undone without relying solely on `git checkout`.
- **Explicit 4-mode control**: Users choose between **Ask**, **Plan**, **Agent**, and **Auto**—each with clear, enforced boundaries on what the agent can read, write, or execute.
- **Scoped approvals & observability**: Approvals can be scoped rather than all-or-nothing, and the trace drawer exposes what the agent and decision kernel actually did.

---

## Current Status: Honest Pros & Cons

### Pros (Implemented Now)
- **Fast native shell**: Rust backend + WebView2 frontend with low overhead compared to Electron.
- **Real 4-mode agent workbench**: **Ask**, **Plan**, **Agent**, and **Auto** modes wired end-to-end with scoped approvals, clocked questions, turn undo/checkpoints, and a trace drawer.
- **Solid core editor**: CodeMirror 6 with multi-file tabs, breadcrumbs, minimap, Find/Replace, workspace regex search, and Rust/TypeScript LSP diagnostics + go-to-definition.
- **Built-in Git & ConPTY terminal**: Stage/unstage/commit, branch/stash management, unified/split diffs, Ours/Theirs conflict resolution, and PowerShell 7 / Windows PowerShell tabs.
- **Multi-provider AI gateway**: Presets for Anthropic, OpenAI, DeepSeek, OpenRouter, GLM, Ollama, and custom OpenAI-compatible endpoints, with keys stored in Windows Credential Manager when available.

### Cons & Current Limitations
- **Windows-only**: Built around WebView2, Windows ConPTY, and Windows Credential Manager; no macOS or Linux builds yet.
- **Permanent file deletion**: Explorer **Delete** bypasses the Recycle Bin and deletes permanently.
- **Unsaved buffers are not persisted**: Open file tabs restore after restart, but unsaved editor contents do not.
- **Limited LSP coverage**: Language intelligence currently targets Rust and TypeScript; other languages rely on syntax highlighting only.
- **Limited keybinding customization**: Only four shortcuts are currently rebindable.
- **Active migration**: The codebase is still mid-migration from Selora IDE (`skarn/migration-baseline`); some internal docs and legacy naming are still being cleaned up.

---

## Currently Implemented (Stages 0–3)

### Stage 0–2: Foundation, Rebrand & UI Cleanup
- Isolated updater and removed legacy Selora gateway.
- Mechanical rename (`selora` → `skarn`) and v1.0.0 identity.
- Consolidated stylesheets, repaired `tokens.css` craft constitution, enforced region routing, and removed duplicate home views.

### Stage 3: Decision Kernel & 4-Mode Workbench
- Standalone `skarn-decision` Rust crate with `proptest` proofs and shadow replay.
- CAS-backed filesystem mutations, live trust tracking, and per-turn checkpoints/undo.
- 4-mode composer (**Ask**, **Plan**, **Agent**, **Auto**), scoped approval cards, clocked questions, and trace drawer.
- Multi-session chat, subagent sessions (with title-bar pet indicators), and multi-phase workflow scripts with token/cost accounting.

---

## Planned in Next Stages

- **Recycle Bin & safer file ops**: Move Explorer deletions to the Windows Recycle Bin instead of permanent delete.
- **Unsaved buffer recovery**: Persist dirty/unsaved editor buffers across restarts.
- **Expanded LSP & language support**: Add language server support beyond Rust and TypeScript.
- **Full keybinding customization**: Make all editor and workbench shortcuts rebindable.
- **Release & updater polish**: Finalize signed `Skarn-Setup` installer releases and automated update verification.

---

## First Run

1. Open Skarn.
2. Choose **Open Folder** and select a project.
3. Open a file from the **Files** sidebar.
4. Press `Ctrl+S` to save, or `Ctrl+L` to open the agent dock.

AI features require a provider key. Open **Settings > AI Gateway** or **Connect**, choose a provider, enter the gateway URL and key, and test the connection.

## Keyboard Shortcuts

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

## Build from Source

Requirements: Windows 10 or 11 (64-bit) with WebView2, Rust stable with the MSVC toolchain, Node.js 22, and Inno Setup 6.3+ for the installer.

```powershell
npm --prefix apps/skarn-desktop/frontend ci
npm --prefix apps/skarn-desktop/frontend run build
cargo build --release -p skarn-desktop --locked
```

Run the built app with `start.bat`. Build the installer with `installer\prebuild.bat`; output is written to `installer\output\`.

## Privacy & Credentials

Do not commit API keys or `.env` files. Skarn stores provider credentials in Windows Credential Manager when available.

## License

MIT. See [LICENSE](LICENSE).
