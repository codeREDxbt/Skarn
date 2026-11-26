# Skarn — Phase 0 Forensic Audit & Stage-Gate Execution Brief

> **Baseline commit:** `626b90e` (`chore: pre-skarn baseline with .agents hooks and blueprint`)
> **Workspace state at lock:** Clean working tree; `.agents/` installed at repository root with `setup-abs-paths.mjs` paths active; stray Vite timestamp artifact removed.
> **Mode:** Phase 0 Plan Locked (D1–D7) — Executing Stage 0 (`S0-1` & `S0-2`).

---

## Owner Decision Lock Status

### Locked Decisions (Approved by Owner)

1. **Workspace & Git Baseline:** `.agents/` moved to repo root, `node .agents/hooks/setup-abs-paths.mjs` executed, stray `apps/selora-desktop/frontend/vite.config.ts.timestamp-*` deleted, clean baseline branch secured at `626b90e`.
2. **Legacy Repository Control (`ToxicAngelX/Selora-IDE`):** <!-- skarn-allow-legacy-fixture --> Owner **does NOT control** the legacy repository and the new repository is **not live yet**. Stage 0 (`S0-1`) **must disable `check_for_update` / `install_update` (Rust + frontend auto-check/About fetch)** until `codeREDxbt/Skarn` goes live with `v1.0.0`, while repointing all manifest URLs cleanly so no code references the old slug.
3. **Brand & Release Identity:**
   - **Product / Display Name:** `Skarn` (confirmed; NOT `Skarnify`). Descriptor where secondary copy is used: `Skarn -- code workbench`.
   - **Target Repository URL:** `https://github.com/codeREDxbt/Skarn` (`https://github.com/codeREDxbt/Skarn.git` in `apps/selora-desktop/frontend/package.json`; normalizes to slug `codeREDxbt/Skarn` under `scripts/check-release-metadata.ps1:115-125`).
   - **Legal `AppPublisher`:** `codeREDxbt` (replacing `Toxic` in `installer/selora-ide.iss:51`).
   - **Code-Signing Certificate:** Issuance started separately; CI release workflow keeps `SIGNING_REQUIRED` failing on unsigned release builds while local developer builds (`installer/prebuild.bat:14-16`) remain allowed unsigned.
   - **Third-Party Frontend Fx Packages:** `img-fx` and `metal-fx` cleared (verified MIT npm dependencies, not Rust crates).
4. **4-Mode Permission Evolution:** Approved to evolve `AgentModeDto` (`crates/selora-protocol/src/lib.rs:237-244`) to the 4 canonical modes (**Read-only**, **Ask before changes**, **Auto-edit**, **Full access**) during **Stage 3 (Decision Engine)**, deleting duplicate composer mode rows (`MODE_OPTIONS` vs `COMPOSER_MODES`).
5. **Phase 0 Scope Additions & External Provider Guardrail:**
   - Added `crates/selora-ai/src/agent/tools/fs.rs:884` (`.<name>.selora-bak` -> `.<name>.skarn-bak`, together with `temp_sibling` `:870` `.<name>.selora-tmp-<pid>-<seq>` -> `.<name>.skarn-tmp-<pid>-<seq>` and companion docs/tests) to **Card S1-2** for `--strict` legacy scan compliance.
   - Added the 12th `localStorage` key (`agents:last-context-tab` in `AgentsContextPane.tsx:17` -> `skarn:agents-last-context-tab`) explicitly to **Card S1-2**'s migration table.
   - Confirmed for **TypeSafe Jev**, **Google Gemini Decisions API**, and **Laya**: **zero** fabrication of unverified REST schema fields or runtime model formats; keep unverified boundaries behind Rust adapter traits, deterministic/adversarial mock/dev runtimes, and wire-level contract fixtures until validated live.

### Resolved Decisions (D4, D5, D7 — Now LOCKED by Owner)

6. **Docs Host URL (`F1` / About) — D4 LOCKED:** `https://github.com/codeREDxbt/Skarn/tree/main/docs` (permitted by `EXTERNAL_URL_HOST_ALLOWLIST` in `state.rs:1029`).
7. **`ConnectModal.tsx` Preset Roster Scope (`S0-2`) — D5 LOCKED (Option A):** Keep all 7 non-gateway BYOK presets with `anthropic`, `openai`, `ollama`, `custom` featured first, followed by `deepseek`, `openrouter` (re-tagged `'Aggregator'`), and `glm`; delete `selora`, `api.selora.lol`, `sk-selora-`/`sk-gw-`, and `'Recommended'`; set NO provider preselected on first run. *(Note: `known_disabled_providers` in `state.rs:962-966` safely ignores stale disabled IDs on load.)* <!-- skarn-allow-legacy-fixture -->
8. **Stage 0 Hygiene — D7 LOCKED:** Delete stray root `nul` only if present; leave `.zcode/`, `graphify-out/`, and archived research `.md` files untouched during Stage 0 (cleanup deferred to final release).

---

## A. Baseline Script Results (Verified on Commit `626b90e`)

| Script | Exit Code | Exact Measured Output |
|---|---|---|
| `node .agents/skills/skarn-supply-chain-gate/scripts/check-legacy-refs.mjs` | `1` (**FAIL — P0**) | **27 P0 hits** across **16 files** (and 432 total candidate text files): **14** `[legacy-repo-slug]` (`ToxicAngelX/Selora-IDE`) + **13** `[legacy-gateway]` (`api.selora.lol`) <!-- skarn-allow-legacy-fixture --> |
| `node .agents/skills/skarn-migration-guard/scripts/check-ipc-parity.mjs` | `0` (**OK**) | `IPC Channel Parity OK (all 5 Rust <-> TS channels aligned).` |
| `node .agents/skills/skarn-craft-constitution/scripts/css-audit.mjs` | `0` (**OK**) | **28** CSS files, **20,899** total lines: **3** `*-polish.css` files (`agent-card-polish.css`, `agent-feed-polish.css`, `agent-session-tabs-polish.css`), **75** `transition: all`, **222** glow/glass/grad refs, **167** raw px `border-radius`, **73** `!important`. |
| `node .agents/skills/skarn-decision-invariants/scripts/check-decision-hygiene.mjs` | `0` (**OK**) | `Decision hygiene: no decision source files found yet (OK).` |

