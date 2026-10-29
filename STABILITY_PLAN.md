# Selora IDE — Stability Plan

Status: active. Started 2026-10-29.
Scope: stability only. No new features, no style refactors.

This plan replaces the draft supplied from a README-only review. Paths, crate
names and checks below were verified against the real repository.

## Verified architecture (do not re-derive)

- Windows desktop app: Tauri 2 shell at `apps/selora-desktop/src-tauri`, React
  frontend at `apps/selora-desktop/frontend`, WebView2 runtime.
- Editor: CodeMirror 6 in `frontend/src/features/editor/CodeMirrorPane.tsx`.
  Buffer text and edit application live in Rust:
  `crates/selora-core/src/buffer.rs` and `apps/selora-desktop/src-tauri/src/workspace.rs`.
- Commands cross a typed bridge: `crates/selora-protocol`, Tauri commands in
  `apps/selora-desktop/src-tauri/src/commands.rs`, dispatch in `state.rs`.
- Workspace crates: `selora-core`, `selora-fs`, `selora-git`, `selora-lsp`,
  `selora-ai`, `selora-editor`, `selora-settings`, `selora-tokens`,
  `selora-protocol`, `selora-workflow`, `selora-ui`, `selora-app`.
- Terminal: `src-tauri/src/terminal.rs` (portable-pty/ConPTY), UI in
  `frontend/src/features/dock/TerminalPanel.tsx`.
- AI: `crates/selora-ai` client + agent loop/subagents; desktop orchestration in
  `src-tauri/src/sessions.rs`; UI in `frontend/src/features/agent`.
- Workflows: `crates/selora-workflow`, `src-tauri/src/workflows_*`.
- LSP: `crates/selora-lsp` actor/manager; desktop bridge
  `src-tauri/src/lsp_bridge.rs`.
- Settings: `crates/selora-settings/src/settings.rs`; UI
  `frontend/src/features/sidebar/SettingsPanelView.tsx`.
- CI: `.github/workflows/ci.yml` already runs frontend typecheck/test/build,
  `cargo fmt --check`, `clippy -D warnings`, `cargo build`, `cargo test`,
  `cargo deny check` on Windows. Release: `.github/workflows/release.yml`.
- Installer: `installer/selora-ide.iss` + `installer/prebuild.bat`, output
  `Selora-IDE-Setup-<version>.exe` with a SHA-256 sidecar.

## Already present (do not duplicate)

- Atomic save: `workspace.rs::atomic_write` writes a same-directory temp file,
  `sync_all`, then rename.
- Panic hook writes a backtrace to `%TEMP%/selora_last_panic.log`.
- Command dispatch, bootstrap, settings and most filesystem work are offloaded
  with `spawn_blocking` (`commands.rs`).
- Workspace search supports ripgrep and a builtin fallback with bounded reads.
- External-change refresh exists: clean buffers reload, dirty buffers raise a
  conflict (`workspace.rs::refresh`).
- CI already enforces fmt, clippy, tests, deny and frontend build.

## Real gaps and priorities

### P1 — Data safety (owner: data-safety)
- No hot-exit store: unsaved buffers are not continuously persisted, so a
  process kill or power cut loses edits. Add a workspace-scoped, atomic,
  size-capped snapshot of dirty buffers plus restore-on-boot that never
  overwrites newer disk content.
- Save path is atomic but has no explicit validation that the rename preserved
  line endings/encoding; verify CRLF/LF and UTF-8 round-trip.
- Snapshot cleanup: stale/mismatched-workspace snapshots must be discarded.

### P2 — Crash handling (owner: crash) — queued
- Panic log is a single non-rotating temp file; move to app-data logs with
  rotation and add an "Open Logs" command.
- Audit `unwrap`/`expect`/`panic!` and unchecked indexing in non-test code on
  request paths; prefer typed errors.
- Optional: attempt dirty-buffer snapshot in the panic hook.

### P3 — Responsiveness (owner: responsiveness)
- Confirm no blocking I/O on the UI thread for open/save/search/tree.
- Add cancellation and caps for long searches and large file opens; define
  behaviour for minified/huge files.
- Debounce repeated frontend events where needed.

### P4 — Language servers (owner: lsp)
- Restart with exponential backoff and a retry cap; surface status; ensure a
  dead server never blocks typing.

