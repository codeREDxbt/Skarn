# Skarn — Master Architecture, Rebrand, UX & Decision Engine Blueprint

> **Single Source of Truth** unifying four previously separate workstreams into one coherent
> engineering program for **Skarn** (formerly Selora IDE):
>
> 1. **Supply-Chain Hardening & Rebrand (`Selora` -> `Skarn`)** — closing the untrusted
>    auto-updater path, migrating 8 breaking data/protocol contracts without user data loss,
>    and removing the first-party Selora API Gateway.
> 2. **Shell Subtraction, Routing & Craft Constitution Repair** — eliminating fake UI state,
>    enforcing strict region ownership, collapsing multi-home configuration, and resolving the
>    `tokens.css` contradiction once.
> 3. **ZCode Trust Convergence** — one canonical permission vocabulary, scoped approvals,
>    labelled/clocked waits, per-turn undo checkpoints, `AGENTS.md` project rules, and typed
>    context without injected protocol prose.
> 4. **The Decision Engine Layer** — fast, grounded, verifiable three-tier execution
>    (Deterministic IDE Tools -> Closed-Ended Decision Models [Laya local default, BYOK
>    TypeSafe Jev, Google Gemini Decisions API] -> General LLMs) with TOCTOU snapshot protection,
>    asymmetric risk gating, privacy-by-construction, and shadow-mode evaluation.
>
> **Companion detail artifacts** (kept in `docs/research/` as dated evidence, never renamed):
> `REBRAND_PLAN.md`, `SELORA_MASTER_DOSSIER.md`, `SELORA_UX_PAINPOINTS.md`,
> `SELORA_CRAFT_SPEC.md`, `ZCODE_UX_TEARDOWN.md`, `ZCODE_PARITY_PROMPTS.md`.

---

## 0. Executive Architectural Verdict (Why These Four Tracks Are One System)

Treating the rebrand, shell cleanup, ZCode trust fixes, and Decision Engine as parallel projects
will fracture the repository. Every track mutates the exact same shared integration spine:
`crates/selora-protocol/src/lib.rs`, `apps/selora-desktop/src-tauri/src/{state,sessions}.rs`,
`crates/selora-settings/src/{settings,keyring}.rs`, `frontend/src/bridge/bridge.ts`,
`features/agent/*`, `features/auth/*`, and `styles/tokens.css`.

A Principal Systems Engineer unifies them around **five load-bearing architectural couplings**:

1. **One Execution & Policy Kernel (ZCode Trust + Decision Engine):**
   Do not build a ZCode permission gate *and* a separate Decision Engine policy engine. Four
   canonical permission modes, scoped allow/deny rules, calibrated risk thresholds, wait
   classification (blocking Approval vs. clocked Question), and capability-bound tool validation
   are **one finite state machine**.
2. **One Workspace Snapshot & Checkpoint Layer (TOCTOU Defense + Per-Turn Undo):**
   The Decision Engine needs immutable workspace snapshots (document revisions, file content
   hashes, workspace root, git HEAD) so actions never execute against drifted editor state.
   ZCode Wave 1 needs content-addressed before-snapshots for `Undo turn / Reapply`. Build one
   workspace-state/snapshot subsystem that serves both.
3. **One Configuration & Credential Spine (Gateway Removal + UX Consolidation + BYOK Providers):**
   Removing the Selora API Gateway, collapsing three provider-config homes into one, promoting
   Settings out of the 300px sidebar, and adding Decision Providers (Laya, Jev, Gemini) all
   resolve into a single `Settings -> Providers` page backed by one OS keyring service
   (`skarn-ide`) and one workspace privacy boundary.
4. **One Artifact Trust Model (P0 Updater Fix + Laya Local Installer):**
   The current custom updater (`crates/selora-ai/src/updates.rs`) checks a `.sha256` fetched
   from the same GitHub release—proving integrity, not authenticity—against hardcoded old-repo
   URLs. Both app updates and future Laya local runtime/model downloads must share a pinned,
   cryptographically verified artifact contract.
5. **One Design Constitution Pass (`tokens.css` Single-Owner Rule):**
   `tokens.css` §1.8 carries the old brand palette ("Selora Monochrome Space System") alongside
   banned `--glass-*`, `--glow-*`, and `--grad-*` tokens. Replacing the palette for Skarn and
   repairing the craft constitution happen in **one owned pass**, eliminating merge conflicts
   and half the lighting-model debt simultaneously.

---

## 1. Stage 0 (P0 Emergency): Supply-Chain Security & Baseline Ground Truth

**Ship before any rename, UX pass, or Decision Engine code.**

### 1.1 The Untrusted Auto-Updater Defect (Verified)

* `tauri.conf.json` sets `bundle.active: false` and has **no** `plugins.updater` block (no
  Tauri pubkey, no signature verification).
* `crates/selora-ai/src/updates.rs` is a custom client reading `CARGO_PKG_REPOSITORY`, but
  `apps/selora-desktop/src-tauri/src/state.rs:6055` and `:6058` **hardcode**
  `https://api.github.com/repos/ToxicAngelX/Selora-IDE/releases/latest`.
* Asset matching expects `Selora-IDE-Setup-<tag>.exe` + `.sha256` (`updates.rs:149`, fixtures
  `:397-453`) from the same release. Whoever controls the release controls both files.
* Docs links are separately hardcoded at `App.tsx:94`, `AgentsWindowApp.tsx:177`,
  `frontend/package.json`, and installer `RepoUrl`.

**Mandatory P0 actions:**
1. Create the new repository `<owner>/skarn` (private initially) with branch protection
   (`cargo test`, `typecheck`, `npm test`, `build`).
