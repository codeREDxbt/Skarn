# Skarn Breaking Identities & Persisted Contracts

## Fixed v1.0 Identity Tokens
- Product Name: `Skarn`
- Lowercase token: `skarn`
- Tauri Identifier: `app.skarn.desktop` (from `lol.selora.ide`)
- Installer Output: `Skarn-Setup-<version>.exe` (from `Selora-IDE-Setup-<version>.exe`)
- Installer AppId (v1.0): KEEP `{7A3E9C41-5D2B-4E8F-B6A1-9C0D2E4F6A8B}`
- Settings Dir: `%APPDATA%\skarn` (from `%APPDATA%\selora-ide`)
- Keyring Service: `skarn-ide` (from `selora-ide`; entries `api-key`, `brave-key`, plus new Decision BYOK entries)
- Env Prefix: `SKARN_` (fallback read `SELORA_` with deprecation warning for v1.0)
- Release Version: `1.0.0`

## The 11 `localStorage` Keys (Migrate Table-Driven Before Any Store Reads)
1. `selora:model_settings`
2. `selora:agent-pinned-sessions`
3. `selora:agent-draft:<key>` *(parameterized — preserve suffix verbatim)*
4. `selora:editor-session:<path>` *(parameterized — preserve suffix verbatim)*
5. `selora:last-editor-workspace`
6. `selora:editor-preferences`
7. `selora:recent-commands`
8. `selora:agents-workspace:v1:<id>` *(parameterized — preserve suffix verbatim)*
9. `selora:ui-sounds`
10. `selora:notification_history`
11. `selora:onboarding_seen`

## The 5 Cross-Process IPC Event Channels (Rename in One Atomic Commit)
| Legacy Channel | Target Channel | Rust Sites | Frontend Sites |
|---|---|---|---|
| `selora://event` | `skarn://event` | `src-tauri/src/state.rs` | `frontend/src/bridge/bridge.ts` |
| `selora://agent-preferences` | `skarn://agent-preferences` | `src-tauri/src/agents_preferences.rs` | `frontend/src/bridge/bridge.ts` |
| `selora://agents-navigate` | `skarn://agents-navigate` | `src-tauri/src/agents_window.rs` | `frontend/src/bridge/bridge.ts`, `agents-bridge.test.ts` |
| `selora://main-session-navigate` | `skarn://main-session-navigate` | `src-tauri/src/agents_window.rs` | `frontend/src/bridge/bridge.ts` |
| `selora:update-progress` | `skarn:update-progress` | `src-tauri/src/updates.rs` | `frontend/src/bridge/bridge.ts` |

## Theme ID Migration
- Map persisted `selora-dark` -> `skarn-dark` on read; write `skarn-dark`.