### P5 — Terminal and Git (owner: terminal-git)
- Timeouts and cancellation for PTY spawn and git network operations; clean
  shutdown with no orphan children; clear typed errors.

### P6 — AI (owner: ai) — queued while credential work finishes
- Request timeouts, visible cancel, retry/backoff, typed bad-key/network
  errors, chat history preserved on failure, AI failure isolated from editor.

### P7 — Tests (owner: tests) — queued until feature work lands
- Inline unit tests only (`#[cfg(test)]`). The public tree intentionally omits
  top-level `tests/` directories and screenshots, so do not recreate them.
- Cover buffer edit/undo, atomic save, hot-exit restore, conflict handling.

### P8 — CI and release (owner: ci)
- Keep the existing gates green. Add a lightweight startup/large-file smoke
  check that fails on regression. Document unsigned-installer and updater
  signing requirements; never invent keys.

## Decisions

- Public repo stays lean: no docs dumps, screenshots, logs or top-level test
  trees. Stability tests live inline.
- Release publishing is on hold until stability work and verification finish.
- Where two areas need the same file, the lead serialises the edits.

## Open risks

- Unsigned installer: in-app updates now require a valid Authenticode
  signature, so automatic update cannot install an unsigned build.
- Secret redaction depends on keys observed at runtime; keys rotated while the
  app is closed cannot be retroactively scrubbed.
- History still contains old planning docs and screenshots until the planned
  rewrite runs.

## Progress log

- 2026-10-29: plan written from verified source audit. Data safety, LSP,
  terminal/git, responsiveness and CI agents dispatched.
- 2026-10-29 (responsiveness): search bounded (10k hits, 8 MiB preview text,
  4k-char hit lines, 10s deadline, 512 MiB / 64 MiB-per-file scans) with
  `SearchCancel`, drop-kills the `rg` child, newest interactive search
  supersedes the old one per root; tree walk capped at 50k nodes / depth 64 /
  5 s; `selora-editor` skips tree-sitter over 2 MiB or 16k-char lines;
  CodeMirror opens >2 MiB or >10k-char-line files read-only with plain text
  and no lint/hover/completion. 64 selora-fs + 78 selora-editor + 189 frontend
  tests pass. No sync FS on the Tauri IPC thread; desktop build currently blocked by
  another owner's in-flight `crate::hot_exit` import in state.rs.
- 2026-10-29 (terminal/git): git fetch/pull/push are bounded by
  `NETWORK_TIMEOUT` (180 s) through a polled wait; an overrun kills the run
  (including the transport helper via `taskkill /T`) and returns the new typed
  `GitError::Timeout` (`network_timeout`). Each output stream is captured up to
  64 KiB while still being fully drained, and the readers are detached with a
  bounded drain, so a chatty remote or a surviving helper cannot deadlock or
  hang the request. File diffs are capped (`MAX_DIFF_LINES`,
  `MAX_DIFF_BYTES`, `DiffOptions::max_size`) and a cut shows a truncation
  notice; `MERGE_HEAD` reads are bounded. Terminal close now waits a bounded
  2 s for the reaped tree (exited flag set by the waiter) and the session cap
  is re-checked under the same lock as the insert. `cargo test -p selora-git
  --locked` passes (45 tests). `cargo test -p selora-desktop terminal
  --locked` cannot run yet: the crate fails to compile on an unrelated
  in-flight edit at `explorer.rs:347`; the 16 terminal tests pass in an
  isolated harness that includes the real `terminal.rs` and
  `workflows_process.rs`. Not fixed (needs `state.rs`/`workspace.rs`): the
  fetch/pull/push body runs inside `with_workspace`, so it holds the workspace
  mutex for up to the timeout and other workspace commands queue behind it;
  the protocol has no cancel command for git.
- 2026-10-29: hot-exit module added (`src-tauri/src/hot_exit.rs`): atomic
  workspace-scoped dirty-buffer snapshots (16 MiB total, 4 MiB/buffer), restore
  filtered by root and existing disk file, digest-based disk-change check,
  cleanup and stale pruning. 14 inline tests pass; not wired into `main.rs`.
- 2026-10-29 (LSP): manager now restarts dead servers with exponential backoff
  (500 ms doubling, 30 s cap, 5 attempts), resets the budget after 60 s of
  uptime, refuses to spawn past the cap, and surfaces the capped failure as a
  typed `Failed` state; hover/definition/completion run in their own tasks with
  a 10 s deadline, so a hung server never stalls document sync. Inline tests
  added; `cargo test -p selora-lsp --locked` green (63 tests).