2. Make `[workspace.package] repository` in root `Cargo.toml` the **single source of truth**.
   Delete every hardcoded repo URL copy and add a CI grep check failing on any second slug.
3. Repoint or disable the updater until the new repo exists; update the asset matcher to
   `Skarn-Setup-<version>.exe` + `.sha256` in lockstep with the installer; plan cryptographic
   signature verification (not just same-release SHA-256) as the shared trust gate.
4. Release the rebranded build as **v1.0.0** so semver comparison can never prefer an old `0.x`.
5. Remove `https://api.selora.lol` defaults immediately.
6. Begin OV/EV code-signing certificate issuance immediately under new secret names
   (`SKARN_SIGNING_CERT_BASE64`, `SKARN_SIGNING_CERT_PASSWORD`, `TAURI_SIGNING_PRIVATE_KEY`).
   Never reuse `selora-signing.pfx` or sign Skarn with the old publisher cert.

### 1.2 Raw Ground-Truth Baseline (Before Architectural Coding)

Before writing Decision Engine or UX Wave 1 code, collect objective baselines:
1. **Live runtime seam audit:** Trace the exact agent dispatch loop, tool/action registry,
   CodeMirror/LSP buffer revision mechanism, workspace trust hooks, cancellation token plumbing,
   keyring wrapper, and Tauri IPC boundaries.
2. **Real workload & UX session capture:** Record one 20-minute real session (mouse path +
   six metrics: approvals/session, turns waiting > 2 min, mode switches/session, undo-vs-git
   ratio, re-explained project facts, time-to-first-accepted-change) and mine 50-200 real
   closed-ended decision points from agent runs/tests for the offline shadow-mode golden set.
3. **External wire verification:** Inspect live official documentation for **TypeSafe Jev**
   (`https://typesafe.ai/`) and **Google Gemini Decisions API**
   (`https://aistudio.google.com/docs/decisions-api`). Record exact HTTP schemas, auth headers,
   decision forms, rate limits, and error payloads. List every missing or unverified detail
   explicitly—never fabricate API fields or a Laya model format.

---

## 2. Brand Identity, Rename Census & Data-Preserving Migration

### 2.1 Identity & Clearance Protocol

* **Chosen product name:** **`Skarn`** (recorded 2026-11-25; no `-ify` suffix, no `IDE` in the
  product name; descriptor: `Skarn -- code workbench`). Backups: `Pelorus`, `Alidade`.
* **Live-checked rejected collisions:** Cairn, Burin, Umber, Mullion, Kerf, Chamfer (plus
  known collisions: Rook, Loom, Shuttle, Kestrel, Cinder, Graphite, Onyx, Codex, Spline,
  Vellum, Chisel, Tracer, Atlas, Nova).
* **Owner gate before assets/publishing:** Complete the 30-minute clearance run (USPTO/EUIPO
  classes 9 & 42, `"Skarn" IDE / code editor / coding agent`, GitHub org, npm/crates/PyPI,
  `.dev`/`.app`/`.com`, Microsoft Store/WinGet, social handles) and commit
  `docs/brand/NAME_DECISION.md`.
* **Canonical identity tokens:**
  - Display: `Skarn` | Slug: `skarn` | Tauri ID: `app.skarn.desktop`
  - Installer: `Skarn-Setup-<version>.exe` | Version: `1.0.0`
  - Settings dir: `%APPDATA%\skarn` | Keyring service: `skarn-ide` | Env prefix: `SKARN_`

### 2.2 Measured Rename Surface (358 Files)

Case-sensitive census: `Selora` (659), `selora` (593), `selora_ai::` (446), `selora-desktop`
(419), `SeloraError` (309), `selora_protocol::` (214), `selora-ui` (212), `selora_settings::`
(205), `SeloraStatus` (133), `Selora-IDE-Setup-` (33), `selora-dark` (30), `selora:` (29).

### 2.3 The Eight Breaking Data & Protocol Contracts

Mechanical renaming is safe; these eight contracts silently destroy user data or break IPC if
handled naively:

| # | Contract | Old Value / Location | Safe Migration Design | Drop Compatibility |
|---|---|---|---|---|
| 1 | **Tauri identifier** | `lol.selora.ide` (`tauri.conf.json`) | Switch to `app.skarn.desktop`. On first launch, if new WebView2 dir is absent and old exists, copy it. Never lose `%APPDATA%` settings. | v1.1 |
| 2 | **Installer `AppId` & dir** | `{7A3E9C41-5D2B-4E8F-B6A1-9C0D2E4F6A8B}`, `DefaultDirName` (`installer/selora-ide.iss`) | **Keep GUID and install folder for v1.0** so upgrade is in-place with one uninstall entry. Change display strings & output filename only. Move folder in v1.1 with explicit `[InstallDelete]`. | Move dir in v1.1 |
| 3 | **Settings directory** | `%APPDATA%\selora-ide` | Copy tree to `%APPDATA%\skarn` if absent, write `migrated_from: "selora-ide"` + timestamp. Keep old dir read-only fallback for one release. Delete old plaintext `.api_key` only after keyring migration succeeds. | Read fallback v1.1; delete v1.2 |
| 4 | **Settings JSON root key (silent reset trap)** | `pub selora: SeloraSettings` (`crates/selora-settings/src/settings.rs:135`) | Rename field to `skarn`, add `#[serde(alias = "selora")]` on read, always write `skarn`. Without this alias, every migrated profile silently resets. | v1.1 |
| 5 | **OS Credential Store** | `keyring::SERVICE = "selora-ide"` (`api-key`, `brave-key`) | On launch, if `skarn-ide` lacks key: read old -> write new -> verify -> delete old; log one secret-free line. On read failure, show one clear re-entry prompt (never appear silently disconnected). | v1.1 |
| 6 | **Environment variables** | `SELORA_API_KEY`, `SELORA_WORKSPACE_ROOT`, `SELORA_SETTINGS_DIR`, `SELORA_ISCC`, `SELORA_SETUP*` | Introduce `SKARN_*`; read `SELORA_*` fallback with deprecation warning for one release. | v1.1 |
| 7 | **11 `localStorage` keys (silent reset trap)** | `selora:model_settings`, `agent-pinned-sessions`, `agent-draft:<key>`, `editor-session:<path>`, `last-editor-workspace`, `editor-preferences`, `recent-commands`, `agents-workspace:v1:<id>`, `ui-sounds`, `notification_history`, `onboarding_seen` | Single table-driven bootstrap migration **before any store reads**. Copy absent new keys from old keys; preserve parameterized path/JSON suffixes verbatim; never reset `onboarding_seen`. | v1.2 |
| 8 | **5 Tauri Event Channels (silent IPC death)** | `selora://event`, `selora://agent-preferences`, `selora://agents-navigate`, `selora://main-session-navigate`, `selora:update-progress` (8 Rust/JS sites + `bridge/agents-bridge.test.ts:72`) | Rename all emit/listen sites in **one atomic commit** (or dual-emit for one commit if phased). Verify one live flow per channel. Also alias persisted theme id `selora-dark`. | Atomic |