**Supplementary `--strict` `selora` census (via Node walk mirroring `check-legacy-refs.mjs` rules on `626b90e`):**
- **389 files** / **13,668 matching lines** total outside allowlisted paths.
- **9,904 lines (72.5%)** live inside generated `graphify-out/` (`graphify-out/graph.json`: `9,807`, `graphify-out/manifest.json`: `97`), which `check-legacy-refs.mjs:16-18` does not skip in `SKIP_DIRS`.
- **3,764 lines** across active Rust, TypeScript, CSS, batch/PS1, Inno Setup, YAML, TOML, and Markdown files (top hotspots: `sessions.rs` `419`, `state.rs` `410`, `crates/selora-app/src/shell.rs` `213`, `crates/selora-app/src/app_state.rs` `154`, `crates/selora-app/src/wiring.rs` `153`, `commands.ts` `123`, `crates/selora-ai/src/client.rs` `106`, `crates/selora-settings/src/settings.rs` `90`).

### Exact Breakdown of All 27 P0 Hits (`626b90e`)

- **14 `[legacy-repo-slug]` (`ToxicAngelX/Selora-IDE`):** <!-- skarn-allow-legacy-fixture -->
  - `Cargo.toml:23` (`repository`), `Cargo.toml:24` (`homepage`)
  - `README.md:7`
  - `installer/selora-ide.iss:22` (`#define RepoUrl`)
  - `apps/selora-desktop/frontend/package.json:8` (`repository.url`)
  - `apps/selora-desktop/frontend/src/app/App.tsx:94`
  - `apps/selora-desktop/frontend/src/features/agents-workspace/AgentsWindowApp.tsx:177`
  - `crates/selora-ai/src/updates.rs:325` (unit test fixture `html_url`)
  - `crates/selora-app/src/shell.rs:4115` (unit test fixture `html_url`)
  - `apps/selora-desktop/src-tauri/src/state.rs:6049,6051,6055,6058` (`open_external_allowlist_accepts_github_and_rejects_dangerous_urls` test) + `state.rs:5784` (`[legacy-gateway]`)
- **13 `[legacy-gateway]` (`api.selora.lol`):** <!-- skarn-allow-legacy-fixture -->
  - `crates/selora-settings/src/settings.rs:67` (`default_base_url()`) and tests at `:793,985,1002,1190,1327`
  - `crates/selora-app/src/wiring.rs:47` (`pub const DEFAULT_BASE_URL`)
  - `crates/selora-ai/src/lib.rs:1` (crate doc comment)
  - `crates/selora-ai/src/client.rs:550` (doc comment)
  - `crates/selora-ai/examples/smoke.rs:19` (`unwrap_or_else` fallback URL)
  - `apps/selora-desktop/src-tauri/src/workflows_host.rs:193` (doc comment)
  - `apps/selora-desktop/src-tauri/src/state.rs:5784` (test assertion on `persisted.selora.base_url`)
  - `apps/selora-desktop/frontend/src/features/auth/ConnectModal.tsx:56` (`PROVIDER_PRESETS` `'selora'` `baseUrl`) and `:257` (`useState<string>`)

---

## B. Contradictions Log (Docs vs. Actual Codebase)

