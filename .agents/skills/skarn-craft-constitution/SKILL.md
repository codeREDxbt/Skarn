---
name: skarn-craft-constitution
description: Enforce Skarn's visual craft constitution, region ownership routing rules, deletion-first shell cleanup, and CSS token discipline whenever editing CSS, tokens.css, layout shell TSX, status bar, or agent UI components.
---

# Skarn Craft Constitution

Load `references/craft-rules.md` before modifying CSS, `tokens.css`, or shell layout components.

## Non-Negotiable Rules

1. **Deletion before design:** Remove lies and duplicate homes first (fake Git sync, hardcoded `Ln 1, Col 1`, `UTF-8`, `TypeScript` fallback, `core <hash>`, "Not wired yet" Output tab, developer phase-name errors, non-savable settings controls).
2. **Region owns content kind:**
   - Left rail + sidebar = Project (`files`, `search`, `git`). A rail icon NEVER opens the bottom dock.
   - Bottom dock = Process (`terminal`, `problems`, `runs`).
   - Right dock = Task (agent).
   - Settings = Dedicated page/modal (never a 300px sidebar route).
3. **One vocabulary:** Permission modes are strictly `Read-only`, `Ask before changes`, `Auto-edit`, `Full access`. Never preserve old synonyms (`Full auto-apply`, `Armed`, `Guarded`, `Gateway`).
4. **Tokens & CSS discipline (single owner for `tokens.css`):**
   - Keep sharp `0 / 2 / 4` px radius identity (max 4 radius tokens, zero raw px bypasses).
   - **Zero** `transition: all` — animate explicit properties only.
   - **Zero** new gradients, glass (`backdrop-filter`), `--glow-*`, or `*-polish.css` files.
   - Replace `tokens.css` §1.8 brand-derived `--space-*` / `--moonlight-*` ramps cleanly in one pass.
   - Longest non-splash animation <= 300 ms; keyboard-initiated transitions = 0 ms; protect `prefers-reduced-motion`.
   - No new CSS/UI frameworks (no Tailwind, shadcn, Lucide, or Motion runtime).
5. **Nine-state completeness:** Any modified core control must define: `default`, `hover`, `active`, `focus-visible`, `selected`, `disabled`, `loading`, `empty`, `error`.

## Mandatory Verification

Run after any CSS or shell layout edit:
```bash
node .agents/skills/skarn-craft-constitution/scripts/css-audit.mjs
```