- 2026-10-29 (CI/release): `ci.yml` gained a `stability-smoke` job (deterministic
  large-file open guard via the existing `binary_and_oversized_files_are_rejected`
  test, plus an opt-in cold-start probe gated by `ENABLE_STARTUP_SMOKE=true`
  because hosted runners do not guarantee a desktop); `SIGNING.md` documents the
  three signing inputs; the release workflow was verified tag-gated, asset names
  match the updater, and published releases are refused. The stale `docs/mkdocs.yml`
  requirement in `scripts/check-release-metadata.ps1` was made optional for the
  docs-free public tree. CI is still red at this timestamp from other agents'
  in-flight edits: `cargo fmt` diffs in `crates/selora-git/src/{lib,merge,net}.rs`
  and `crates/selora-fs/src/search.rs`, and `selora-fs/src/search.rs` does not
  compile (`search_builtin_with`, `search_in_files_regex_cancellable`,
  `bounded_hit`, `run_rg_source` arity).
  An earlier full clippy pass also reported 9 warnings-as-errors in
  `sessions.rs` (`redact_card` unused) and `explorer.rs` (`clone_on_copy` /
  `cloned_ref_to_slice_refs`); re-run the gates after those land.
- 2026-10-29 (crash): the panic hook now writes a timestamped record (thread,
  location, message, forced backtrace) to `%APPDATA%\selora-ide\logs\selora.log`
  with 2 MiB size-based rotation keeping 5 files, a re-entrancy guard and
  `catch_unwind` so it never aborts from its own I/O; unwritable dirs are
  swallowed and the hook deliberately does not save buffers while unwinding.
  New `OpenLogs` protocol command (no workspace needed) opens that folder via
  the existing file-manager launcher, exposed as "Open logs" in the About
  dialog. `cargo check -p selora-desktop` and the frontend typecheck are green;
  the 6 inline `crash_log` tests pass in an isolated harness, but
  `cargo test -p selora-desktop crash --locked` cannot compile yet because the
  untracked `explorer.rs` test module fails borrowck (`E0505` at
  `explorer.rs:347`).
- 2026-10-29 (responsiveness, search/git mutex): the two confirmed lock-hold
  defects are fixed in `state.rs`/`workspace.rs`. A search now reads the root
  from a lock-free mirror, takes a monotonic search epoch, and calls the new
  `selora_fs::search::cancel_interactive_search` *before* any workspace lock;
  the scan runs through `workspace::search_root` with no workspace guard, a
  superseded scan maps to a silent ack (no stale `SearchResults`), and results
  publish only while the epoch and workspace id still match. Fetch/pull/push
  capture an owned `GitNetwork` handle (`Arc<Mutex<Option<GitRepo>>>` + root),
  drop the workspace guard, run the bounded network call, then re-lock only to
  check the workspace id and publish `GitStatusChanged`; the inner git mutex
  still serialises every git operation on the same repository handle, and
  `git_error` maps `GitError::Timeout` to the new `network_timeout` code.
  `selora-fs::SearchGuard` now owns its root slot (`start_interactive_search`,
  `cancel_interactive_search`, `SearchCancelGuard` for bare tokens); 65
  selora-fs tests pass, `cargo test -p selora-desktop state::tests --locked`
  passes (97, 2 ignored) with new tests proving the cancel fires while the
  workspace mutex is held and a parked fake git op does not block a tree read,
  `workspace::tests` passes (35) with a `network_timeout` mapping test, and
  `cargo fmt --all` is clean. Residual risk: a workspace switch during a scan
  still lets the old scan run to its 10 s deadline (results are dropped, no
  lock held); overlapping searches can reorder at the fs-slot registration by
  microseconds; and a local git command issued during a fetch/pull/push still
  waits on the shared per-repository git mutex while holding the workspace
  mutex, so that one command can queue save/tree/search until the network op
  ends (the network op itself no longer holds the workspace lock).
  `cargo clippy -D warnings` still fails only on the pre-existing
  `sessions.rs::redact_card` dead code owned by another agent.
