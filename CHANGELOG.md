# Changelog

All notable changes to Selora IDE are documented here. This project follows
[Keep a Changelog](https://keepachangelog.com/) and semantic versioning.

## [0.1.0] - 2026-10-29

Public Windows release. The installer is `Selora-IDE-Setup-0.1.0.exe`, published
at <https://github.com/ToxicAngelX/Selora-IDE/releases/latest>.

### Added

- Editor: CodeMirror 6 with multi-file tabs, dirty-state indicators, save, Auto
  Save, breadcrumbs, a minimap, configurable font size, tab size, and word wrap.
- Syntax highlighting for Rust, JavaScript, TypeScript, JSX/TSX, CSS, JSON, and
  Markdown.
- Editor find and replace with match navigation and Replace All.
- Workspace explorer: create files and folders, rename, and permanently delete
  entries. Delete does not use the Recycle Bin.
- Workspace search with case-sensitive, whole-word, and regular-expression
  options; results open at the matching line.
- Command palette, File/Edit/View/Help menus, editor tab context menus, and a
  searchable keyboard shortcut reference.
- Session restore for open files in a workspace. Unsaved contents are not
  restored.
- Terminal: embedded ConPTY tabs with PowerShell 7 or Windows PowerShell, starting
  in the workspace root. Copy and paste work, and file paths in output open in the
  editor.
- Git: stage, unstage, discard, and commit; create, switch, merge, and delete
  branches; fetch, pull, and push; save, apply, pop, and drop stashes; browse
  commit history and details; view unified or split diffs; resolve conflicts with
  Ours or Theirs.
- Agent workbench: multi-session chat with persisted transcripts, session
  switching, and Ask, Agent, and Plan permission modes.
- Approval cards for file changes, commands, plans, and workflow scripts, with
  unified or split diff views.
- Plan cards, live todos, reasoning details, markdown replies, syntax-highlighted
  code blocks, copy, retry, prompt rewind/edit, and drag-and-drop workspace file
  attachments.
- Subagent sessions with live transcripts and artifacts. Running children appear
  as clickable pets in the title bar.
- Workflows: multi-phase agent scripts with named agents, per-step status,
  approvals, artifacts, a run log, token and cost accounting, and Stop and Resume
  controls. Resume warns when earlier side effects may exist.
- Language intelligence for Rust and TypeScript through language servers: hover,
  go-to-definition, and completion.
- Diagnostics inline in the editor and in the Problems panel, with severity and
  text filters and click-to-open.
- Settings for AI Gateway, Editor, Theme, and Keybindings.
- Themes: Selora Cosmic Dark, Selora Moonlight Light, and System.
- Reduced motion, automatic update checks, editor preferences, LSP restart
  controls, and four rebindable shortcuts with conflict warnings.
- Provider presets for the Selora gateway, Anthropic, OpenAI, DeepSeek,
  OpenRouter, GLM, Ollama, and custom OpenAI-compatible gateways.
- In-app update checks against GitHub Releases, with installer download and
  signature verification.
- Per-user Windows installer that does not require administrator access and
  installs WebView2 when it is missing.
- Provider credentials are stored in Windows Credential Manager when available.