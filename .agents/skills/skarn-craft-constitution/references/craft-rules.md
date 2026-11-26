# Skarn Craft & Shell Reference

## Status Bar Policy: Live or Absent
- Every displayed item must react to real state changes.
- Delete: fake Git sync spinner (`setTimeout(1200)`), hardcoded `Ln 1, Col 1` (unless bound to live cursor), hardcoded `UTF-8`, hardcoded `'TypeScript'` fallback, `core <hash>`, lowercase `selora connected`.

## One Home Per Concept
| Concept | Single Surviving Home | Delete Duplicates |
|---|---|---|
| Provider + API key + Decision BYOK | `Settings -> Providers` | Standalone Connect config path, Customizations providers |
| Model & Reasoning effort | Composer | Customizations model/effort |
| Project instructions | `AGENTS.md` + rules | Global-only Customizations blob (move user defaults to Settings) |
| New work | One `New task` button per window | Extra `+` / duplicate rail buttons / `"New Session"` label |
| Blocking file errors | Tab indicator + Problems + status area | 4-second auto-dismissing toast |

## Motion & Frequency Gate
- Triggered 100+/day or keyboard-initiated: **0 ms** (instant).
- State feedback / micro-transitions: **<= 150 ms**, explicit properties only (`opacity`, `transform`, `color`, `background-color`, `border-color`).
- Disclosure / drawer motion: **<= 300 ms**, no overshoot springs, no 2,400 ms composer border beam.
- Max 4 easing tokens; no `ease-in`.

## CSS Consolidation Targets
- Collapse the 3 `*-polish.css` files into their base stylesheets and delete the polish files.
- Remove duplicate class aliases (`agent-session-tabs-strip` vs `agent-sessiontabs-strip`, etc.).
- Eliminate the 20 `!important` overrides in `shell.css`.
- Target >= 20% total CSS line reduction with zero unintended visual diff.