### 2.4 First-Party Gateway Removal & BYOK First-Run Redesign

* **Remove completely:** `ConnectModal.tsx` preset `id: 'selora'`, `https://api.selora.lol`,
  `sk-selora-` / `sk-gw-` hints and branches, `useState('selora')`, `provider-visibility.ts:37`
  heuristic, `'selora'`/`'selora-fast'` test fixtures, Rust gateway base-URL defaults and
  `x-selora-*` / `selora/*` headers/UA, and all "Gateway" copy (`Model Provider & Gateway
  Connection`, `Gateway Base URL`, `Settings > AI Gateway`, `selora connected`, `Appended to
  Selora's system prompt`).
* **Vocabulary fix:** Re-tag OpenRouter from `'Gateway'` to `'Aggregator'`.
* **First-run replacement (BYOK-honest):**
  - No preselected cloud provider and no `'Recommended'` badge on paid services.
  - Four clean LLM cards: **Anthropic · OpenAI · Local (Ollama) · Custom endpoint** (Ollama
    honestly noted as requiring no key, not preselected).
  - Clear local-first trust statement: *"Bring your own key. Keys stay in Windows Credential
    Manager."*
  - Composer `no_model` empty state reads *"Choose a provider to start"* linking directly to
    `Settings -> Providers`.
  - Model catalogs come strictly from provider `/models` responses plus a custom model ID
    override.

### 2.5 Legal, Attribution & Repo Hygiene

* Retain original MIT copyright in `LICENSE`; add Skarn copyright line; add one attribution
  line in `README.md`; add `CHANGELOG.md` entry *"1.0.0 -- renamed from Selora IDE"*. Never
  rewrite git history.
* Bundle OFL licences for Space Grotesk and JetBrains Mono; audit provenance and licences of
  `img-fx` and `metal-fx` before v1.0.0.
* Delete `.zcode/` developer scratch and stray `nul` file from repo root; move dated research
  docs to `docs/research/` unmodified.

---

## 3. Shell Architecture, Subtraction & Craft Constitution Repair

### 3.1 The Three Root UX Faults (Source-Verified)

1. **Jumbled (Routing is a `switch`):** `SurfaceId` is a flat 9-value union (`bridge.ts:44`)
   routed by `apply_toggle` (`state.rs:4927`). Left-rail `problems` opens the bottom dock and
   can never light up (`ActivityBar.tsx:227`); `PrimarySidebar.tsx:41-48,223` carries dead
   labels and an unreachable `agent` route; Settings is trapped in a ~300px sidebar column
   (`PrimarySidebar.tsx:222`); `Ctrl+B` targets different chrome in each window.
2. **Unorganised (Multi-home & multi-name):** Provider config has 3 homes, model has 2,
   reasoning effort has 2 with conflicting labels (`Max` vs. `Maximum`), new-session has 5
   affordances, session lists have 3 projections, todo state has 4 renderings, "Editor" buttons
   have 2 incompatible meanings, agent header packs 11 peer elements (`AgentConversationSurface.tsx:120-250`),
   settings exposes non-savable controls with apology tooltips (`SettingsPanelView.tsx:83,90`),
   and blocking file-lock warnings expire as 4-second toasts (`editor-store.ts:284,481,489`).
3. **Sloppy (UI lies & dev leakage):** Fake 1.2s Git sync spinner (`StatusBar.tsx:119-125`),
   hardcoded `Ln 1, Col 1` (`:190`), `UTF-8` (`:186`), `'TypeScript'` fallback (`:88-92`),
   `core <hash>` (`:193`), `Output` tab badged *"Not wired yet"* (`OutputPanel.tsx:15`),
   `terminal_placeholder` (`bridge.ts:44`), user error printing a source path and *"Phase 8 UI
   slice"* (`PrimarySidebar.tsx:143`), *"later slice"* tooltip (`StatusBar.tsx:64`), lowercase
   `selora connected` (`:47`).

### 3.2 Target Region Model (Non-Negotiable Ownership Rule)

A rail icon **never** opens a panel in a different region. Placement is taxonomy, not a match arm:

| Region | Owns Exclusively | Never Contains |
|---|---|---|
| **Left rail + sidebar** | Project: Files, Search, Git | Problems, Output, Terminal, Agent, Settings |
| **Bottom dock** | Process: Terminal, Problems, Runs, **Decision Inspector** | Files, Search, Git, Agent chat |
| **Right dock** | Task (transient): one conversation, its plan, its approvals | Project browsing, Settings |
| **Agents window** | Task (durable): expanded task workspace sharing canonical state | Divergent scroll/draft, duplicate session list, duplicate config |
| **Settings (real page/modal)** | All configuration: **Providers (LLM + Decision)**, Global Instructions, Keybindings, Privacy | Embedded in 300px sidebar |
| **Command Center (`Ctrl+K`)** | Every destination (commands + tasks + files) as text | Duplicated toolbar controls |

### 3.3 The 18-Item Subtraction List (Execute Before Adding Any New UI)

1. Fake Git sync handler/button · 2. Hardcoded `Ln 1, Col 1`, `UTF-8`, `core <hash>` · 3.
`'TypeScript'` default language fallback · 4. `Output` tab (`OutputPanel`) · 5. `Problems`
icon in left activity bar · 6. Dead `VIEW_LABEL` entries (`problems`, `output`,
`terminal_placeholder`, `agent`) · 7. Dead sidebar `agent` route & `AgentSidebarView` · 8.
Hidden `Session history` button & embedded rail · 9. Floating plan toggle & redundant infeed
progress bar · 10. Duplicate permission menu & `Ask/Agent/Plan` / `Full auto-apply (Armed)`
vocabulary · 11. `ConnectModal` as a separate config path (becomes deep link to `Settings ->
Providers`) · 12. `Customizations` model/effort/provider sections · 13. Every non-savable
control using `unexposedTitle`/`unsupportedTitle` · 14. All three `*-polish.css` files (merge
into base stylesheets) · 15. Duplicate class aliases on single DOM nodes · 16. The 2,400ms
composer border beam · 17. All 75 `transition: all` declarations · 18. `.zcode/` scratch and
stray `nul` file.

### 3.4 Craft Constitution Repair (Single-Owner Pass on `tokens.css`)

* **Replace Selora palette with Skarn palette** (removing §1.8 `--space-*` / `--moonlight-*`)
  in the exact same pass that enforces the design constitution.
* **Ban violations:** Remove or strictly allowlist `gradient()` (12 files), `backdrop-filter` /
  `--glass-*` (8 files, 28 refs), `box-shadow` (16 files), and `--glow-*` (80+ refs). Every
  surviving effect must name the exact UI state it communicates.
* **Sharp radius discipline:** Keep Skarn's sharp `0 / 2 / 4` px identity; collapse `--agents-radius-*`,
  delete ~10 raw bypass values (`3px`, `5px`, `6px`, `7px`, `8px`, `12px`) and contradicting
  fallbacks (`var(--radius-2, 6px)`).
* **Motion budget:** `0` instances of `transition: all`; collapse 6 easings to 4 (keep no
  `ease-in`); longest non-splash transition `<= 300ms`; 100+/day and keyboard-initiated actions
  get zero animation; preserve both reduced-motion paths.
* **CSS target:** Reduce 20,871 CSS lines by `>= 20%` with zero unintended visual regression,
  move inline shell styles to stylesheets, and add nine-state coverage + `axe-core` CI.
* **Persistent blocking errors:** Move unsynchronized/locked file states off 4-second toasts onto
  tab indicators + Problems + status area, using the single error format: *what happened, what
  is safe right now, one action*.

### 3.5 Canonical Vocabulary (One State, One Name)

| Concept | Canonical Label | Forbidden Synonyms (Delete) |
|---|---|---|
| Permission modes | **Read-only** · **Ask before changes** · **Auto-edit** · **Full access** | Ask/Agent/Plan row; Full auto-apply (Armed); Guarded |
| Unit of work | **Task** | session, conversation, chat, thread (as interchangeable UI terms) |
| Blocking wait | **Needs approval** | pending, blocked |
| Optional wait | **Question** (+ countdown) | needs attention |
| Active execution | **Running** | thinking, breathing |
| Reversing one turn | **Undo turn / Reapply** | reset, revert (reserved for Git) |
| Agent-authored edits | **Agent changes** | changes (ambiguous with Git) |
| Workflow history | **Runs** | Automations (until triggers exist) |
| Provider config | **Providers** | Gateway, Assistant providers |
| Reasoning level | **Reasoning** (one scale: Low / Medium / High / Max) | Maximum (inconsistent with Max) |
| Creation action | **New task** | New Session |

---

## 4. Unified Execution, Trust & Decision Engine Architecture

The Decision Engine sits **alongside—never replacing—the existing general LLM runtime**. Its
reliability comes from deterministic tools, immutable workspace snapshots, trusted/untrusted
context separation, structured schemas, calibrated asymmetric policy checks, and post-action
verification—never from claiming zero hallucinations.

### 4.1 Three-Tier Execution & Hop Economics

Every agent step is routed through the Rust **Decision Orchestrator** (`crates/skarn-decision`)
into the lowest capable tier, with explicit per-tier latency deadlines, `CancellationToken`
propagation, and a strict **max hop budget** (preventing cascading provider loops or double-hop
p95 latency regressions):

