# Skarn — Antigravity Workspace Setup (Solo Gemini 4 Hydrogen)

Ready-to-drop `.agents/` configuration for executing `SKARN_MASTER_BLUEPRINT.md` using **Gemini 4 Hydrogen** solo in Google Antigravity (`agy` CLI, Antigravity App, or Antigravity IDE).

## Directory Layout

```text
.agents/
├── README.md
├── hooks.json
├── hooks/
│   ├── pre-tool-guard.mjs          # Deterministic PreToolUse safety & policy gate
│   ├── post-tool-audit.mjs         # Fast PostToolUse feedback on edited files
│   ├── stop-verification-gate.mjs  # Quota-safe Stop gate (cargo check / typecheck / legacy scan)
│   └── setup-abs-paths.mjs         # Rewrites hooks.json commands to absolute paths (recommended on Windows)
└── skills/
    ├── skarn-migration-guard/      # Stage 2: 5 breaking identities, serde alias, 11 localStorage keys, 5 IPC channels
    ├── skarn-craft-constitution/   # Stage 3: tokens.css repair, 0/2/4 radii, 0 transition:all, live-or-absent status bar
    ├── skarn-decision-invariants/  # Stages 4-5: Decision FSM, TOCTOU snapshots, adversarial fuzzing, provider adapters
    └── skarn-supply-chain-gate/    # Stages 0-1: P0 updater isolation, gateway removal, repo URL single source of truth
```

## Quick Setup (2 Minutes)

1. Copy the `.agents/` folder to your repository root (next to `Cargo.toml` and `SKARN_MASTER_BLUEPRINT.md`).
2. **Pin absolute paths in `hooks.json` (important on Windows / subfolder launches):**
   ```bash
   node .agents/hooks/setup-abs-paths.mjs
   ```
3. Reload skills and verify hooks inside Antigravity CLI:
   ```text
   /skills reload
   /hooks
   ```
   > **Windows IDE / App note:** If you use the Antigravity desktop IDE rather than `agy` CLI, fully quit and relaunch the IDE after placing `.agents/hooks.json`. Verify hooks fire by running a harmless read command or checking `/hooks` in `agy` CLI. On builds where IDE workspace hooks do not fire, run your execution turns via `agy --model gemini-4-hydrogen --effort high` in the integrated terminal, or copy the generated `hooks.json` entries into `%USERPROFILE%\.gemini\config\hooks.json`.
4. Install the official Gemini API plugin before Stage 5 so Hydrogen reads live Google AI Studio / Decisions API docs:
   ```bash
   npx skills add google-gemini/gemini-skills --skill gemini-api-dev --global
   ```

## Solo-Hydrogen Execution Cadence

| Stage | Mode | Effort | Pattern |
|---|---|---|---|
| Phase 0 (Audit) | `plan` | `xhigh` | Single read-only session; write findings to a markdown brief and stop. |
| Stage 0 (Updater P0 + Gateway Kill) | `accept-edits` | `high` | Small surface; let Rust tests + `check-legacy-refs.mjs` verify. |
| Stage 1 (358-File Mechanical Rename) | `auto` | `medium` | Save quota; compiler + typecheck catch every missed symbol. |
| Stage 2 (5 Breaking Identities) | `plan` -> `accept-edits` | `xhigh` -> `high` | Two-pass split; verify all 5 IPC channels in one commit. |
| Stage 3 (Shell Subtraction + Craft) | `accept-edits` | `high` | One task card per fresh session; run `css-audit.mjs` + `/browser`. |
| Stage 4 (Decision FSM + TOCTOU) | `plan` -> `accept-edits` | `xhigh` -> `high` | Use `xhigh` only for planning or debugging failing `proptest`/`fast-check` races. |
| Stage 5 (Providers + Settings UI) | `accept-edits` | `high` | One adapter per fresh session (Mock/Laya -> Jev -> Gemini -> UI). |

## Testing the Scripts Manually

Every skill script can be run directly without burning any Hydrogen tokens:

```bash
node .agents/skills/skarn-supply-chain-gate/scripts/check-legacy-refs.mjs
node .agents/skills/skarn-supply-chain-gate/scripts/check-legacy-refs.mjs --strict
node .agents/skills/skarn-migration-guard/scripts/check-ipc-parity.mjs
node .agents/skills/skarn-craft-constitution/scripts/css-audit.mjs
node .agents/skills/skarn-decision-invariants/scripts/check-decision-hygiene.mjs
```

## Disabling a Hook Temporarily

Set `"enabled": false` on any named hook inside `.agents/hooks.json` (for example, to silence `skarn-stop-gate` during a mid-refactor scratch commit). Note that `pre-tool-guard.mjs` intentionally blocks the agent itself from modifying `.agents/hooks.json` or `.agents/hooks/*`—only you can edit them manually.