- 2026-10-29 (data safety / hot-exit wiring): `hot_exit` is now live. Edits and
  saves capture immediately and a 2 s `selora-hot-exit-timer` thread is the
  safety net; `HotExitWriter` coalesces submissions to the latest snapshot on
  one background thread, skips unchanged fingerprints and ignores write
  errors. Restore runs on boot and on `OpenFolder`/`OpenFolderAt`: a buffer is
  installed only when the file exists and its content digest still matches the
  recorded saved baseline (rechecked against the text actually read), it is
  marked dirty without touching disk, and recovered buffers are published as
  `BufferSnapshot` before the bootstrap; changed or missing files raise a
  warning toast and the stale snapshot is removed. Close (after save/discard),
  `CloseFolder` and root switches snapshot remaining dirty buffers or clear
  the snapshot when everything is clean; snapshots for other roots are pruned
  after 7 days. `cargo test -p selora-desktop hot_exit --locked` passes (19
  tests, including 4 new state-level restore/discard/save-clear/write-failure
  cases) and `cargo test -p selora-desktop state::tests --locked` passes (95
  tests, 2 ignored); `cargo fmt --all` is clean. Remaining risk: snapshot
  gathering still clones dirty text and stats dirty files on the command
  thread (the write itself never blocks), the close quiesce is bounded at 5 s,
  and the digest is a non-cryptographic content fingerprint.
- 2026-10-29 (AI): gateway calls are now bounded (15 s connect, 90 s non-streaming/header deadline, 2 min SSE read gap, 5 s ping) and timeouts surface as `SeloraError::Net` with the typed `timeout:` marker plus `GatewayFailure` classification; the retry policy is capped exponential with 25% jitter (2 transport retries, 3 retries for 408/429/5xx, 8 s ceiling, whole exchange deadline-bounded) and never retries 400/401/403, so one logical request (never a duplicated user turn) is retried. Desktop maps the classes to `timeout`/`auth_failed`/`network_error`/`gateway_unavailable`/`gateway_error` with actionable copy and redaction, and request-path mutex locks recover from poisoning instead of panicking. Stop is now polled before the delta channel in `stream_round_gated` so a chatty stream cannot starve it; the agent loop keeps the partial text, ends `StoppedByUser` with the user turn intact, and the frontend marks the row `cancelled: true` (partial text kept) instead of error/completion. Tests: `cargo test -p selora-ai --locked` 238 pass (new timeout/backoff/403/partial-stop/stream-failure cases), `cargo test -p selora-desktop sessions --locked` 55 pass (new typed-error mapping), frontend agent vitest 28 pass (new `agent-store.cancel.test.ts`) and typecheck green; clippy `-D warnings` green for selora-ai and selora-desktop, with the previously unused `redact_card` now `#[cfg(test)]`. Not fixed: the visible cancelled badge needs `AgentExecutionFeed.tsx` (outside this slice), so the flag is store-level only; a mid-stream read timeout still ends the turn without a receipt.

## Completion status (lead integration)

Done and verified locally:

- Full gates on the recovery tree: `cargo fmt --check`, `cargo clippy
  --workspace --all-targets -D warnings`, `cargo test --workspace`,
  `cargo deny check`, frontend `typecheck`, `test` (189 + later suite), and
  `build` all pass. No failing test remains.
- Workflow subagents now derive `workspace_trusted` from settings instead of
  defaulting to false; the two workflow tests that regressed under the trust
  gate pass again.
- Hot exit, crash logs, LSP backoff/timeouts, bounded search/tree/highlight,
  terminal/Git teardown limits, AI timeouts/retries/cancel, and lock-free
  search/Git networking are all wired into the live app.
- Public tree stays lean: no screenshots, internal docs, logs, dumps, or
  `.env`; tests are colocated with source (CI needs them).

Decisions:

- Release publishing is intentionally on hold. The installer is built and
  verified locally, but nothing was pushed or attached to GitHub.
- Git history cleanup (removing old screenshots/planning docs from reachable
  history) is prepared but not executed, because it rewrites public refs.
- Unsigned installers remain rejected by the in-app updater until a signing
  certificate is provided (see SIGNING.md).

Remaining risks (documented, not silently ignored):

- Hot-exit digest is non-cryptographic; a collision could restore stale text
  over changed disk content (the UI still shows it as dirty, never silent).
- Secret redaction cannot know keys that were rotated while the app was off.
- Large dirty sets are copied on the command thread when snapshotting.
- The legacy egui shell (`selora-app`) still uses the non-cancellable search
  entry point; the shipped Tauri shell does not.