| Tier | Engine | Workload | Latency / Cost Rule |
|---|---|---|---|
| **Tier 1** | Deterministic IDE Tools | Open file, locate symbol, inspect diff, check path, run targeted test | Always preferred when solvable without inference (0 model calls). |
| **Tier 2** | Closed-Ended Decision Models (**Laya** local default; BYOK **TypeSafe Jev**, **Google Gemini Decisions API**, custom decision endpoints) | Select next tool/action, scope gate, risk classification, pick 1 of N fixed candidates, continue/clarify/escalate, edit-proceed gate | Bounded latency deadline; strict JSON schema validation; never used for open-ended prose/code synthesis. |
| **Tier 3** | Existing General LLM Provider (Anthropic, OpenAI, Ollama, Custom) | Implement features, explain complex code, plan broad refactors, resolve ambiguous or escalated tasks | Invoked directly for open-ended tasks or on bounded escalation from Tier 2. |

* **Explainable routing:** Every route produces a serializable `RoutingDecision` recording
  candidate tiers considered, selected provider/model, privacy/capability filters applied, and
  reason code.
* **Bounded escalation triggers:** Confidence below calibrated action/risk threshold, provider
  schema validation failure, open-ended task classification, risk exceeding provider scope, or
  post-action verification failure.

### 4.2 Unified Decision & Execution Lifecycle FSM

Every action—whether routed from Tier 1, Tier 2, or Tier 3—executes through **one** explicit
finite state machine:

```text
Snapshotted -> Routed -> [Inferred] -> SchemaValidated -> PolicyEvaluated
  -> (AutoProceed | ApprovalPending [blocking] | QuestionPending [clocked])
  -> SnapshotRechecked (TOCTOU) -> Checkpointed -> Executed -> Verified
  (Terminal aborts at any gate: Stale | Escalated | Rejected | Cancelled | Failed)
```

### 4.3 Immutable Workspace Snapshots, TOCTOU Defense & Per-Turn Checkpoints

* **Snapshot Stamp:** Before assembling context or invoking a provider, capture an immutable
  `WorkspaceSnapshot` containing workspace root canonical path, relevant open buffer revisions
  (CodeMirror/LSP versions), SHA-256 content hashes of target files, and git HEAD/index state.
* **TOCTOU Recheck:** Immediately before mutating any file or running any approved tool,
  re-verify the live workspace against `WorkspaceSnapshot`. If relevant buffers or files mutated
  while inference or user approval was in flight, transition to **`Stale`** and abort or
  re-evaluate—**never apply a decision to drifted state**.
* **Shared Per-Turn Checkpoints (`Undo turn / Reapply`):** Immediately before mutation on a
  verified snapshot, persist content-addressed before-images keyed by `(task_id, turn_id)`.
  Restoring a turn is all-or-nothing, checks for intervening human edits before writing anything,
  and displays an explicit list of any un-restorable conflicts first.

### 4.4 Trusted vs. Untrusted Grounded Context & Project Rules

* **Compact factual assembly:** Reuse existing workspace indexing, symbol, diagnostic, and tool
  facilities—never dump full repositories into prompts.
* **Prompt-injection trust boundary:** Treat **both** model outputs and all workspace-derived
  payloads (file contents, code comments, compiler diagnostics, terminal/tool outputs) as
  **untrusted input**. Structurally separate trusted frames (system policy, action registry
  definitions, permission mode) from untrusted evidence frames in `DecisionRequest`.
* **Evidence provenance:** Every `Evidence` record carries its exact source (file path + line
  range, symbol ID, diagnostic ID, or tool call ID) and explicitly distinguishes
  `verified_fact` from `inferred`.
* **Project instructions & typed attachments:** Load global + workspace `AGENTS.md` and
  `.skarn/rules` with documented precedence and a visible *"rules in effect"* chip. Pass file,
  image, and multi-line selection attachments as typed context DTOs—**zero** machine-authored
  protocol prose (`read them with read_file`) injected into user transcripts.

### 4.5 Calibration-Aware Policy Engine, Scoped Approvals & Waits

* **Uncalibrated confidence invariant:** Never treat raw model confidence (`0.85` from Laya vs.
  `0.85` from Gemini) as calibrated or directly comparable. Configure operating points per
  `(provider, action_class, risk_level)` calibrated offline via risk-coverage curves, with
  explicit **fail-closed** defaults.
* **Unified Permission Mode x Risk Policy Matrix:**

| Permission Mode | Low-Risk Read / Inspect | Low-Risk Edit (High Calibrated Conf.) | Medium-Risk Action | High / Critical-Risk Action |
|---|---|---|---|---|
| **Read-only** | Auto | Reject / Escalate to Plan | Reject | Reject |
| **Ask before changes** | Auto | Approval (or scoped allow) | Approval | Approval (never auto) |
| **Auto-edit** | Auto | Auto (workspace-bounded + checkpointed) | Commands/external -> Approval | Approval (never auto) |
| **Full access** | Auto | Auto | Auto (workspace-bounded) | Hard boundary & critical safety gate still enforced |

* **Scoped decisions:** Approvals support **Allow once**, **Always allow (this task / this
  project)**, **Reject once**, and **Always reject (this task / this project)** with narrow,
  explicitly defined action-class keys and visible revocation in Settings.
* **Two distinct waits (never silent parking):**
  - **Needs approval** (blocking, user-owned): pauses execution indefinitely, marks task as
    waiting in the task list.
  - **Question** (optional, clocked): visible countdown (default 5 min, retroactive toggle),
    auto-continues **only** for non-destructive questions, and records *"Unanswered,
    auto-continued"* in task history.
* **Capability-bound action execution:** Model output never executes shell strings or arbitrary
  commands. Every approved action is validated against the IDE action registry, argument schema,
  canonicalized workspace-root boundary (with symlink-escape resolution), and workspace trust
  before dispatch through the controlled executor.