| # | Spec Claim (`SKARN_MASTER_BLUEPRINT.md` / `REBRAND_PLAN.md`) | Actual Repository Ground Truth (`file:line`) | Action Impact |
|---|---|---|---|
| 1 | References `lib.rs` in Tauri shell / shared choke-point list (`src-tauri/src/lib.rs`) | `apps/selora-desktop/src-tauri/src/lib.rs` **does not exist**. The Tauri crate is binary-only at `apps/selora-desktop/src-tauri/src/main.rs:1-265`. The protocol `lib.rs` is `crates/selora-protocol/src/lib.rs`. | Choke-point sequencing separates `crates/selora-protocol/src/lib.rs` (DTO contract) from `apps/selora-desktop/src-tauri/src/main.rs` (command/event registration). |
| 2 | `SKARN_MASTER_BLUEPRINT.md:72-75` & `REBRAND_PLAN.md:27-30` claim `state.rs:6055,6058` hardcode the live GitHub updater endpoint | `state.rs:6046-6059` is `#[cfg(test)] fn open_external_allowlist_accepts_github_and_rejects_dangerous_urls()` testing `validate_external_url`. Live update checks execute in **two** places: (1) Rust `crates/selora-ai/src/updates.rs:23-41,111-169` (`CARGO_PKG_REPOSITORY`, `Selora-IDE-Setup-{version}.exe` `:149`, fallback `"selora-installer-v1.exe"` `:160`) + `apps/selora-desktop/src-tauri/src/updates.rs:65,84`; (2) Frontend `AboutDialog.tsx:23-82,131` (reads Vite `__REPOSITORY_URL__` from `frontend/package.json:8` via `vite.config.ts:10-17`) and `UpdatePrompt.tsx:22` (`App.tsx:176`). | Disabling updates cleanly in Stage 0 requires gating **both** Tauri commands (`updates.rs:65,84`) and the frontend `AboutDialog.tsx` / `UpdatePrompt.tsx` paths while keeping `CARGO_PKG_REPOSITORY` / `__REPOSITORY_URL__` derivation intact so `scripts/check-release-metadata.ps1:283-314,340-347` still passes. |
| 3 | `SKARN_MASTER_BLUEPRINT.md:149,537` & `REBRAND_PLAN.md:259` report **11** `localStorage` keys | There are **12** persisted `localStorage` keys. Missing #12 is `TAB_KEY = 'agents:last-context-tab'` in `AgentsContextPane.tsx:17,19,64` (`AgentsContextPane.test.tsx:83`). | Card `S1-2` migrates all **12** keys (including `agents:last-context-tab` -> `skarn:agents-last-context-tab`). |
| 4 | `REBRAND_PLAN.md:171` cites **13** `selora:*` DOM event names + **5** IPC channels | Direct `rg` over non-test frontend source shows **12** DOM `CustomEvent` names (`selora:about`, `selora:open-cheatsheet`, `selora:editor-command`, `selora:agents-context-tab`, `selora:attach-agent-file`, `selora:set-composer-draft`, `selora:search-query`, `selora:open-notifications`, `selora:open-connect`, `selora:set-view-mode`, `selora:preview-agent-file`, `selora:apply-code`) + **5** Rust<->TS IPC channels (`selora://event`, `selora://agent-preferences`, `selora://agents-navigate`, `selora://main-session-navigate`, `selora:update-progress`). | `S1-1` renames the 12 DOM `CustomEvent` names; `S1-2` atomically renames the 5 IPC channels. |
| 5 | `SKARN_MASTER_BLUEPRINT.md:130` & `REBRAND_PLAN.md:140` cite **358** files; `css-audit.mjs` line target cites `20,871` lines (`:247`) | On `626b90e`, `--strict` text walk finds **389 files / 13,668 lines** (`9,904` inside generated `graphify-out/`, `3,764` across source/config/docs); `css-audit.mjs` measures **28 CSS files / 20,899 lines**. | Stage 5 hygiene should ignore or clean `graphify-out/` alongside `.zcode/` and `nul` so `--strict` scans are meaningful. |
| 6 | `SKARN_MASTER_BLUEPRINT.md:157` & `REBRAND_PLAN.md:349` refer to `x-selora-*` / `selora/*` gateway headers/UA | Zero `x-selora-*` headers exist in Rust. Actual headers/UAs are: `client.rs:618-620` (`HTTP-Referer: https://selora.ide`, `X-Title: Selora IDE`, `User-Agent: Selora-IDE/0.1.0`), `crates/selora-ai/src/updates.rs:119,187` (`selora-ide/<ver>`), `crates/selora-ai/src/agent/tools/web.rs:549` (`SeloraIDE/<ver>`), and `crates/selora-lsp/src/client.rs:794` (`clientInfo.name = "selora-ide"`). | Update those 4 exact Rust sites (`S0-2` / `S1-1`). |
| 7 | Blueprint/Rebrand overlook additional gateway, fallback, sidecar, and registry sites | Found directly: `crates/selora-ai/examples/smoke.rs:5,16,19,41` (`api.selora.lol`, `SELORA_KEY`, `SELORA_MODEL`); `crates/selora-ui/src/dialogs/settings/selora_section.rs:82` (`sk-gw-`); `crates/selora-ai/src/agent/tools/fs.rs:870,884` (`temp_sibling` `.<name>.selora-tmp-*`, `backup_sibling` `.<name>.selora-bak`) + `router.rs:310`; `installer/selora-ide.iss:138-141` (`SeloraIDE.txt` HKCU ProgId); `apps/selora-desktop/src-tauri/src/updates.rs:149,151` (`SELORA_UPDATE_INSTALLER`). <!-- skarn-allow-legacy-fixture --> | Fold all of them into Cards `S0-2`, `S1-1`, and `S1-2`. |
| 8 | `SKARN_MASTER_BLUEPRINT.md:559` calls `img-fx` and `metal-fx` "crates" | Both are MIT-licensed **npm packages** in `apps/selora-desktop/frontend/package.json` (`img-fx`, `metal-fx`), not Rust crates. | Owner already cleared both in Phase 0 sign-off. |
| 9 | `SKARN_MASTER_BLUEPRINT.md:160,163` has internal tension in `ConnectModal.tsx` (re-tag OpenRouter to `'Aggregator'` vs "Four clean LLM cards") | `ConnectModal.tsx:52-139` & `crates/selora-settings/src/settings.rs:89-98` (`KNOWN_PROVIDER_IDS`) hold **8** presets (`selora`, `anthropic`, `openai`, `deepseek`, `ollama`, `openrouter`, `glm`, `custom`). Also `SettingsPanelView.tsx:322,338` and `AgentsCustomizationsView.tsx:131` render from `PROVIDER_PRESETS`. | Locked removal of `selora` and **Option A (7 BYOK presets)** with `anthropic`, `openai`, `ollama`, `custom` featured first and `openrouter` re-tagged `'Aggregator'`. |
| 10 | `SKARN_MASTER_BLUEPRINT.md:289,348-354` assumes 4 canonical permission modes | `AgentModeDto` in `crates/selora-protocol/src/lib.rs:237-244` has **3** variants (`Execute`, `Explore`, `Auto`), where `Auto` auto-applies file edits but still cards bash. Frontend has 3 modes (`full`, `plan_only`, `ask_before_change`) duplicated in `MODE_OPTIONS` (`AgentComposer.tsx:75-108`) and `COMPOSER_MODES` (`AgentComposer.tsx:121-145`). | Deferred by owner decision to **Stage 3** Decision Engine pass (`Read-only`, `Ask before changes`, `Auto-edit`, `Full access`). |
| 11 | Startup trust & TOCTOU gaps noted in codebase comments vs blueprint expectations | (1) `state.rs:2072-2076` latches `engine_workspace_trusted` once at startup (runtime settings toggles refuse new agent turns until restart via `:3346`). (2) `fs.rs:724-729` explicitly documents `apply_write` does **not** CAS-compare disk bytes against the approved diff card bytes (`apply_edit` re-checks `old_str` `:785-800`). (3) `bash.rs:48-80,594-644` strips child env via an allowlist and explicitly tests zero `SELORA_*` leakage; Stage 1/3 must keep blocking `SELORA_*`, `SKARN_*`, and future BYOK env keys (`JEV_*`, `GEMINI_*`). | Carry directly into **Stage 3** `WorkspaceSnapshot` + Policy Engine design. |

---

## C. Shared Choke-Point Map (Single-Writer Sequencing Rules)

All integration work must respect **one writer at a time** on shared files; any PR touching `> 2` shared integration files splits into sequential commits.

