---
name: skarn-migration-guard
description: Enforce zero-data-loss migration rules when editing Tauri identity, Inno Setup installer, settings.rs, keyring.rs, env vars, localStorage stores, or cross-process IPC event channels during the Selora-to-Skarn migration.
---

# Skarn Migration Guard

Load `references/identities.md` before modifying any persistence, installer, credential, or IPC file.

## Non-Negotiable Rules

1. **Settings JSON key alias (silent-reset trap):** When renaming `pub selora: SeloraSettings` to `pub skarn: ...`, you MUST add `#[serde(alias = "selora")]` for v1.0. Write the new key; accept the old key on read.
2. **Settings directory (`%APPDATA%\selora-ide` -> `%APPDATA%\skarn`):** Copy tree only if new dir is absent and old dir exists; write `migrated_from: "selora-ide"` + timestamp; leave old dir intact as read-only fallback for v1.0.
3. **Windows Credential Manager (`selora-ide` -> `skarn-ide`):** Strict sequence: **read old -> write new -> verify write -> delete old**. Log one redacted line on success; surface one actionable UI prompt on failure. Never leave plaintext `.api_key` behind after migration succeeds.
4. **Inno Setup `AppId` & `DefaultDirName`:** KEEP the existing GUID `{7A3E9C41-5D2B-4E8F-B6A1-9C0D2E4F6A8B}` and `DefaultDirName` for v1.0 so upgrades happen in place with one uninstall entry. Folder relocation is deferred to v1.1.
5. **Environment variables:** Add `SKARN_*` and keep reading `SELORA_*` with a deprecation log for v1.0.
6. **11 `localStorage` keys:** Run a single table-driven bootstrap migration BEFORE any store reads a key. Preserve parameterized suffixes verbatim (`editor-session:<path>`, `agent-draft:<json>`, `agents-workspace:v1:<id>`). Never let `onboarding_seen` reset.
7. **5 IPC event channels:** Rename ALL Rust emit sites and TypeScript listen sites in **one single commit** (or add temporary dual-emit). Never leave a half-renamed window.

## Mandatory Verification

Before claiming any Stage 2 task complete, run:
```bash
node .agents/skills/skarn-migration-guard/scripts/check-ipc-parity.mjs
```
