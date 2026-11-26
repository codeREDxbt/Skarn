---
name: skarn-supply-chain-gate
description: Enforce Stage 0/P0 updater supply-chain isolation, single-source repository URL, Selora API Gateway removal, signing hygiene, and brand-leak grep gates when touching updates.rs, state.rs, Cargo.toml, ConnectModal.tsx, CI workflows, or installer scripts.
---

# Skarn Supply-Chain & De-Brand Gate

Load `references/supply-chain-checklist.md` before modifying updater code, gateway/auth UI, CI release pipelines, or installer scripts.

## Non-Negotiable Rules

1. **P0 Updater Isolation (Security First):**
   - Remember: this app uses a custom GitHub-release client (`updates.rs`), NOT the Tauri updater plugin—there is no embedded signature pubkey, and a sibling `.sha256` from the same GitHub release proves integrity only, not authenticity.
   - Repoint or disable the updater before anything else; make `[workspace.package] repository` in root `Cargo.toml` the SINGLE source of truth (`CARGO_PKG_REPOSITORY`).
   - Delete ALL hardcoded copies of `ToxicAngelX/Selora-IDE` (`state.rs`, `App.tsx`, `AgentsWindowApp.tsx`, `package.json`, `.iss`).
   - Update the asset matcher to `Skarn-Setup-<tag>.exe` + `.sha256` in the same commit as the installer output filename.
   - Prove via unit test fixture that an old-repo release tagged `9.9.9` produces **zero** update offer.
2. **Gateway De-Brand (BYOK Only):**
   - Remove the `selora` preset, `https://api.selora.lol`, `sk-selora-` / `sk-gw-` hints, `provider-visibility.ts` `'selora'` heuristic, and `'Recommended'` badge.
   - First-run replacement: **no preselection**, 4 neutral cards (Anthropic, OpenAI, Local/Ollama, Custom endpoint), plus honest copy ("Bring your own key. Keys stay in Windows Credential Manager.").
   - Re-tag OpenRouter from `'Gateway'` to `'Aggregator'`.
3. **Legal & Hygiene:**
   - NEVER rewrite git history. Keep original MIT copyright notice in `LICENSE` + add your line.
   - Do NOT rename the archived research docs (`REBRAND_PLAN.md`, `SELORA_*.md`, `ZCODE_*.md`).
   - Delete `.zcode/` scratch and any stray `nul` file before v1.0.0 release.

## Mandatory Verification

Run after any Stage 0 / Stage 1 / Stage 5 release change:
```bash
node .agents/skills/skarn-supply-chain-gate/scripts/check-legacy-refs.mjs
# At Stage 5 / final release gate:
node .agents/skills/skarn-supply-chain-gate/scripts/check-legacy-refs.mjs --strict
```