| Shared File | Verified Role & Size | Sequenced Owner by Stage |
|---|---|---|
| `Cargo.toml` | Workspace members (13 crates + desktop shell), `[workspace.package]` URLs (`:20-25`) & shared deps (`:26-79`) | **S0-1** (`repository`/`homepage` repoint) -> **S1-1** (workspace member paths + `[workspace.dependencies]` crate renames, regenerate `Cargo.lock`) -> **S1-2** (`version = "1.0.0"`) -> **S3** (`crates/skarn-decision` member added) |
| `crates/selora-protocol/src/lib.rs` | Canonical Rust<->TS IPC DTO contract (`4,463` lines; `AgentModeDto` `:237`, `SettingsViewDto`/`SettingsPatchDto` `:459+`) | **S1-1** (crate rename -> `crates/skarn-protocol/src/lib.rs`) -> **S1-2** (`selora-dark` test/default fixtures -> `skarn-dark`) -> **S2** (shrink `SurfaceId` / Settings DTOs) -> **S3** (4-mode `AgentModeDto`, `DecisionRequest`/`DecisionResult`, typed attachments) |
| `apps/selora-desktop/src-tauri/src/main.rs` *(Note: no `src-tauri/src/lib.rs` exists)* | Binary entry, plugins, close lifecycle, boot probe, `tauri::generate_handler!` (`265` lines) | **S1-1** (window title/log path comments untouched until S1-2; `use selora_*` -> `use skarn_*`) -> **S1-2** (`"Selora IDE"` `:183`, `%LOCALAPPDATA%/temp` log name `:76`, crash/thread names `:221`, copy prompt `:48`) -> **S4** (Decision Inspector/Provider commands) |
| `apps/selora-desktop/src-tauri/src/state.rs` | App state, bootstrap, settings dispatch, `apply_toggle` (`:4924`), `validate_external_url` (`:1080`), event log (`:1161-1271`), workspace trust latch (`:2076`) (`11,395` lines) | **S0-1** (fixture `:6049-6059`) -> **S0-2** (`known_disabled_providers` `:962`, `:5784` test) -> **S1-1** (module imports & `SeloraError`/`SeloraStatus`/`SeloraClient`) -> **S1-2** (`selora://event` `:1271`, `selora.disabled_providers` -> `skarn.disabled_providers`, `selora-dark` `:3226`) -> **S2** (`apply_toggle` region routing `:4924`) -> **S3** (dynamic trust + checkpoint/snapshot coordination) |
| `apps/selora-desktop/src-tauri/src/sessions.rs` | Agent loop bridge, tool approval/execution, `workspace_trusted` (`:4301`), `selora://event` relay (`9,348` lines) | **S1-1** (`use selora_*` / `Selora*` renames) -> **S1-2** (`SKARN_*` / `SELORA_API_KEY` `:155` & `SELORA_WORKSPACE_ROOT` `:502` alias order) -> **S3** (Decision Engine FSM, 4 modes, TOCTOU recheck, per-turn checkpoints) |
| `apps/selora-desktop/frontend/src/bridge/bridge.ts` | TS<->Tauri typed bridge (`1,609` lines): `SurfaceId` (`:35`), `Layout` (`:48`), `SettingsView` (`:72`), 5 event listeners (`:967,1029,1033,1587,1609`), `check_for_update`/`install_update` (`:1578,1585`) | **S0-1** (`check_for_update`/`install_update` error handling if surfaced) -> **S1-1** (non-channel symbol comments if any) -> **S1-2** (atomic rename of all **5** event channels + `UPDATE_PROGRESS_EVENT`) -> **S2** (`SurfaceId` subtraction) -> **S3** (Decision/4-mode DTO mirrors) |
| `apps/selora-desktop/frontend/src/features/agent/components/AgentComposer.tsx` | Composer input, `MODE_OPTIONS` (`:75`), `COMPOSER_MODES` (`:121`), `ATTACHMENT_HEADER` (`:160`), no-model copy (`:697,773`) (`1,173` lines) | **S0-2** (neutral no-model copy `:697,773` -> "Choose a provider to start") -> **S1-1** (internal CSS/event names `selora:*`) -> **S2** (delete `COMPOSER_MODES` duplicate + BorderBeam `-webkit`/`glow` debt + connect link to `Settings -> Providers`) -> **S3** (remove `ATTACHMENT_HEADER` prose, 4 canonical modes) -> **S5** (`@ # / $` grammar) |
| `apps/selora-desktop/frontend/src/styles/tokens.css` | Master design tokens (`259` lines): §1.1–1.7 ink/fg/status, §1.8 (`:83-105`), §1.9 `--glow-*`/`--glass-*` (`:106-123`), §1.10 `--grad-*` (`:125-130`), `--dur-lap` (`:203`) | **Freeze through Stage 0 & Stage 1** (zero edits) -> **Stage 2 Single-Owner Pass** (replace §1.8 Selora Space System with Skarn palette + remove §1.9–1.10 banned tokens + drop `2400ms` lap + unify radii) |

**Secondary synchronized choke-points:** `crates/selora-settings/src/{settings.rs,keyring.rs}`, `apps/selora-desktop/frontend/src/features/agent/agent-store.ts`, `apps/selora-desktop/frontend/src/features/keymap/commands.ts`.

---

## D. Tight Stage-Gate Task Cards (Stage 0 & Stage 1)

### Stage 0 — Card S0-1: Supply-Chain Isolation & Updater Kill-Switch

