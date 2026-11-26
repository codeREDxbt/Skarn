# Rebrand & De-brand Plan

> Renaming the product away from **Selora**, removing the first-party **Selora API Gateway**,
> and doing it without losing users' settings, keys, or trust — plus the one item that must ship
> before anything else because it is a security problem, not a cosmetic one.
>
> `Selora` appears in **358 files**. This is a migration with five breaking identities, not a
> find-and-replace. No implementation code in this document.
>
> **Name decided (2026-11-25): `Skarn`** — lowercase `skarn`, identifier `app.skarn.desktop`,
> installer output `Skarn-Setup-<version>.exe`, settings dir `%APPDATA%\skarn`, keyring service
> `skarn-ide`, env prefix `SKARN_`. The body below keeps the `<NAME>` / `newname` placeholder so the
> text stays correct if the name changes after trademark clearance; read `<NAME>` as `Skarn`.
>
> **On "skarnify":** you wrote it with the `-ify` suffix. I have recorded the name as **Skarn**
> because §1.3 rules out `-ify` for a developer tool (it is the consumer-app suffix, it lengthens a
> one-syllable name to four, and it reads as a verb about the product rather than a name for it).
> If you meant the `-ify` form literally, say so and the token changes everywhere; nothing else in
> this plan depends on which spelling wins.

---

## 0. Do this first (before the name, before the logo)

**The auto-updater currently trusts a repository you no longer control.**

- `crates/selora-ai/src/updates.rs` builds its endpoint from `CARGO_PKG_REPOSITORY` (good — one line), but
- `apps/selora-desktop/src-tauri/src/state.rs:6055` and `:6058` **hardcode**
  `https://api.github.com/repos/ToxicAngelX/Selora-IDE/releases/latest`, and
- the asset matcher looks for `Selora-IDE-Setup-<version>.exe` + `.sha256` (`updates.rs:149`), and
- `tauri.conf.json` allows `https://api.github.com` in `connect-src`, and
- the installer/signing flow publishes signed installers under the old publisher.

If an old build (or a new build that still carries the old slug) runs its update check and the old
repository publishes a higher version, that build will offer to download and run an installer from
the old owner's namespace. After you leave, that is a remote-code path into your users' machines
that you cannot audit.

**Mechanism, corrected** (an earlier draft of this section said Authenticode was the gate; that was
wrong in a way that matters): there is **no Tauri updater** in this app — `tauri.conf.json` sets
`bundle.active: false` and has no `plugins.updater` block, so there is no pubkey and no signature
check. `crates/selora-ai/src/updates.rs` is a custom client: GET the release, compare `tag_name` as
a version triple, match the `Selora-IDE-Setup-<tag>.exe` asset and its `.sha256` sibling — both
**from the same release**. A checksum that travels with the binary proves *integrity, not
authenticity*: whoever controls the release controls both files. The only remaining gate is Windows
Authenticode on the installer, and by default that is the old publisher's certificate opening it.
So treat the existing update path as **untrusted**, and fix it by repointing, not by trusting the
hash.

**Therefore, before the rebrand ships:** repoint or disable the update check, repoint the asset
matcher, and make the new build refuse anything not published under the new slug. Then, when the
rename lands, release it as **1.0.0** so a semver compare can never prefer an old `0.x`.

This is item #1 in the work order (§7, P0) and the first row of the risk table (§9).

---

## 1. Naming

### 1.1 What was checked, and what failed

I checked six candidates instead of guessing. Every one is already used — and five of the six are
used by a **coding agent or code editor**, which is the worst possible collision.

| Candidate | Already exists | Verdict |
|---|---|---|
| **Cairn** | `cairn/cairn` — "a terminal-based coding agent"; `cairn.computer` — "a durable system for agents"; `cairn-dev/cairn` — background coding agents; `cairn-framework/cairn` — agent architecture memory | Reject — four direct AI-coding collisions |
| **Burin** | `burin.ai` — personal AI agent; **"Burin Code"** — a coding workbench; `burin` crate — a Rust GUI framework; a macOS automation tool | Reject — same category |
| **Umber** | `Spillebulle/Umber` — a GPU painting app in Rust | Reject — Rust desktop creative tool, adjacent audience |
| **Mullion** | `jchigg2000-git/mullion` — a macOS window manager built around **panes and 6-pane grids** | Reject — identical metaphor, adjacent product |
| **Kerf** | `kerf.fyi` — a web editor with Monaco + live preview; `OrellBuehler/kerf` — a Rust/Tauri editor with MCP; a CAD app; a PyPI workflow engine | Reject — three editors |
| **Chamfer** | `chamferonline.com` / `SmartAI/Chamfer` — an AI agent that writes CAD; "Chamfer AI" in the Microsoft Store; a PyPI package | Reject — AI-agent brand density |