### 4.6 Provider-Neutral Decision Contract (Rust Canonical, TS Mirrored)

Provider wire formats never leak past their adapter module. Internal contracts are strictly
typed, versioned, and serializable across `crates/skarn-protocol` and `frontend/src/bridge`:

```rust
// Conceptual canonical contract (adapt exact derives/naming to crates/skarn-protocol)
pub enum DecisionType { Binary, Choice, Score, Action }
pub enum RiskLevel { Low, Medium, High, Critical }
pub enum EvidenceStatus { VerifiedFact, Inferred }

pub struct DecisionRequest {
    pub request_id: String,
    pub snapshot: WorkspaceSnapshot,
    pub task_type: String,
    pub question: String,
    pub options: Vec<DecisionOption>,
    pub trusted_policy: PolicyFrame,
    pub untrusted_context: GroundedContext, // source-tagged evidence only
    pub allowed_actions: Vec<ActionDescriptor>,
    pub deadline_ms: u64,
}

pub struct DecisionResult {
    pub provider: String,
    pub model: String,
    pub decision_type: DecisionType,
    pub value: serde_json::Value,
    pub probabilities: std::collections::BTreeMap<String, f64>,
    pub confidence: f64,
    pub evidence: Vec<EvidenceItem>,
    pub risk: RiskLevel,
    pub latency_ms: u64,
    pub reasoning_summary: Option<String>,
    pub validation_errors: Vec<ValidationError>,
}
```

### 4.7 Decision Providers & Adapter Seams

1. **Laya (Default Local Provider — Clean Runtime Seam First):**
   - Separate the **runtime IPC & lifecycle interface** (`LayaRuntimeAdapter` +
     `LayaLifecycleManager`) from unfinalized packaging formats. Do not fabricate a model
     format or assume an OpenAI chat endpoint works for structured decisions.
   - Lifecycle contract: hardware probe (CPU, RAM, GPU/accelerator, OS, free disk) -> package
     recommendation (Recommended / CPU-only / GPU-accelerated / Custom / Not now) -> resumable
     artifact download -> checksum + cryptographic signature verification -> IDE-managed
     isolated install dir -> local warm-up & self-test -> health/cancel/hard-kill/memory-ceiling
     -> update, version rollback, uninstall -> actionable `Ready / Failed` state.
   - Back with a deterministic + adversarial **dev/mock runtime** in Slice 1 so the entire
     engine, inspector, and tests are proven before the final Laya binary ships. Zero project
     code upload during local install or inference.
2. **TypeSafe Jev (BYOK — `https://typesafe.ai/`):**
   - Implement strictly against verified official API schemas and authentication. Pin wire-level
     HTTP record/replay contract tests. If public docs omit required fields, gate behind a
     documented manual/mock configuration mode and list every unresolved API detail in Section 8.
3. **Google Gemini Decisions API (BYOK — `https://aistudio.google.com/docs/decisions-api`):**
   - Implement strictly against official documented decision forms (`binary`, `choice`/categorical,
     `score`, etc.), schema, and auth. Pin wire-level HTTP contract fixtures; never rely on
     guessed fields if official docs differ.
4. **Custom Decision Providers:**
   - Supported via the `DecisionProvider` trait only when the endpoint genuinely supports
     structured decision outputs. Never silently coerce generic chat-completion providers into
     Tier 2.

### 4.8 Privacy-by-Construction, Observability & Shadow-Mode Evaluation

* **Local-only privacy enforcement:** Workspace-level `local_only` mode is enforced inside the
  Rust orchestrator before any adapter dispatch—physically blocking cloud decision and LLM
  egress. Remote provider data-retention policy is surfaced as an explicit user setting.
* **Secret hygiene:** All BYOK keys live exclusively in Windows Credential Manager (`skarn-ide`).
  Zero API keys or auth headers in logs, error payloads, crash reports, or telemetry.
* **Privacy-safe structured telemetry:** Records provider/model, decision type, confidence bucket,
  latency, validation result, approval outcome, escalation hop count, verification result, and
  hosted usage/cost estimate. Never records secrets or unrestricted source code.
* **Decision Inspector (Bottom/Process dock):** Inspects per-decision request context summary,
  routing explanation, selected provider, candidate probabilities, source-tagged evidence,
  validation errors, policy outcome, snapshot check, and execution/verification result.
* **Offline Shadow-Mode Replay & Evaluation Harness:** Replays labeled IDE decision traces across
  deterministic rules, Laya, Jev, and Gemini (strictly isolated from production telemetry) to
  produce **confusion matrices, Expected Calibration Error (ECE), risk-coverage curves, hop
  distributions, net p50/p95 latency delta, and LLM-call reduction vs. baseline**.

---

## 5. Contested Decisions, Refused ZCode Warts & Non-Goals

### 5.1 Six Contested UX Decisions (Resolve via Evidence Before Wave 2)

1. Four permission modes vs. three honest modes? (Default plan assumes 4: adds `Auto-edit`.)
2. Merge Agents window + dock into one surface, or keep two with strict shared state?
3. Is a 5-minute auto-continuing question timer safe for file-writing agents? (Default plan
   restricts auto-continue strictly to non-destructive questions.)
4. Per-turn content-addressed checkpoints vs. pre-turn Git snapshots? (Default plan uses
   content-addressed checkpoints unified with Decision Engine TOCTOU snapshots.)
5. Typed `@ # / $` grammar vs. attachment-driven UI? (Default plan supports typed grammar + `+`
   menu in Wave 2.)
6. Does a plugin/skill ecosystem fit a BYOK Windows workbench? (Deferred to Wave 3.)