- **Goal:** Guarantee zero build ever queries or trusts `github.com/ToxicAngelX/Selora-IDE`, repoint manifests to `https://github.com/codeREDxbt/Skarn`, and keep runtime auto-updates **disabled** until the new repository exists and publishes `v1.0.0`. <!-- skarn-allow-legacy-fixture -->
- **Files Owned:**
  - `Cargo.toml:23-24` (`repository = "https://github.com/codeREDxbt/Skarn"`, `homepage = "https://github.com/codeREDxbt/Skarn"`)
  - `apps/selora-desktop/frontend/package.json:8` (`"url": "https://github.com/codeREDxbt/Skarn.git"`)
  - `installer/selora-ide.iss:22` (`#define RepoUrl "https://github.com/codeREDxbt/Skarn"`), `README.md:7`
  - `apps/selora-desktop/frontend/src/app/App.tsx:94` & `AgentsWindowApp.tsx:177` (derive from `__REPOSITORY_URL__` or confirmed docs URL instead of literal `ToxicAngelX/Selora-IDE`) <!-- skarn-allow-legacy-fixture -->
  - `crates/selora-ai/src/updates.rs:149,160,325,388-459` (replace fallback `"selora-installer-v1.exe"` -> `"Skarn-Setup-{version}.exe"` in `latest_release_info` test/matcher prep OR keep gated while adding `UPDATES_ENABLED` guard; update `:325` fixture or tag `// skarn-allow-legacy-fixture` + add wiremock regression test proving old-repo `v9.9.9` yields **no** update offer)
  - `apps/selora-desktop/src-tauri/src/updates.rs:58,65-85,149-151` (Owner Decision #2: disable `check_for_update` / `install_update` behind an explicit kill-switch constant/env e.g. `UPDATES_ENABLED: bool = false` until `codeREDxbt/Skarn` goes live, returning a clear non-fetching disabled status / `ProtocolErrorDto` cleanly without hitting GitHub; migrate staging dir to `%LOCALAPPDATA%\Skarn\updates` and `SKARN_UPDATE_INSTALLER`)
  - `apps/selora-desktop/frontend/src/components/AboutDialog.tsx:59,126` & `apps/selora-desktop/frontend/src/components/UpdatePrompt.tsx:22` (respect updater-disabled build state in UI so opening About/startup never emits background GitHub update checks while `codeREDxbt/Skarn` does not yet exist; preserve `__REPOSITORY_URL__` derivation so `check-release-metadata.ps1:308` passes)
  - `apps/selora-desktop/src-tauri/src/state.rs:6049-6059` & `crates/selora-app/src/shell.rs:4115` (replace hardcoded `ToxicAngelX/Selora-IDE` test literals with `codeREDxbt/Skarn`, leaving `validate_external_url` negative security tests intact) <!-- skarn-allow-legacy-fixture -->
- **Deletions in S0-1:**
  - Verify `apps/selora-desktop/frontend/vite.config.ts.timestamp-*` absent (already deleted by owner); Stage 0/5 hygiene candidates requiring owner confirmation before delete: root `nul` and `.zcode/`.
- **Check Commands:**
  - `node .agents/skills/skarn-supply-chain-gate/scripts/check-legacy-refs.mjs` -> check `[legacy-repo-slug]` count drops to **0**
  - `powershell -NoProfile -ExecutionPolicy Bypass -File scripts\check-release-metadata.ps1` -> exit `0`
  - `cargo test -p selora-ai updates::` (including new wiremock isolation test)
  - `npm --prefix apps/selora-desktop/frontend run typecheck && npm --prefix apps/selora-desktop/frontend test`
- **Rollback / Gate Criteria:** Revert single commit cleanly if any check fails; **do not** begin `S0-2` until `check-legacy-refs.mjs` reports 0 `[legacy-repo-slug]` hits and `check-release-metadata.ps1` passes.

---

### Stage 0 — Card S0-2: Gateway Kill (`api.selora.lol` Removal & Honest BYOK First-Run) <!-- skarn-allow-legacy-fixture -->

- **Goal:** Eliminate all `api.selora.lol` endpoints, gateway presets, key hints, headers, and Gateway copy cleanly under `0.1.0` before renaming crates. <!-- skarn-allow-legacy-fixture -->
- **Files Owned:**
  - `apps/selora-desktop/frontend/src/features/auth/ConnectModal.tsx:45-140,190-197,256-257` & UI copy: remove `id: 'selora'` preset (`api.selora.lol`, `sk-selora-`, `sk-gw-`, `'Recommended'` badge), remove `useState('selora')` and `useState('https://api.selora.lol')` defaults (neutral non-preselected initial state, empty custom base URL), keep 7 BYOK presets with `anthropic`, `openai`, `ollama`, `custom` featured first and `openrouter` re-tagged `'Gateway'` -> `'Aggregator'`, add local-first BYOK copy (*"Bring your own key. Keys stay in Windows Credential Manager."*), replace `"Gateway"` strings. *(Option A locked.)* <!-- skarn-allow-legacy-fixture -->
  - `apps/selora-desktop/frontend/src/features/auth/provider-visibility.ts:1-7,37`: delete `if (haystack.includes('selora')) return 'selora';` and gateway comment.
  - `apps/selora-desktop/frontend/src/features/auth/{ConnectModal.discovery.test.tsx,ConnectModal.providers.test.tsx,provider-visibility.test.ts}` + `apps/selora-desktop/frontend/src/features/agents-workspace/AgentsCustomizationsView.test.tsx` + `SettingsPanelView.tsx`: rewrite `'selora'`/`'selora-fast'` assertions around neutral providers (`anthropic`/`openai`/`ollama`).
  - `apps/selora-desktop/frontend/src/features/agent/agent-store.ts:266` & `AgentComposer.tsx:697,773` & `StatusBar.tsx:40-67,102`: update `NO_MODEL_MESSAGE` / composer empty state -> `"Choose a provider to start"` (linking to Connect / Providers) and replace `selora connected` / `Gateway connected` strings.
  - `crates/selora-settings/src/settings.rs:66-68,89-98`: change `default_base_url()` to `String::new()`, remove `"selora"` from `KNOWN_PROVIDER_IDS` (note `state.rs:962-966` already filters unknown ids on boot), update unit tests at `:793,985,1002,1025,1037,1190,1327` (using `// skarn-allow-legacy-fixture` **only** if a test explicitly verifies ignoring/migrating legacy URLs).
  - `crates/selora-app/src/wiring.rs:47,933,946,1314-1636` & `shell.rs:277` & `crates/selora-ui/src/dialogs/settings/selora_section.rs:82`: remove `DEFAULT_BASE_URL = "https://api.selora.lol"` and `sk-gw-` hints/test literals (`crates/selora-app` & `crates/selora-ui`). <!-- skarn-allow-legacy-fixture -->
  - `crates/selora-ai/src/lib.rs:1`, `crates/selora-ai/src/client.rs:550,618-620`, `crates/selora-ai/examples/smoke.rs:5,16,19,40-41`, `crates/selora-ai/src/updates.rs:119,187`, `crates/selora-ai/src/agent/tools/web.rs:549`, `apps/selora-desktop/src-tauri/src/workflows_host.rs:193`, `apps/selora-desktop/src-tauri/src/state.rs:5784`: remove `api.selora.lol` doc/code defaults, update client `HTTP-Referer` (`https://github.com/codeREDxbt/Skarn`), `X-Title: Skarn`, and `User-Agent` headers. <!-- skarn-allow-legacy-fixture -->
- **Check Commands:**
  - `node .agents/skills/skarn-supply-chain-gate/scripts/check-legacy-refs.mjs` -> **exit `0` (`0` P0 hits total)**
  - `cargo test --workspace`
  - `npm --prefix apps/selora-desktop/frontend run typecheck && npm --prefix apps/selora-desktop/frontend test`
- **Rollback / Gate Criteria:** Whole Stage 0 must exit `0` on `check-legacy-refs.mjs` before **Stage 1** opens.

---

### Stage 1 — Card S1-1: Mechanical Rename (Commit 1 — Zero Logic Diff)

- **Goal:** Rename workspace packages, modules, public Rust types, binary/app directories, CI workflows, scripts, internal CSS classes, and DOM `CustomEvent` names in **one pure rename commit** with zero changes to persisted user data contracts or IPC channels.
- **Files Owned:**
  - `git mv crates/selora-* crates/skarn-*` (12 crates: `core`, `fs`, `editor`, `lsp`, `git`, `ai`, `settings`, `tokens`, `ui`, `app`, `protocol`, `workflow`) + `git mv apps/selora-desktop apps/skarn-desktop` + `git mv launch-selora.bat launch-skarn.bat`.
  - Root `Cargo.toml` (`members`, `[workspace.dependencies]`), all per-crate `Cargo.toml`, `apps/skarn-desktop/src-tauri/Cargo.toml` (`name = "skarn-desktop"`), regenerate `Cargo.lock` (`cargo check --workspace` / `cargo generate-lockfile`).
  - All `use selora_*::` / `selora_*::` paths + public types (`SeloraError` -> `SkarnError`, `SeloraStatus` -> `SkarnStatus`, `SeloraClient` -> `SkarnClient`, `SeloraSettings` JSON key still aliased in `S1-2`, `selora_section.rs` -> `skarn_section.rs`).
  - Scripts & CI: `.github/workflows/{ci,release}.yml`, `start.bat`, `launch-skarn.bat`, `scripts/*`, `SIGNING.md`, `deny.toml`, `crates/skarn-lsp/src/client.rs:794,959` (`clientInfo.name`), `crates/skarn-ai/src/updates.rs:119,187`, `web.rs:549`.
  - Frontend non-contract symbols: `apps/skarn-desktop/frontend/package.json` (`name`), `SeloraMark.tsx` -> `SkarnMark.tsx`, internal CSS classes (`.selora-*`), and all **12 DOM `CustomEvent('selora:...')`** (`selora:about`, `selora:open-cheatsheet`, `selora:editor-command`, `selora:agents-context-tab`, `selora:attach-agent-file`, `selora:set-composer-draft`, `selora:search-query`, `selora:open-notifications`, `selora:open-connect`, `selora:set-view-mode`, `selora:preview-agent-file`, `selora:apply-code`).
- **Explicitly Deferred to S1-2 (DO NOT rename here):**
  - `tauri.conf.json` `identifier` / `productName` / `version`, `installer/selora-ide.iss`, `CONFIG_DIR` (`"selora-ide"`), `settings.json` root field (`pub selora`), keyring `SERVICE` (`"selora-ide"`), `SELORA_*` env vars, persisted theme id `"selora-dark"`, the 12 `localStorage` keys, and the 5 IPC channels (`selora://*`, `selora:update-progress`).
- **Check Commands:**
  - `cargo check --workspace && cargo test --workspace --locked`
  - `npm --prefix apps/skarn-desktop/frontend run typecheck && npm --prefix apps/skarn-desktop/frontend test && npm --prefix apps/skarn-desktop/frontend run build`
  - `node .agents/skills/skarn-migration-guard/scripts/check-ipc-parity.mjs` -> exit `0`
- **Rollback / Gate Criteria:** `git diff --stat` / spot diff confirms zero logic changes; compiler + full test suite green before Commit 2 (`S1-2`).

---

### Stage 1 — Card S1-2: Breaking Migrations + Version `1.0.0` (Commit 2)

- **Goal:** Atomically cut over all breaking persisted/protocol contracts with lossless v0.1.0 -> v1.0.0 migration paths + update agent sidecar extension + bundle Skarn legal/attribution files.
- **Files Owned (all 8 contracts + sidecar + legal):**
  1. **Tauri Identity (`apps/skarn-desktop/src-tauri/tauri.conf.json` & `main.rs`):** `productName: "Skarn"`, `version: "1.0.0"` (synced in `Cargo.toml:20` & `frontend/package.json`), `identifier: "app.skarn.desktop"`, window titles `"Skarn"`. On boot before WebView2 init, if new Tauri WebView2 data dir is absent and legacy `lol.selora.ide` data dir exists, copy tree cleanly.
  2. **Installer (`git mv installer/selora-ide.iss installer/skarn.iss`, `installer/prebuild.bat`, `scripts/check-release-metadata.ps1`, `.github/workflows/{ci,release}.yml`):**
     - **KEEP** `AppId={{7A3E9C41-5D2B-4E8F-B6A1-9C0D2E4F6A8B}}` (`:47`) and `DefaultDirName={localappdata}\Programs\Selora IDE` (`:54`) for v1.0 in-place upgrade (moving install dir moves to v1.1 with `[InstallDelete]`).
     - Update `AppName=Skarn`, `AppVerName=Skarn {#MyAppVersion}`, `AppPublisher=codeREDxbt`, `OutputBaseFilename=Skarn-Setup-{#MyAppVersion}`, `UninstallDisplayName=Skarn`, shortcut names, `MyAppExe=skarn-desktop.exe`, HKCU ProgId `Skarn.txt` (`[InstallDelete]` / clean old `SeloraIDE.txt` ProgId entry), `[UninstallDelete]` covering `{userappdata}\skarn`, staging path `%LOCALAPPDATA%\Skarn\updates`.
     - Re-target `installer/prebuild.bat` binary (`skarn-desktop.exe`), SHA-256 regex (`^([0-9a-f]{64})  Skarn-Setup-\d+\.\d+\.\d+\.exe$`), and `scripts/check-release-metadata.ps1` (`installer/skarn.iss`, `crates/skarn-ai/src/updates.rs`, etc.).
  3. **Settings Dir & JSON Schema (`crates/skarn-settings/src/settings.rs` & `app/persistence.rs`):**
     - `CONFIG_DIR`: `"selora-ide"` -> `"skarn"`; migrate `%APPDATA%\selora-ide` tree -> `%APPDATA%\skarn` once if new dir absent, writing `migrated_from: "selora-ide"` + timestamp marker. Keep legacy dir read-only fallback for v1.0/v1.1.
     - Rename root JSON field `pub selora` (`:135`) -> `pub skarn` with `#[serde(alias = "selora")]` (`// skarn-v1-migration-compat`); always serialize as `skarn`.
     - Default theme `"selora-dark"` -> `"skarn-dark"` across `settings.rs`, `selora-protocol/src/lib.rs`, `state.rs`, `frontend/src/features/theme/theme.ts`; accept `"selora-dark"` as read alias mapped to dark palette so persisted themes never error (`state.rs:3226`).
  4. **OS Keyring (`crates/skarn-settings/src/keyring.rs`):**
     - `SERVICE = "skarn-ide"`. On read/boot when `skarn-ide` lacks `api-key` or `brave-key`: read old `"selora-ide"` -> write `"skarn-ide"` -> verify read-back -> delete `"selora-ide"` -> delete plaintext `.api_key` / `.brave_key` fallback only after keyring write succeeds; log one secret-free status line; show clear re-entry prompt on failure.
  5. **Environment Variables (`crates/skarn-settings`, `src-tauri/src/{commands.rs,sessions.rs,state.rs,updates.rs}`, `crates/skarn-ai/src/agent/tools/bash.rs`, `workflows_host.rs`, `installer/prebuild.bat`, `find-iscc.ps1`):**
     - Primary names: `SKARN_API_KEY`, `SKARN_SETTINGS_DIR`, `SKARN_WORKSPACE_ROOT`, `SKARN_ISCC`, `SKARN_UPDATE_INSTALLER`, `SKARN_SETUP*`; support `SELORA_*` fallback aliases with one deprecation warning (`// skarn-v1-migration-compat`) for v1.0. In `bash.rs` (`build_child_env` / tests) and `workflows_host.rs:173` (`SECRET_ENV_VARS`), block **both** `SELORA_*` and `SKARN_*` from child processes.
  6. **All 12 `localStorage` Keys (Frontend Bootstrap Migration Before Any Store Reads):**
     Table-driven one-shot bootstrap migration copying old -> new only when new key is absent (`prefix` replacement must preserve dynamic suffixes verbatim; **never** reset `onboarding_seen`):
     1. `selora:model_settings` -> `skarn:model_settings` (`agent-store.ts:133`)
     2. `selora:agent-pinned-sessions` -> `skarn:agent-pinned-sessions` (`agent-store.ts:134`)
     3. `selora:agent-draft:<json>` -> `skarn:agent-draft:<json>` (`draft-store.ts:15`)
     4. `selora:editor-session:<path>` -> `skarn:editor-session:<path>` (`editor-persistence.ts:18`)
     5. `selora:last-editor-workspace` -> `skarn:last-editor-workspace` (`editor-persistence.ts:14`)
     6. `selora:editor-preferences` -> `skarn:editor-preferences` (`local-preferences.ts:2`)
     7. `selora:recent-commands` -> `skarn:recent-commands` (`CommandPalette.tsx:107,172`)
     8. `selora:agents-workspace:v1:<id>` -> `skarn:agents-workspace:v1:<id>` (`agents-workspace-store.ts:41`)
     9. `selora:ui-sounds` -> `skarn:ui-sounds` (`sound-store.ts:16`)
     10. `selora:notification_history` -> `skarn:notification_history` (`toast-store.ts:22`)
     11. `selora:onboarding_seen` -> `skarn:onboarding_seen` (`onboarding-store.ts:8`)
     12. **`agents:last-context-tab` -> `skarn:agents-last-context-tab` (`AgentsContextPane.tsx:17`, Owner Addition)**
  7. **5 IPC Channels (Atomic Rename Within Commit 2):**
     - `selora://event` -> `skarn://event` (`state.rs:1271`, `bridge.ts:1609`, `sessions.rs` doc)
     - `selora://agent-preferences` -> `skarn://agent-preferences` (`agents_preferences.rs:161,304`, `bridge.ts:967`)
     - `selora://agents-navigate` -> `skarn://agents-navigate` (`agents_window.rs:170`, `bridge.ts:1029`, `agents-bridge.test.ts:72`)
     - `selora://main-session-navigate` -> `skarn://main-session-navigate` (`agents_window.rs:233`, `bridge.ts:1033`)
     - `selora:update-progress` -> `skarn:update-progress` (`updates.rs:19`, `bridge.ts:1587`)
  8. **Agent File Sidecars (Owner Addition):**
     - `crates/skarn-ai/src/agent/tools/fs.rs:884` `backup_sibling`: `.<name>.selora-bak` -> `.<name>.skarn-bak`
     - `crates/skarn-ai/src/agent/tools/fs.rs:870` `temp_sibling`: `.<name>.selora-tmp-{pid}-{seq}` -> `.<name>.skarn-tmp-{pid}-{seq}`
     - Update companion docs/tests (`fs.rs:856,873,1629,1640,1695`, `router.rs:310`) so `--strict` scans stay clean.
  9. **Legal, Fonts & Attribution:** Keep original MIT copyright line in `LICENSE` + add `codeREDxbt` copyright line; add one attribution line in `README.md` (*"This project began as Selora IDE..."*); add `CHANGELOG.md` `## [1.0.0]` entry noting rename from Selora IDE; verify bundled OFL fonts (Space Grotesk, JetBrains Mono). *(Note: prior to Stage 4/5 `--strict` checks, archive this brief under `docs/research/` so historical citations remain allowlisted.)*
- **Check Commands:**
  - `node .agents/skills/skarn-migration-guard/scripts/check-ipc-parity.mjs` -> exit `0`
  - `node .agents/skills/skarn-supply-chain-gate/scripts/check-legacy-refs.mjs` -> exit `0`
  - `powershell -NoProfile -ExecutionPolicy Bypass -File scripts\check-release-metadata.ps1` -> exit `0`
  - `cargo test --workspace --locked` (including migration unit tests: `serde(alias="selora")`, settings dir copy + marker, keyring `selora-ide` -> `skarn-ide`, `SELORA_*` fallback + `SKARN_*` child env stripping, `.skarn-bak` sidecar tests)
  - `npm --prefix apps/skarn-desktop/frontend run typecheck && npm --prefix apps/skarn-desktop/frontend test && npm --prefix apps/skarn-desktop/frontend run build` (including 12-key bootstrap `localStorage` migration test suite)
- **Rollback / Gate Criteria:** Clean Windows 11 VM upgrade (`0.1.0` -> `1.0.0`) must pass: settings preserved, API key migrated with 0 `.api_key` survivor, all 12 `localStorage` keys intact (`onboarding_seen` not reset), single Uninstall entry, all 5 live IPC channels flowing.

---

## E. Owner Decisions Summary & Unverified External Provider Contracts

### 1. Decisions Resolution Table

| # | Item | Status | Resolved Value / Next Action |
|---|---|---|---|
| D1 | Workspace & git clean baseline (`626b90e`), `.agents/` setup, Vite temp deletion | **LOCKED** | Complete on disk. |
| D2 | Control of `ToxicAngelX/Selora-IDE` <!-- skarn-allow-legacy-fixture --> | **LOCKED** | Owner does NOT control old repo & new repo not live yet -> **Disable `check_for_update` / `install_update` in Stage 0** until `codeREDxbt/Skarn` v1.0.0 is live. |
| D3 | Product Name (`Skarn` vs `Skarnify`), Repo URL, Publisher, Cert, `img-fx`/`metal-fx` | **LOCKED** | Name = `Skarn`; Repo = `https://github.com/codeREDxbt/Skarn` (`.git` in `package.json`); `AppPublisher = codeREDxbt`; Signing = CI release fail on unsigned, local dev allowed unsigned; `img-fx`/`metal-fx` MIT npm cleared. |
| D4 | Docs Host URL (`F1` / About / `OpenExternal`) | **LOCKED** | `https://github.com/codeREDxbt/Skarn/tree/main/docs` (`App.tsx`, `AgentsWindowApp.tsx`, docs links; host already allowlisted in `state.rs`). |
| D5 | `ConnectModal.tsx` Preset Scope (`7` vs `4`) | **LOCKED** | **Option A (7 BYOK presets):** delete `selora`, `api.selora.lol`, `sk-selora-`/`sk-gw-`, `'Recommended'`; no preselection on first run; feature `anthropic`, `openai`, `ollama`, `custom` first; re-tag `openrouter` `'Aggregator'`. <!-- skarn-allow-legacy-fixture --> |
| D6 | 4 Canonical Permission Modes (`Stage 3`) | **LOCKED** | Approved: evolve `AgentModeDto` to `Read-only`, `Ask before changes`, `Auto-edit`, `Full access` in Stage 3 Decision Engine. |
| D7 | Stage 1 Additions (`fs.rs:884` & `agents:last-context-tab`) + Hygiene | **LOCKED** | Both additions included in `S1-2`; delete stray root `nul` only in Stage 0; leave `.zcode/`, `graphify-out/`, and research `.md` files untouched until final release. |

### 2. Unverified External Provider Contracts (Do Not Fabricate — Stage 3/4 Gates)

Per owner instruction and `SKARN_MASTER_BLUEPRINT.md` §1.2 / §4.7, do **not** invent wire fields or model packaging specs before live verification:

1. **TypeSafe Jev (`https://typesafe.ai/`):**
   - *Unverified / must confirm from official docs or live key before Stage 4 wiring:* exact REST request/response JSON envelope for `/v1/systemone`, per-decision-type (`noul`/`choice`/`score`/`box`) output property keys, exact `criteria` / `box` sub-schema, `400`/`422` validation error schema, and org RPM/TPM limits.
   - *Contract until verified:* `DecisionProvider` Rust trait + `JevAdapter` behind config/feature flag + mock/record-replay wire fixtures.
2. **Google Gemini Decisions API (`https://aistudio.google.com/docs/decisions-api`):**
   - *Unverified / must confirm before Stage 4 wiring:* exact REST payload for multimodal `input` blocks (vs Python SDK types), `score` `criteria` structure (array vs object map) and `noul` `criteria` (`true_description` vs `false_description` naming), Google RPC error response structure, and tier-specific RPM/TPM quotas.
   - *Contract until verified:* `GeminiDecisionsAdapter` behind trait boundary + manual JSON fixture tests marked `#[ignore = "needs-live-key"]` until live sandbox verification runs.
3. **Laya (Local Default Provider):**
   - *Unfinalized by design:* model weight format, quantization scheme, binary packaging, CLI/IPC transport protocol, and artifact public signing key distribution remain TBD.
   - *Contract until finalized:* Stage 4 builds `LayaRuntimeAdapter` + `LayaLifecycleManager` against an isolated deterministic/adversarial dev/mock runtime so all lifecycle states, SHA-256 + signature verification hooks, memory caps, and Decision Inspector flows are end-to-end proven before real weights ship.