**Conclusion, and it changes the strategy:** in 2026, every short, evocative craft noun is
already an AI coding product. Borrowing a word means borrowing someone else's search results and,
worse, someone else's category association. **The name must be coined**, not found.

Also rejected without needing a search (already-known dev/tool collisions):
Rook (Ceph `rook.io`), Loom (video), Shuttle (Rust framework), Kestrel (ASP.NET), Cinder
(libcinder), Graphite (`graphite.dev` + Graphite editor), Onyx (agents platform), Codex (OpenAI),
Spline (design tool), Vellum (book formatting), Chisel (hardware language), Tracer, Atlas, Nova.

### 1.2 Recommendation — accepted

**`Skarn`** (chosen 2026-11-25; trademark, domain and package clearance still outstanding — §1.3) — a real geological term: the zone where rock is transformed at the boundary between
magma and surrounding stone. The ore is at the contact, not in either body.

Why it fits this product specifically: the whole dossier is about the boundary between the human
and the agent, and about work happening at that contact. It is one syllable, two hard consonants,
spellable after hearing it, contains no "-AI", no "-ly", no "-ify", no "-io", and does not
describe a colour, a place, or a generic virtue. It also survives the two tests that killed the
others: it is not a craft noun (so it is not already a woodworking tool's brand) and not a pane
metaphor (so it is not already a window manager).

**Backups, in order:**

| Name | Story | Risk |
|---|---|---|
| **Pelorus** | The sighting instrument on a ship's binnacle used to take bearings — finding direction in a large space | Three syllables; slightly nautical; must verify |
| **Alidade** | The sighting rule on a surveyor's plane table — precision measurement before you cut | Three syllables; obscure; must verify |

All three are **unverified**. That is deliberate: a name is a legal and commercial decision, and
the check takes thirty minutes. Do it before you print anything.

### 1.3 The verification protocol (do this, don't ask an agent to guess)

For each candidate, in this order — stop at the first hit:

1. **Trademark**: USPTO TESS and EUIPO, classes **9** (software) and **42** (SaaS/development).
   Look for *similar*, not identical, marks in developer tools.
2. **Category search**: `"<name>" IDE`, `"<name>" code editor`, `"<name>" coding agent`,
   `"<name>" developer tool`. This is what killed all six above.
3. **GitHub org/repo**: `github.com/<name>` and a search for `<name>` + "code editor".
4. **Packages**: `npmjs.com/package/<name>`, `crates.io/crates/<name>`, `pypi.org/project/<name>`.
5. **Domain**: `<name>.dev`, `<name>.app`, `<name>.com` (record whether the .com is parked or live).
6. **Store name**: Microsoft Store / WinGet id availability if you plan to publish there.
7. **Handles**: the same handle on GitHub/X/Bluesky, for support.

Record the result in a small table in the repo (`docs/brand/NAME_DECISION.md`) with the date. If
all three fail, coin from a pattern instead of borrowing: a rare geological/nautical/optical root
+ no suffix, 1–2 syllables, hard consonant, no English word (e.g. *skarn*, *pelorus*, *alidade* are
that pattern). Never: `-AI`, `-GPT`, `-ify`, `-ly`, `-io`, "Code", "Dev", "Studio" as the whole
name.

### 1.4 Brand architecture

- **Product name**: `<NAME>` (no "IDE" inside it — the suffix is generic and hurts trademark
  strength and search).
- **Descriptor** (allowed, secondary): "`<NAME>` — code workbench" on the site, never in the app
  title.
- **Window title / taskbar**: `<NAME>`.
- **Installer / uninstall entry**: `<NAME>`.
- **Version**: **1.0.0** for the rebrand (also solves the updater comparison, §0).
- **Tagline direction**: about the boundary between you and the machine, not about "AI-powered".

---

## 2. The rename surface (measured, not estimated)

**358 files** mention the name outside build output. Occurrence counts (case-sensitive):

| Form | Count | Meaning |
|---|---|---|
| `Selora` | 659 | Display strings, doc comments, product copy |
| `selora` | 593 | Identifiers, paths, URLs, ids |
| `selora_ai::` | 446 | Rust module paths |
| `selora-desktop` | 419 | Crate/binary/target name |
| `SeloraError` | 309 | **Public error type across all crates** |
| `selora_protocol::` | 214 | Rust module paths |
| `selora-ui` | 212 | Crate name |
| `selora_settings::` | 205 | Rust module paths |
| `SeloraStatus` | 133 | Type name |
| `Selora-IDE-Setup-` | 33 | **Update asset name + installer output** |
| `selora:` | 29 | DOM custom events |
| `selora-dark` | 30 | **Persisted theme id** |

### 2.1 Categories and exact locations

| # | Category | Where | Breaking? |
|---|---|---|---|
| 1 | Crate names (12) | `Cargo.toml` members, each `crates/*/Cargo.toml`, path deps in `[workspace.dependencies]` | Build |
| 2 | Rust module paths | `use selora_*::` across 99 files in `crates/` | Build |
| 3 | Public types | `SeloraError` (309), `SeloraStatus` (133), `SeloraClient` (59) | API |
| 4 | Binary + target | `selora-desktop.exe`, `-p selora-desktop`, `start.bat`, `launch-selora.bat` | Build + release |
| 5 | Tauri identity | `productName: "Selora IDE"`, **`identifier: "lol.selora.ide"`**, window title | **Runtime data** |
| 6 | Installer | `installer/selora-ide.iss`: `AppId={{7A3E9C41-5D2B-4E8F-B6A1-9C0D2E4F6A8B}}`, `AppName`, `AppPublisher=Toxic`, `DefaultDirName={localappdata}\Programs\Selora IDE`, `OutputBaseFilename=Selora-IDE-Setup-{version}`, `UninstallDisplayName` | **Registry + files** |
| 7 | Settings dir | `%APPDATA%\selora-ide\settings.json`, plaintext fallback `.api_key`, `SELORA_SETTINGS_DIR` | **User data** |
| 8 | Credential store | `keyring::SERVICE = "selora-ide"` (+ entries `api-key`, `brave-key`) | **User data** |
| 9 | Env vars | `SELORA_API_KEY`, `SELORA_WORKSPACE_ROOT`, `SELORA_SETTINGS_DIR`, `SELORA_ISCC`, `SELORA_SETUP*` | Dev/CI |
| 10 | Network identity | `api.selora.lol` (gateway), `github.com/ToxicAngelX/Selora-IDE` (docs + updates), `api.github.com` in CSP | **Supply chain** |
| 11 | Frontend | package name, `SeloraMark`, 13 `selora:*` DOM event names + 5 cross-process channels (`selora://*`, `selora:update-progress`), 11 localStorage keys, theme id `selora-dark`, `provider-visibility.ts:37`, tests asserting `selora-fast` — see §3.6, §3.7 | **Runtime data** |
| 12 | Copy | Status bar "selora connected", "Selora's system prompt", "Gateway …" strings, onboarding, error text | UX |
| 13 | Design tokens | `--space-*`, `--moonlight-*`, header note "Selora Monochrome Space System (Website parity — `E:\Projects\Selora`)" | **Brand-derived colours** |
| 14 | Docs/build | `README.md`, `CHANGELOG.md`, `SIGNING.md`, `.github/workflows/release.yml`, `deny.toml`, `frontend/package.json` repository field | Release |
| 15 | Repo hygiene | `.zcode/` scratch (30 files mention the name), stray `nul` file | Cleanup |

---

## 3. The five breaking identities (and how to migrate each)

These are the only parts where a naive rename damages users.

### 3.1 Tauri identifier — `lol.selora.ide`

Changes the app's Windows identity and the WebView2 user-data directory. Settings live in your own
`%APPDATA%` folder, so they are not moved by this — but **window state, webview cache and any
Tauri-managed storage are**.

**Plan:** new identifier (`app.<name>.desktop` or your reverse-DNS). On first launch, if the new
Tauri data dir is absent and the old one exists, copy it. Accept a one-time loss of window
geometry if copying is fragile; never accept a loss of settings.

**Test:** install 1.0, confirm window opens at a sane size, settings intact, no crash on missing
old dir.

### 3.2 Installer AppId — `{7A3E9C41-…}`

This GUID is the uninstall identity. Change it and Windows treats the new build as a **different
product**: side-by-side installs, two uninstall entries, two Start-menu entries, two copies of the
exe.

**Plan (recommended):** **keep the GUID** for the rebrand release so the new installer upgrades in
place, and keep `DefaultDirName` unchanged for v1.0 — change only the display strings (`AppName`,
`UninstallDisplayName`, `AppPublisher`, `OutputBaseFilename`). Moving the install folder in the
same release leaves orphaned files behind, because Inno Setup does not delete files from the old
path on an in-place upgrade. Move the folder in **1.1** with an explicit `[InstallDelete]` and a
test that no `Selora IDE` folder survives.

**Alternative:** new GUID = side-by-side, which is only acceptable if you ship a "remove the old
app" step and expect users to migrate manually. Do not pick this for a first release.

### 3.3 Settings directory — `%APPDATA%\selora-ide`

**Plan:** new `%APPDATA%\<newname>`. Migration order on first launch:
1. If new dir exists → use it (normal case).
2. Else if old dir exists → copy the tree, write `migrated_from: "selora-ide"` + timestamp into the
   new `settings.json`.
3. Keep the old dir **read-only fallback for one release**, then stop reading it; delete only on
   explicit user action or in 1.2.
4. The plaintext `.api_key` fallback must move with the tree and be deleted from the old location
   **only after** the keyring migration succeeds (§3.4).
5. **The file's top-level key is also `selora`.** `crates/selora-settings/src/settings.rs:135`
   reads `pub selora: SeloraSettings`, so the JSON is `{"selora": {…}}`. Copying the directory
   achieves nothing if the new build looks for a key named after the new product: deserialization
   fails and **every setting silently resets**. Rename the field, put
   `#[serde(alias = "selora")]` on the new name for one release, keep *writing* the new key, and
   drop the alias in 1.1. This is the most damaging item in §3 precisely because it fails as a
   reset, not an error — nobody files a bug for "it forgot my theme".

**Test:** upgrade from 0.1.0 with a configured provider → app opens connected, no re-entry, old
folder untouched, and the new `settings.json` parses with the renamed key while the old one is
still readable.

### 3.4 Credential store — service `selora-ide`

Windows Credential Manager entries are keyed by `(service, user)`. Renaming the service makes every
stored key invisible: the app looks connected-and-keyless, and users must re-enter.

**Plan:** on first launch after upgrade, if the new service has no `api-key` entry:
1. read the old service's entry;
2. write it to the new service;
3. **only after a successful write**, delete the old entry;
4. log one line ("credential migrated") with no key material.

If the read fails (permissions, absent), show one clear prompt: "Your saved key could not be
carried over — re-enter it" with a link to the provider, rather than silently appearing
disconnected.

**Test:** migrate, restart, disconnect, verify old entry is gone and no plaintext copy survived.

### 3.5 Environment variables — `SELORA_*`

Used by dev/CI and by `SELORA_API_KEY` at runtime (`commands.rs:528`, `sessions.rs:155`).

**Plan:** rename to `<NAME>_*`, but **keep reading the old names for one release** with a
deprecation warning in the log. This costs ten lines and prevents every CI script and dev shell
from breaking on upgrade day. Remove the aliases in 1.1.

### 3.6 Browser storage — 11 keys, all user data

`localStorage` holds frontend state that never reaches `settings.json`. Renaming the prefix without
a migration resets each of these **silently**:

| Key | Lost if not migrated |
|---|---|
| `selora:model_settings` | The agent's model and provider selection |
| `selora:agent-pinned-sessions` | Pinned agent sessions |
| `selora:agent-draft:<key>` | Unsent composer text, per surface/workspace/session |
| `selora:editor-session:<path>` | Open files and the active tab, per workspace |
| `selora:last-editor-workspace` | Which workspace reopens |
| `selora:editor-preferences` | Editor preferences |
| `selora:recent-commands` | Command-palette recency |
| `selora:agents-workspace:v1:<id>` | Pane widths, open/closed state per workspace |
| `selora:ui-sounds` | Sound preference |
| `selora:notification_history` | The 50-entry notification history |
| `selora:onboarding_seen` | Whether onboarding ran — a reset re-onboards every existing user |

**Plan:** one table-driven migration in the frontend bootstrap, running **before any store reads a
key**: if the new key is absent and the old one is present, copy the value and leave the old key in
place until the release after next. Do not do it with a string replace — three keys are
parameterised (`editor-session:<path>`, `agent-draft:<json>`, `agents-workspace:v1:<id>`) and the
path inside the name must be preserved verbatim.

**Test:** seed all 11 keys in a profile, upgrade, confirm each value appears under the new key and
that `onboarding_seen` does not re-run onboarding.

### 3.7 Cross-process event names — five channels, one commit

These are not strings in one place; they are a protocol between Rust and JS. A partial rename does
not fail loudly — the `listen` simply never fires.

| Channel | Emitted in | Listened in |
|---|---|---|
| `selora://event` | `src-tauri/src/state.rs:1271` | `frontend/src/bridge/bridge.ts:1609` |
| `selora://agent-preferences` | `src-tauri/src/agents_preferences.rs:161`, `:304` | `bridge.ts:967` |
| `selora://agents-navigate` | `src-tauri/src/agents_window.rs:170` | `bridge.ts:1029` |
| `selora://main-session-navigate` | `src-tauri/src/agents_window.rs:233` | `bridge.ts:1033` |
| `selora:update-progress` | `src-tauri/src/updates.rs:19` | `bridge.ts:1587` (+ `UPDATE_PROGRESS_EVENT`) |

**Plan:** change every emit and listen site in **one commit**, and update
`bridge/agents-bridge.test.ts:72`, which asserts the literal `selora://agents-navigate`. If the
rename has to be split across phases (crate rename first, frontend later), add a temporary
dual-emit for one commit rather than leaving a window where names disagree — otherwise the event
log, preference sync and window-to-window navigation all go quiet while the app still builds and
the UI still renders, which reads as an engine bug and will be debugged in the wrong place.

**Test:** after the rename, exercise one flow per channel — an agent turn (event log), a preference
change (sync), rail navigation from the agents window, and an update check (progress).

---

## 4. Removing the Selora API Gateway

Goal: the app becomes **bring-your-own-key only**, with no endpoint, key format, or copy that
belongs to the old company.

### 4.1 Frontend, file by file

`features/auth/ConnectModal.tsx`:
- Delete the preset object `id: 'selora'` — "Selora API Gateway", `baseUrl: 'https://api.selora.lol'`,
  placeholder `sk-selora-... or sk-gw-...`, description, `tag: 'Recommended'`, `iconType: 'sparkles'`.
- Delete the validation branch `if (presetId === 'selora')` and both `sk-selora-` / `sk-gw-` hints.
- Change the defaults: `useState<string>('selora')` → no preselection; `useState('https://api.selora.lol')`
  → empty.
- Rename the docs-host allowlist comment/none of its entries belong to Selora — keep only the
  provider hosts.
- Copy: "Model Provider & Gateway Connection" → "Model provider"; "Gateway Base URL" → "Base URL";
  "Gateway Connected / Not Connected" → "Provider connected / not connected"; "Testing Gateway…" →
  "Testing connection…"; "Settings > AI Gateway" → "Settings > Providers"; "Assistant providers" →
  "Providers"; "Appended to Selora's system prompt" → "Appended to the system prompt".

`features/auth/provider-visibility.ts:37`: delete the `haystack.includes('selora') → 'selora'`
heuristic (it exists only for the gateway's model names).

Tests: `ConnectModal.providers.test.tsx:67` (disables `'selora'`), `provider-visibility.test.ts:13,30,57`
(`'selora'`, `'selora-fast'`) — rewrite around a neutral provider.

`features/auth` tag vocabulary: OpenRouter is tagged `'Gateway'` → rename to `'Aggregator'` so
"gateway" stops meaning two things.

Status bar (`StatusBar.tsx`): `selora connected` → provider name or neutral "Connected".

Onboarding / About / docs links: remove gateway mentions.

### 4.2 Rust side

- Any default base URL constant or settings default that points at `api.selora.lol` — search and
  delete; the base URL becomes required for custom providers and preset-provided otherwise.
- Any gateway-specific header or user-agent string (`x-selora-*`, `selora/…` UA).
- `disabled_providers` defaults containing `'selora'`.
- Tests with `api.selora.lol` fixtures.

Note: the CSP does **not** list `api.selora.lol` (provider calls go through Rust `reqwest`), so
removing the gateway needs no CSP change — but the CSP does list `api.github.com`, which stays only
while the update check exists (§0).

### 4.3 The first-run replacement (the part people forget)

Deleting a preset changes the first-run experience, so design it rather than leaving a hole:

- **No preselection.** An empty grid with four cards: **Anthropic · OpenAI · Local (Ollama) ·
  Custom endpoint**. Nothing is highlighted; nothing is "Recommended". A recommendation implies an
  agreement the new product does not have.
- **Local first for trust**: Ollama needs no key, so it is the honest default *suggestion*, not the
  default *selection*.
- **Copy**: "Bring your own key. Keys stay in Windows Credential Manager." One sentence, no
  "gateway".
- **Empty state**: the composer already blocks send with a reason (`no_model`); make that reason
  say "Choose a provider to start" and link to the same modal.
- **Model catalog**: with no gateway there is no curated list; the provider's `/models` response is
  the only source. Keep the "Custom model id" override — it is the escape hatch for aggregators.

---

## 5. Repository, supply chain, signing, and release identity

### 5.1 Bootstrap (you are creating the repo)

Because the repo is new, the identity is right from the first commit instead of being repaired
later. Order matters: the repo must exist before P0 lands, or P0 has nowhere to point.

1. **Create `<owner>/skarn`** (private at first). Description: "Code workbench." README's second
   line: "This project began as Selora IDE; the original copyright notice is retained in LICENSE."
   Topics: `code-editor`, `tauri`, `rust`, `agents`.
2. **One source of truth for the repo URL.** Set `[workspace.package] repository` in the root
   `Cargo.toml`. The updater already derives its endpoint from `CARGO_PKG_REPOSITORY`; then **delete
   the hardcoded copies** at `state.rs:6055` and `:6058`, `App.tsx:94`, `AgentsWindowApp.tsx:177`,
   `frontend/package.json` `repository.url`, and the installer's `RepoUrl`. Add a CI check that
   greps for a second occurrence of the slug.
3. **Release asset names.** `Skarn-Setup-<version>.exe` + `.sha256`; update the matcher at
   `updates.rs:149` and its fixtures at `:397-453` in the same commit as the installer change.
4. **Secrets under new names.** `SKARN_SIGNING_CERT_BASE64`, `SKARN_SIGNING_CERT_PASSWORD`,
   `TAURI_SIGNING_PRIVATE_KEY`; delete the `selora-signing.pfx` path in `SIGNING.md`. Old secret
   names must not be reused, and the old cert must not sign the new brand.
5. **Do not import old releases.** No re-uploading old installers, no copying old tags. Tag the
   first release `v1.0.0`.
6. **Branch protection + required checks** (`cargo test`, typecheck, `npm test`, build) so a
   half-landed rename cannot be merged.
7. **Docs host.** GitHub Pages from `docs/`, or the new domain. Point the About dialog, `F1`, and
   the installer's URL here — these are three separate hardcoded strings today.
8. **Old repo.** Archive it after 1.0.0 ships; if you don't control it, treat P0 as urgent (§0).

---

### 5.2 Supply chain, signing, and release identity

| Item | Current | Change | Why it must not be skipped |
|---|---|---|---|
| Update endpoint | `CARGO_PKG_REPOSITORY` + two hardcoded URLs (`state.rs:6055,6058`) | Point the workspace `repository` at the new repo; delete the hardcoded ones; assert one source | A second copy is how a rename half-lands |
| Update assets | `Selora-IDE-Setup-<tag>.exe` + `.sha256` matcher (`updates.rs:149`), tests at `:397-453` | New asset name + matcher + fixtures | Mismatch = "update available" with no installable file |
| Docs link | `App.tsx:94`, `AgentsWindowApp.tsx:177` hardcode the old URL | New docs URL | 404 on every `F1` |
| Publisher | `AppPublisher=Toxic`, `RepoUrl` in the `.iss` | New publisher entity | The installer metadata is the product's identity in Windows |
| Signing | `SIGNING.md`, `SIGNING_CERT_BASE64`, `%RUNNER_TEMP%\selora-signing.pfx` | New certificate issued to the new entity; new secret names; start the OV/EV application early | New brand + old cert looks like impersonation; OV issuance takes days–weeks |
| Version | `0.1.0` | **1.0.0** | Semver compare must never prefer an old 0.x |
| Old repo | Public, still reachable by old installs | Archive it, or accept that you cannot control installed 0.x builds — but never ship a build that still points there | §0 |

Also: `frontend/package.json` `repository.url`, `deny.toml` comments, `.github/workflows/release.yml`
artifact names and step names, `start.bat`, `launch-selora.bat`, `installer/find-iscc.ps1`
(`SELORA_ISCC`), `read-version.ps1` if it names the product.

---

## 6. Legal, assets, and the brand-derived palette

1. **MIT licence.** Keep the original copyright notice in `LICENSE`, add your own line. Renaming
   does not erase authorship, and removing the notice would breach the licence you inherited.
2. **Attribution note.** Add one line to `README.md`: "This project began as Selora IDE; the
   original copyright notice is retained in LICENSE."
3. **Old-owner strings.** Remove `Toxic` from `AppPublisher`, old repo links, old docs URL, and any
   "official" claims about `api.selora.lol`.
4. **Brand-derived colour tokens.** `tokens.css` §1.8 is literally titled "Selora Monochrome Space
   System (Website parity — `E:\Projects\Selora`)" with `--space-*` and `--moonlight-*` ramps. That
   is the old company's palette: replace it rather than rename it. Bonus: this removes half of the
   two-lighting-model problem described in `SELORA_CRAFT_SPEC.md` §1.
5. **Fonts.** Space Grotesk and JetBrains Mono are OFL — ship their licence files alongside the
   bundled fonts.
6. **Unusual dependencies.** `img-fx` and `metal-fx` are not common; verify their licences and
   provenance before shipping them under a new brand.
7. **Icons.** New mark at 16/24/32/48/64/128/256 px, plus `assets/icons/wizard_large.bmp` and
   `wizard_small.bmp` for the installer, and the splash/onboarding hero (`BrandFx`, `BrandMark`,
   `SeloraMark`). One accent, no gradient (the constitution in `tokens.css` already says so).
8. **History.** Do not rewrite git history. Add the CHANGELOG entry "1.0.0 — renamed from Selora
   IDE" so existing users recognise the app and old links make sense.

---

## 7. Work order (six phases, each with a gate)

### P0 — Safety, ship under the old name (1 day)
Repoint/disable the updater, remove the hardcoded old-repo URLs, rename the update asset matcher,
remove `api.selora.lol` from defaults (temporary: gateway off), start the certificate process.
**Gate:** a built binary contains no `api.selora.lol` and no old repo slug; with a fixture where the
old repo publishes `9.9.9`, the new build does **not** offer it.

### P1 — Name (0.5 day)
Run §1.3 for the shortlist, record the decision in `docs/brand/NAME_DECISION.md`, freeze `<NAME>`,
`newname`, publisher name, domain, and the new reverse-DNS identifier.
**Gate:** signed-off table with date, trademark/category/domain results, and a chosen identifier.

### P2 — Mechanical internal rename (2–3 days, one commit)
No behaviour change. Order matters for compilation:
1. `git mv crates/selora-* crates/<newname>-*`; update `Cargo.toml` members + `[workspace.dependencies]`;
2. `sed` module paths (`selora_core::` → `<newname>_core::`, etc.);
3. rename public types (`SeloraError`, `SeloraStatus`, `SeloraClient`) — do this in one pass with
   the compiler as the test;
4. binary/target rename (`selora-desktop` → `<newname>-desktop`) and every `-p` reference;
5. frontend identifiers: package name, imports, `SeloraMark` → `<Name>Mark`;
   **do not** rename the persisted `localStorage` keys or the five event channels here — those are
   data contracts with migrations and belong to P3 (§3.6, §3.7);
6. regenerate `Cargo.lock`.
**Gate:** `cargo check --workspace`, `cargo test --workspace`, `npm run typecheck`, `npm test`,
`npm run build` all green; `git diff --stat` shows no logic changes.

### P3 — Identity + migration (2–3 days)
`tauri.conf.json` (`productName`, `identifier`, window title), installer display strings,
exe name, settings-dir copy **and its `selora` JSON key alias (§3.3 item 5)**, keyring migration,
env-var aliases, the 11 `localStorage` key migrations (§3.6), the five event channels (§3.7), and
the theme-id map (`selora-dark` → new id, old value still accepted on read).
**Gate:** the upgrade test from 0.1.0 on a clean Windows VM passes: settings kept, key kept, one
uninstall entry, no duplicate folder, app opens.

### P4 — Copy, assets, release (1–2 days)
All user-visible strings, docs, `README`, `SIGNING.md`, CI artifact names, new icons/splash, new
palette, `docs/brand/` set.
**Gate:** `grep -ri "selora"` returns only allowlisted hits (LICENSE attribution + CHANGELOG
history); installer builds and signs; About dialog shows the new name and repo.

### P5 — Publish and close (1 day)
Release 1.0.0, archive the old repo, add the CHANGELOG "renamed from" entry, delete `.zcode/`
scratch and the stray `nul` file, and record the migration notes.
**Gate:** fresh-VM install, upgrade install, and the §8 matrix all pass.

---

## 8. Test matrix (run every one; these are the ones that cost users)

| # | Test | Pass condition |
|---|---|---|
| 1 | Fresh install on a clean Windows 11 VM | App runs, taskbar/title show `<NAME>`, no "Selora" anywhere in UI |
| 2 | Upgrade 0.1.0 → 1.0.0 | Settings retained, provider still connected, **one** uninstall entry, no `Selora IDE` folder left behind |
| 3 | Credential migration | New service entry exists, old entry deleted, no plaintext `.api_key` survives |
| 4 | Settings migration | New dir populated, `migrated_from` marker present, old dir untouched |
| 5 | Updater isolation | Old repo publishing `9.9.9` produces **no** update offer; new repo `1.0.1` does |
| 6 | Binary strings | `strings`/`Select-String` on the exe finds no old brand beyond the LICENSE/CHANGELOG allowlist |
| 7 | Repo strings | `grep -ri selora` = allowlist only |
| 8 | Gateway absence | Connect modal has no Selora preset, no `sk-selora`/`sk-gw` hints, no `api.selora.lol` default; Ollama works offline; no provider selected on first run |
| 9 | Uninstall | Removes app + install dir; documented behaviour for `%APPDATA%` and credential entries |
| 10 | Signing | Signature is `Valid` and issued to the new publisher; unsigned build fails the release gate |

---

## 9. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| **Old repo still reachable by old builds** | Silent update from a namespace you don't control | §0: repoint before shipping; never ship a build carrying the old slug; archive if you can |
| Identifier change orphans webview state | Window geometry/cache loss | Copy the Tauri data dir; test on a VM |
| Installer GUID/folder mismatch | Two installs, duplicate entries, orphan files | Keep AppId + folder for 1.0; move folder in 1.1 with `[InstallDelete]` |
| Keyring service change | Users appear disconnected | Migration with read-old→write-new→delete-old, plus an explicit prompt on failure |
| Settings-key rename without a serde alias | **Every setting silently resets** — reported as data loss, not as a migration | Alias on read for one release; upgrade test asserts values survive |
| `localStorage` keys or event names renamed piecemeal | Pinned sessions, open files and all live updates vanish with no error | §3.6 table + §3.7 single commit; tests assert both |
| Env var rename breaks CI | Release pipeline fails on rename day | Keep reading old names for one release |
| Certificate lead time | Can't sign 1.0 on time | Start issuance in P0 |
| Trademark collision after launch | Forced second rename | §1.3 before any asset work; it costs 30 minutes |
| Half-renamed network identity | "Update available" with no asset, or wrong docs link | One source of truth for the repo URL; delete hardcoded copies; fixture test |
| Palette still carries the old brand | Visible continuity with the company you left | Replace §1.8 tokens, don't rename them |

---

## 10. Deliberately excluded

- **Git history.** Not rewritten. Attribution and `git blame` matter more than a clean log.
- **The five research documents** (`SELORA_MASTER_DOSSIER.md`, `SELORA_UX_PAINPOINTS.md`,
  `SELORA_CRAFT_SPEC.md`, `ZCODE_UX_TEARDOWN.md`, `ZCODE_PARITY_PROMPTS.md`, `REBRAND_PLAN.md`).
  They are dated internal artefacts that quote the old name in evidence; renaming them would make
  the citations wrong. Move them to `docs/research/` if you want them out of the root.
- **`.zcode/` scratch.** Delete, don't rename — CDP probes and PowerShell click scripts are not
  product code.
- **The old repo's existing releases.** They cannot be renamed; they can only be archived or left.

---

## 11. Decisions I need from you

1. ~~Name sign-off~~ — **decided: `Skarn`**. Still outstanding: the §1.3 clearance run (trademark,
   domain, packages) before any asset work, and confirmation that you want `Skarn` rather than the
   `-ify` form.
2. **Publisher entity** for `AppPublisher` and the certificate (your name or a company).
3. **Do you still control `github.com/ToxicAngelX/Selora-IDE`?** If yes, archive it after 1.0.0 and
   stop publishing; if no, ship the §0 fix immediately, because you cannot stop old installs from
   following it.
4. **In-place upgrade or side-by-side?** Recommended: in-place (keep the AppId), folder move in 1.1.
5. **Domain + docs host** (needed for the About dialog and the updater allowlist).
6. **Version line**: 1.0.0 for the rebrand (recommended) vs continuing 0.x.

---

## 12. Prompt (copy-paste)

```text
/plan

Role: release engineer + brand owner's technical lead. Planning only — no code, no diffs.

Objective: execute REBRAND_PLAN.md. Rename the product away from "Selora" to <NAME>, remove the
first-party gateway, and preserve every existing user's settings and credentials.

Read first, in this order:
1. REBRAND_PLAN.md (whole file — it contains the measured surface and the work order)
2. apps/selora-desktop/src-tauri/tauri.conf.json
3. installer/selora-ide.iss (Setup section) and installer/prebuild.bat (Selora-IDE-Setup naming)
4. crates/selora-settings/src/{settings.rs,keyring.rs} (SETTINGS_DIR_ENV, CONFIG_DIR, SERVICE,
   and the `pub selora: SeloraSettings` field that names the JSON key)
5. apps/selora-desktop/frontend/src/bridge/bridge.ts (the five event channel literals)
6. crates/selora-ai/src/updates.rs (asset matcher + REPOSITORY_URL + the SHA-256 trust model)
7. apps/selora-desktop/src-tauri/src/state.rs around the hardcoded release URLs
8. apps/selora-desktop/frontend/src/features/auth/{ConnectModal.tsx,provider-visibility.ts}
9. apps/selora-desktop/frontend/src/styles/tokens.css section 1.8
10. the eleven `localStorage` key literals (`features/**/*-store.ts`, `editor-persistence.ts`,
    `toast-store.ts`, `draft-store.ts`, `CommandPalette.tsx`)

Produce:
A. A rename manifest: every path, every identifier category, the exact old→new token, and the
   phase (P0–P5) that owns it. No "search and replace everything" steps.
B. The migration design for the five breaking identities (identifier, installer AppId, settings
   dir, keyring service, env vars), each with: read-old, write-new, delete-old, failure UX, and
   the test that proves it.
C. The gateway removal checklist with file:line for every removal and every string replacement,
   plus the first-run replacement state.
D. The supply-chain fix: one source of truth for the repo slug, asset-name matcher change, and
   the fixture that proves an old-repo release is ignored.
E. A per-phase gate list with the exact commands, in dependency order.
F. The top five ways this migration loses user data, and the check that catches each.
G. Anything in REBRAND_PLAN.md you believe is wrong, with the file:line that contradicts it.

Constraints: no git history rewrite; no new dependencies; keep the MIT notice; do not rename the
internal research docs; every removed string gets one replacement or none; never ship a build that
still points at the old repository.

Stop condition: if any step requires deciding the final name or the publisher entity, stop and list
the decision instead of inventing one.
```

---

*Evidence: measured `grep` census over the working tree (358 files), plus direct reads of
`tauri.conf.json`, `installer/selora-ide.iss`, `installer/prebuild.bat`, `SIGNING.md`,
`crates/selora-settings/src/keyring.rs`, `crates/selora-settings/src/settings.rs`,
`crates/selora-ai/src/updates.rs`, `ConnectModal.tsx`, and the theme id references. Name collisions
were checked live (Cairn, Burin, Umber, Mullion, Kerf, Chamfer). The app was not run; the
upgrade/migration behaviour described here is a design to be tested, not an observed result.*