### 5.2 ZCode Warts Explicitly Refused

* Unauditable memory (memory must be visible/editable from day one).
* Unrecoverable side conversations on close (persist as child thread or label loss loudly).
* Single-line-only selection capture (support multi-line with explicit cap).
* Blocking Goal mode inside Plan mode (allow composition).
* Plugin/settings UI requiring an open workspace.
* Non-retroactive countdown toggle.
* Auto-continuing destructive questions.

### 5.3 Deliberate Non-Goals (Out of Scope)

Phone remote control, bot channels, hosted plugin marketplace, repo wiki, SSH/Docker remote dev,
mobile emulators, second changes panel, and rewriting git history.

---

## 6. Master Phased Execution Plan & Shared-File Ownership

Do not start a stage until the preceding stage gate is green.

| Stage | Scope | Shared Files Touched | Hard Exit Gate |
|---|---|---|---|
| **Stage 0** (1-2d) | **P0 Supply-Chain & Ground Truth:** Create `<owner>/skarn`, repoint/disable updater, delete hardcoded repo URLs & `api.selora.lol`, start signing cert, run 30-min name clearance, record baseline session + trace set, verify live Jev & Gemini Decisions API docs. | `Cargo.toml`, `state.rs` (updater lines), `updates.rs` | Built binary has 0 `api.selora.lol` & 0 old-repo slug; fixture with old repo `9.9.9` offers **no** update; `NAME_DECISION.md` & Phase 0 technical brief signed off. |
| **Stage 1** (4-5d) | **Rebrand + All 8 Migrations + Gateway Removal:** Commit 1: mechanical rename (`selora-*` -> `skarn-*`, types, binary, lockfile, 0 logic diff). Commit 2: all 8 migrations (Tauri ID, keep Installer GUID/dir, `%APPDATA%\skarn` + `serde(alias="selora")`, keyring `skarn-ide`, `SKARN_*` env aliases, 11 `localStorage` keys, atomic 5 Tauri event channels, `selora-dark` alias) + Selora Gateway removal + BYOK first-run. | `Cargo.toml`, `tauri.conf.json`, `selora-ide.iss`, `settings.rs`, `keyring.rs`, `bridge.ts`, `ConnectModal.tsx` | `cargo test --workspace`, `npm run typecheck`, `npm test`, `npm run build` green; clean Windows VM upgrade `0.1.0 -> 1.0.0` retains settings, keys, pinned sessions, open tabs, 1 uninstall entry. |
| **Stage 2** (5-6d) | **Shell Subtraction, Routing & Craft Constitution:** Execute 18-item deletion list; enforce region ownership in `apply_toggle`; promote Settings to real page (`Settings -> Providers`); collapse agent header/nav duplicates; single-owner `tokens.css` pass (Skarn palette + ban glass/glow/grad/shadows + 0 `transition: all` + sharp `0/2/4` radii + merge `*-polish.css`); persistent blocking file-lock errors. | `state.rs` (layout), `bridge.ts`, `ActivityBar.tsx`, `PrimarySidebar.tsx`, `StatusBar.tsx`, `SettingsPanelView.tsx`, `AgentConversationSurface.tsx`, `tokens.css` | Every rail icon lights up and opens right; 0 fake status values; `transition: all` = 0; CSS lines down `>= 20%`; Settings survives closing sidebar; `axe-core` clean on 6 screens. |
| **Stage 3** (6-8d) | **Unified Execution & Decision Kernel (Vertical Slice 1):** `crates/skarn-decision` + `crates/skarn-protocol` contracts; immutable `WorkspaceSnapshot` + TOCTOU recheck + shared per-turn `Undo turn / Reapply` checkpoints; unified Lifecycle FSM & calibrated Policy Engine (4 canonical modes + scoped allow/deny + blocking Approval vs. clocked Question); `AGENTS.md`/`.skarn/rules` + typed attachments; Tier 1 fast-path + Tier 2 orchestrator with deterministic/adversarial mock provider + bounded Tier 3 escalation; 1 low-risk + 1 mutating real IDE action; **shadow-mode replay harness**. | `skarn-protocol/src/lib.rs`, `state.rs`, `sessions.rs`, `AgentComposer.tsx`, `ToolCard.tsx`, `AgentQuestionCard.tsx` | **0%** escape in adversarial property/fuzz suite; TOCTOU & cancellation tests pass with 0 stale mutations or side effects; undo reverses 1 turn preserving human edits; 0 protocol text in transcripts; shadow-mode report published. |
| **Stage 4** (6-8d) | **Decision Providers, Laya Local Seam & Observability (Vertical Slice 2):** `LayaRuntimeAdapter` + `LayaLifecycleManager` (hardware probe, signed-artifact verification hook, isolated dir, warm-up/self-test, kill/rollback/uninstall, dev runtime); **TypeSafe Jev** & **Google Gemini Decisions API** adapters with wire contract tests (or documented manual/mock mode for unverified fields); `skarn-ide` keyring integration; workspace `local_only` privacy enforcement; `Settings -> Providers` (Decision Providers UI) + Bottom-dock `Decision Inspector` + privacy-safe telemetry & A/B eval mode. | `skarn-settings/src/keyring.rs`, `SettingsPanelView.tsx`, `BottomDock.tsx`, `crates/skarn-decision/*` | Wire contract fixtures green; secret-hygiene tests prove 0 leaked keys/headers in logs/errors/telemetry; `local_only` blocks 100% cloud calls; Laya install/self-test/rollback/uninstall lifecycle passes on dev runtime. |
| **Stage 5** (5d+) | **Flow Polish & v1.0.0 Ship:** `@ # / $` grammar, `Ctrl+K` Command Center, `Ctrl+F` in conversation, keyboard map (`Ctrl+N/M/T`, `Shift+Tab`, fix `editFindInFiles`), side conversation, Playwright CI, full Windows 11 VM release matrix, signed `Skarn-Setup-1.0.0.exe`, archive old repo. **Post-1.0 Wave 3** (skills, goal loop, custom subagents, MCP/hooks, browser picker, rich preview) gated on Stage 3-5 metrics. | `commands.ts`, `AgentComposer.tsx`, `.github/workflows/release.yml` | Full Section 7 verification suite passes; signed installer valid under new publisher; `grep -ri selora` = LICENSE + CHANGELOG + `docs/research/` allowlist only. |

**Shared-File Ownership Lock:** At most one writer at a time on `skarn-protocol/src/lib.rs`,
`state.rs`, `sessions.rs`, `settings.rs`, `keyring.rs`, `bridge.ts`, `agent-store.ts`,
`AgentComposer.tsx`, `tokens.css`, and `commands.ts`. Any PR touching more than two shared
integration files must be split into a sequencing commit.

---

## 7. Objective Definition of Done (External Proof, Not Claims)

### 7.1 Decision Engine & Security Proof Gates (Automated)

1. **Zero-escape adversarial & property-based suite (`proptest` / `fast-check`):** Adversarial
   mock provider emitting malformed JSON, unregistered actions, path-traversal/symlink escapes,
   prompt-injected diagnostics, privilege escalations, and spoofed `1.0` confidences -> **0%**
   bypass validation, workspace root, `local_only` privacy mode, or high/critical approval gates.
2. **Deterministic TOCTOU & cancellation tests:** Mutate open editor buffers/files mid-inference
   and cancel in-flight calls -> **0** stale executions, **0** corrupted edits, **0** dangling
   child processes or side effects.
3. **Shadow-mode replay report:** Published metrics on real/labeled traces showing confusion
   matrix, calibration/risk-coverage behavior, bounded hop counts, net p50/p95 latency delta,
   and measurable LLM-call reduction without task degradation.
4. **Wire-level contract & secret-hygiene tests:** Record/replay HTTP fixtures validating Jev
   and Gemini request/response translation; automated assertions proving API keys and auth
   headers never appear in logs, error objects, crash dumps, or telemetry.

### 7.2 Shell, Craft & UX Proof Gates (Automated + Observed)

5. **Structural & Craft CI checks:** Rail test (every icon opens right and lights up); 0 dead
   clicks; status truth test (0 constant values); grep proves 1 label per canonical concept and
   0 `transition: all`; radii `<= 4` values with 0 raw bypasses; longest non-splash animation
   `<= 300ms`; CSS line count down `>= 20%`; `axe-core` clean on 6 screens; 0 protocol English
   in user transcripts.
6. **Behavior delta vs. Stage 0 baseline:** Approvals/session down `>= 40%` without raising
   `Full access` usage; silent waits `> 2 min` -> `0`; `Undo turn` dominates `git checkout`;
   first-time user completes *small fix -> review -> revert* with zero clarifying questions.

### 7.3 Rebrand & Release Windows 11 Clean-VM Matrix (All 10 Must Pass)

1. Fresh install clean of `"Selora"` in UI/taskbar/title · 2. Upgrade `0.1.0 -> 1.0.0` retains
settings, provider, 11 `localStorage` keys (no re-onboarding), 1 uninstall entry, 0 orphan
`Selora IDE` folder · 3. Keyring migrated to `skarn-ide`, old entry deleted, 0 plaintext
`.api_key` · 4. Settings migrated with `migrated_from` marker, JSON key alias verified · 5. Old
repo `9.9.9` produces **no** update offer; new repo `1.0.1` does · 6. Binary string scan clean ·
7. Repo `grep -ri selora` allowlist-only · 8. Gateway absent, Ollama works offline, nothing
preselected · 9. Clean uninstall · 10. Authenticode signature `Valid` under new publisher.

---

## 8. Owner Decisions & Unresolved External Dependencies Ledger

Stop and confirm before implementing dependent items—never invent answers:

| # | Item | Status | Blocks |
|---|---|---|---|
| 1 | `Skarn` clearance run (`docs/brand/NAME_DECISION.md`) & confirmation of `Skarn` (not `Skarnify`) | Pending owner (30 min) | Stage 1 assets & Stage 5 publish |
| 2 | Legal publisher entity for `AppPublisher` & OV/EV code-signing cert | Pending owner | Stage 0 cert application & Stage 5 release |
| 3 | Control status of `github.com/ToxicAngelX/Selora-IDE` | Pending owner | Urgency of Stage 0 updater kill-switch & archive |
| 4 | Domain + docs host (About dialog, `F1`, installer URL) | Pending owner | Stage 1/4 copy & links |
| 5 | `TypeSafe Jev` live API wire schema, auth model & rate limits (`https://typesafe.ai/`) | Verify in Stage 0 | Stage 4 Jev live mode (gate behind mock/manual if incomplete) |
| 6 | `Google Gemini Decisions API` live schema, decision forms & auth (`https://aistudio.google.com/docs/decisions-api`) | Verify in Stage 0 | Stage 4 Gemini live mode |
| 7 | `Laya` final model format, runtime binary packaging, quantization & signature key distribution | Unfinalized by design | Live Laya inference (Stage 4 ships full lifecycle seam + dev runtime) |
| 8 | Provenance & licence verification of `img-fx` and `metal-fx` crates | Verify in Stage 1 | Stage 5 v1.0.0 release |