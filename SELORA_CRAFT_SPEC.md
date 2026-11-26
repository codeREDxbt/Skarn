# Selora Craft Spec — turning references into rules

> The layer the previous two documents didn't cover: not *what* to build or *where* it goes,
> but *how it should look and move* so it stops reading as slop.
>
> Companion: `SELORA_UX_PAINPOINTS.md` (structure, routing, duplication). This one is craft.
> No implementation code.

---

## 0. How these references were used (and two corrections)

**Correction first, because it matters to the budget.** Two of the five "free skills" are paid
products, not skills:

- **ThinkUI** (`thinkui.xyz`) is a hosted **MCP server** — 1,483 screenshots, 3,802 components,
  from **$7.99/mo**. It is a reference library an agent queries, not a method you install.
- **Uizze** (`uizze.com`) is the same shape: 800k screens, 35k elements, **$9/mo**.
- **Anti UI Slop** is the *marketing claim* on Uizze's landing page ("UI taste for agents"), not
  a separate skill with published rules.

The genuinely free, genuinely useful ones are **Vercel's Web Interface Guidelines** (MIT, and it
ships an `AGENTS.md` you can drop straight into a repo) and **Emil Kowalski's skills** (public
SKILL.md files with hard numbers). Those two are methods. The rest are catalogues — useful to
look at, dangerous to depend on, because a reference library solves *"I have no taste of my
own"*, and our problem is the opposite: we have a stated design constitution and we violate it.

**The screenshots are the most valuable thing in the message.** They show a dark desktop
product with a 56 px icon rail, a two-line row anatomy, metric cards with a segmented history
strip, and a log pane with strict columns. That is the exact density and calm Selora is missing.
I read them for **information design**, not for branding.

**Not copied from them, deliberately:** the 10–14 px card radii (Selora's 0/2/4 px sharp
system is a legitimate identity — copying softness would be trend-chasing), the decorative
graph-paper backdrop, and putting a destructive action ("Clean up 16.6 GB") as a peer row in
navigation. Taste is also what you refuse.

---

## 1. The contradiction at the centre of the design system

`tokens.css` opens with a constitution:

> "- no gradients on chrome, no box-shadows, no background images;
>  - saturated color only via `--ok` / `--warn` / `--err` / `--info` …"

and then, 90 lines later, defines the things it forbids:

- `--glass-bg`, `--glass-bg-subtle`, `--glass-bg-elevated`, `--glass-border`, `--glass-border-hover`
- `--blur-surface`, `--blur-popover`, `--blur-scrim`
- `--glow-green`, `--glow-green-subtle`, `--glow-accent`, `--glow-accent-strong`, `--glow-warn`, `--glow-err`, `--glow-info`
- `--grad-radial-subtle`, `--grad-radial-accent`, `--grad-beam`, `--grad-shimmer`

**And the code obeys the tokens, not the constitution.** Census across 28 CSS files:

| Effect the constitution bans | Files using it | Token references |
|---|---|---|
| `gradient()` | **12** | `--grad-*` used across chrome and agent surfaces |
| `backdrop-filter` (glass) | **8** | `--glass-*` referenced **28×** |
| `box-shadow` | **16** | — |
| `--glow-*` | — | **80+ references** (23 `--glow-accent`, 13 `--glow-green-subtle`, 11 `--glow-green`, …) |

This is *the* mechanical explanation for "sloppy". The product is not one design language; it
is two, interleaved file by file. Panels built to the constitution are flat, hairline, mono.
Panels built from the glass/glow set are blurred, glowing and gradient-lit. Adjacent surfaces
therefore disagree about how the app is lit — and users read that as unfinished without being
able to name it.

**Radius census makes the same point numerically:**

| Radius | Count | Note |
|---|---|---|
| `var(--radius-1)` | 98 | canonical (2 px) |
| raw `4px` | 41 | bypasses tokens |
| `var(--radius-pill)` | 38 | canonical |
| raw `6px` | 29 | **not in the scale at all** |
| `50%` | 28 | circles |
| `var(--radius-2)` | 26 | canonical (4 px) |
| raw `3px` | 23 | bypasses tokens |
| raw `8px` | 17 | not in the scale |
| `var(--radius-2, 6px)` | 14 | **fallback disagrees with the token it falls back from** |
| raw `2px` | 14 | bypasses tokens |
| raw `5px`, `7px`, `12px` | 10 / 7 / 5 | not in the scale |
| `var(--agents-radius-*)` | 22 | **a second, parallel radius system** with its own sm/md/pill |

Two radius families, ~10 raw values, and a fallback that contradicts its own token. Same story
for easing: **six** curves (`precise`, `calm`, `orbital`, `fade`, `spring`, `smooth`) for what is
really four decisions.

**Verdict:** the constitution is right and the tokens betray it. Fix by choosing — either
legalise glass/glow in writing for named surfaces (composer, overlay) or delete the tokens.
Keeping both is how this happened.

---

## 2. What the reference screenshots actually teach (12 rules)

Each rule is stated as a decision, then pointed at the Selora element it fixes.

| # | Rule (visible in the screenshots) | Selora element it fixes |
|---|---|---|
| 1 | **The active rail item is a filled 44 px rounded square**, unmistakable at a glance, with a 6 px status dot for "live" | Our rail: selection depends on another region's state, so one icon can never light (P0-2) |
| 2 | **Rail icons carry numbers, not colours alone** — "3 live", "2 running" | Our pulse dots: colour-only status, no count (Vercel: redundant cues) |
| 3 | **Rows are two lines**: primary label, then a mono detail (`web` / `pnpm dev`) at lower contrast | Our task rows: title + four metadata chips of equal weight |
| 4 | **Right-aligned values in a muted column** (`:5173`, `18.6 GB`, `3 live`) — alignment does the grouping | Our session rows: branch, msg count, badges all competing |
| 5 | **Metric cards**: big number, small unit, one muted context line, a segmented strip for history | Our turn report: cost/tokens buried in a sentence |
| 6 | **Section labels are tiny, muted, uppercase-ish, with one action (`+`) on the right** | Our sidebar headers are 24 px title bars with no section grammar |
| 7 | **Mono is reserved for machine data**; labels are sans. Two families, no exceptions | We honour this (Space Grotesk + JetBrains Mono) — keep it, enforce it |
| 8 | **Colour appears only where it means something** (green running, cyan GET, checkmark) | Our 80+ glow references: colour as decoration |
| 9 | **Separation is one hairline plus whitespace**; cards do not float | Our mix of shadows, glass and borders on the same element |
| 10 | **Logs use fixed columns and tabular numerals** — time, source, message, status | Our terminal has no column discipline for agent run logs |
| 11 | **Density is high but breathing**: 32–36 px rows, 12 px gaps inside groups, 24 px between groups | Our agent cards: 8–10 px gaps everywhere, no group rhythm |
| 12 | **Nothing animates except status** — a single pulsing dot | Our composer runs a 2,400 ms perimeter beam (`--dur-lap`) |

---

## 3. Motion policy (Emil's numbers, our violations)

The rule that decides most arguments: **frequency is the gate, not taste.**

| How often the user triggers it | Decision |
|---|---|
| 100+/day (command palette, panel toggles, tab switches, keyboard actions) | **No animation. Ever.** |
| Tens/day (hover, list navigation, row selection) | Near-imperceptible or nothing |
| Occasional (modals, drawers, toasts, menus) | Standard animation |
| Rare / first-time (onboarding, success, celebration) | The delight budget lives here |

**Budgets:** press 100–140 ms · tooltips 125–180 ms (first has a ~400 ms delay, peers open with
zero delay and zero animation) · dropdown/select 150–250 ms · modal/drawer 200–300 ms · nothing
above 300 ms except the splash. Exits ~20% faster than entrances, and exit the way they entered.

**Easing roles (four, not six):** entering/exiting → strong ease-out
`cubic-bezier(0.23, 1, 0.32, 1)`; moving on screen → `cubic-bezier(0.77, 0, 0.175, 1)`;
hover/colour → `ease`; constant progress → `linear`. **Never `ease-in` on UI.** (Our token set
correctly contains no `ease-in` — the one thing it got exactly right.)

**Our violations, in order of harm:**

1. **`transition: all` × 75.** Vercel: explicitly list properties. `all` animates layout and
   is the main source of the "cheap" feel under resize/drag.
2. **The composer BorderBeam: 2,400 ms loop** on the most-used surface, on every in-flight
   turn. Fails frequency (100+/day) *and* necessity (does it communicate a state the orb
   doesn't?). **Delete it.** The thinking orb already carries liveness.
3. **`--ease-spring` with overshoot** (`0.175, 0.885, 0.32, 1.275`) used on accordions.
   Bounce belongs to drag-to-dismiss and gestures, not to disclosure panels.
4. **Six easing curves** for four roles — collapse to four.
5. **Loops on hover-adjacent surfaces** (glow pulses on cards) — tens/day → remove.

**Keep:** reduced-motion handling (already two-path and correct: OS + in-app, loops frozen,
durations preserved — this is better than most products).

---

## 4. The five "anti-slop" skills, converted into audit passes

Marketing skills are useless as dependencies and useful as *gates*. These are the five passes,
each with a pass/fail a reviewer can apply without taste.

**Pass 1 — Frequency (Emil).** Every animation/transition gets a number: times seen per day.
>`tens` → must be instant or imperceptible. Fail = any `transition: all`, any loop on a daily
surface, any animation on a keyboard-initiated action.

**Pass 2 — Provenance (ThinkUI / Uizze, inverted).** A reference library exists to stop agents
inventing UI. Apply it as a rule: every non-obvious pattern must name **the shipped product it
copies and why** — or be removed. Fail = "it looked nice" with no precedent. This is also how
you avoid the accidental *Cursor/VS Code pastiche* that makes AI-built editors look alike.

**Pass 3 — Role & contrast (Vercel).** One accent per meaning; never colour alone (pair with a
label or count); `:hover`/`:focus` increase contrast rather than decrease; visible focus ring on
every focusable element; AA minimum, APCA preferred. Fail = a green dot with no text, a focus
state that dims, an accent used for decoration.

**Pass 4 — State coverage (Anti UI Slop).** Every control must define nine states: default,
hover, active, focus-visible, selected, disabled, loading, empty, error. Fail = any control
whose loading or error state was never designed (our Settings literally ships disabled controls
with apology tooltips — that is this pass failing).

**Pass 5 — Structure (gridlines).** Every element aligns to the 4 px grid and to a named column
or baseline. Optical ±1 px adjustments are allowed but must be commented. Fail = magic numbers
(`5px`, `7px`, `3px` radii are the symptom).

Run all five on one screen per week. They are cheap, they are objective, and they find exactly
the defects that a redesign would have reintroduced.

---

## 5. Tool verdicts — 11 repos, 7 refusals

Taste is mostly refusal. For a Tauri + React + CodeMirror app with a hand-built 20k-line token
system, most of this list is a liability.

| Tool | Verdict | Reasoning |
|---|---|---|
| **Vercel Web Interface Guidelines** | **ADOPT** | Free, MIT, and ships `AGENTS.md`. Drop it in the repo so every future change inherits it. Highest leverage item in the whole list. |
| **Emil's skills** (`animate`, `review-animations`) | **ADOPT** as a review pass | Hard numbers, decision trees, frequency table. Use it to audit, not to generate. |
| **axe-core** | **ADOPT** | Runs in CI against 6 key screens. Finds the contrast/focus/label defects the passes describe. |
| **Playwright** | **ADOPT** | The only way to enforce the 8 acceptance tests from `SELORA_UX_PAINPOINTS.md` §15 ("click every rail icon and never look surprised"). |
| **ui-ux-pro-max-skill** | **ADAPT** | Useful checklist of layout/interaction heuristics; not a generator. Read it, don't wire it. |
| **Radix primitives** | **DEFER** | Our hand-rolled ARIA (focus traps, roving tabindex, inert backgrounds) is already careful. Adopt only if a specific widget's behaviour is failing tests. |
| **Storybook** | **ADAPT narrowly** | A full install is cost without benefit for two windows. A single dev-only "states gallery" route gives 80% of the value for 5% of the cost. |
| **Tailwind** | **REJECT** | 20,871 lines of token CSS. Migration is a rewrite with zero user-visible gain, and it would fight the existing constitution. |
| **shadcn/ui** | **REJECT** | It brings a foreign visual language (rounded, soft, marketing-adjacent) into a sharp 0/2/4 px IDE. Copy patterns, never the chrome. |
| **Lucide** | **REJECT** | We already ship `iconsax-react` plus a custom `Icon` set. Mixing icon families is one of the loudest slop tells. |
| **Motion (runtime library)** | **REJECT** | Our own rule prefers CSS. Fix 75 `transition: all` first; a runtime cannot fix a frequency problem. |
| **Magic UI** | **REJECT** | Gradient/glow effects — precisely the vocabulary the constitution bans, and the reason the app has two lighting models. |
| **Penpot** | **REJECT** | No handoff problem exists; the design lives in code and tokens. |
| **ThinkUI / Uizze** | **NOT ADOPTED** | Paid MCP catalogues solving "no references". We have references; we need discipline. Revisit only if the team grows past one designer. |
| **`npx skills add …unslop-ui -g`** | **DEFER** | A global install with side effects outside this repo. Nothing in the plan needs it; run it yourself if you want the catalog. |

---

## 6. Micro-specs (the actual pixels)

Concrete enough to build against, still decisions rather than code.

### 6.1 Activity rail (48 px, keep)

- Icon 16 px, centred in a 28×28 hit area. Inactive `--fg-muted`; hover `--ink-faint` bg; **active
  = `--ink-hi` filled 28×28 + 2 px leading edge mark + `--fg-bright` icon** (already specified in
  `REVAMP_SPEC §7` — it just doesn't render because of the routing fault).
- **Selection is never derived from another region.** A rail item is selected only if its own
  panel is open and showing it.
- Badges: numeral only, `--type-xs`, tabular, 14 px pill. A dot without a number is only
  permitted for "unread", never for "running".
- Settings pinned to the bottom, no divider.

### 6.2 Status bar (24 px, 5 slots max)

Left: workspace, branch. Right: problems (errors/warnings as numerals), cursor, connection.
Rules: **every value live or absent**; numerals tabular; one space between number and unit
(`42 min`, non-breaking); no internal ids; no hardcoded encodings; no fake spinners.
Agent status does **not** belong here — it belongs to the task (see 6.3), as a count.

### 6.3 Task row (44 px, two lines) — the reference's row anatomy

```
● Fix the login redirect                +12 −3
  main · 2m ago · 3 files
```

- Line 1: 13 px/500, truncate with `…`; right-aligned diff in tabular muted.
- Line 2: 11 px `--fg-faint`, separated by `·`.
- State: 6 px dot **plus** a text label in the tooltip and for screen readers (colour is never
  the only cue). Running = `--ok`, needs-approval = `--warn`, failed = `--err`.
- Selected: `--ink-faint` bg + 2 px leading mark. No border, no shadow, no glow.

### 6.4 Process / service row (mirrors the screenshot exactly)

Two lines: name + mono command at lower contrast, 6 px status dot on the left, port right-aligned
mono. This is the pattern for our workflow runs and terminals list.

### 6.5 Metric card (for turn report, usage, goal iterations)

Big number (24/600), small unit (`k tokens`, `min`), **segmented strip** (10–14 blocks, 4 px gap,
filled = elapsed), one muted line (`Peak 38% · 5 min`). Replaces the current single sentence
`Usage: 12.4k tokens · $0.031 · 9 tools`.

### 6.6 Log line

`10:56:02  web  GET /pricing 200 in 63ms` — time muted/tabular, source muted, message default,
status glyph coloured. Mono throughout. Fixed column widths; truncate at the right, never wrap
mid-token.

### 6.7 Empty state

One sentence, one action, one hint at most. **No brand animation on a surface seen daily**
(that is the onboarding screen's privilege, once).

### 6.8 Error state (one pattern, everywhere)

Three lines, always: *what happened* · *what is safe right now* · *one action*.
Persistent when blocking (not a toast), inline when local to a field, and never a source path.

### 6.9 Approval card

Command/path in mono, a diff summary (files, `+n −n`), **four decisions**
(Allow / Always allow / Reject / Always reject), the consequence line ("this writes 3 files"),
and keyboard affordances that never fire from a text field (already implemented correctly).

### 6.10 Numbers & type

Space Grotesk for chrome and labels; JetBrains Mono for all machine text (paths, commands,
hashes, logs, ports). `font-variant-numeric: tabular-nums` on every count, diff, port, cost,
duration. Non-breaking space before units. `…` character, not `...`. Weights 400/500/600 only
(already correct). One italic use: syntax comments.

---

## 7. Accessibility floor (from Vercel's list, mapped to us)

1. **No dead zones** — if it looks interactive it is. (Direct hit on the fake Git sync.)
2. Visible `:focus-visible` ring on every focusable element; focus never obscured by sticky
   chrome; focus returns on close.
3. Hit targets ≥ 24 px on desktop even when the icon is 12 px.
4. **All states designed**: empty, sparse, dense, error (see Pass 4).
5. `aria-label` on every icon-only button; semantics before ARIA.
6. `aria-live="polite"` for async results; toasts are announcements, not the only channel.
7. Loading: show the original label with a spinner; delay ~150–300 ms, minimum visible ~300–500 ms.
8. Respect reduced motion (already good) and never animate keyboard-initiated actions.
9. Colour never carries meaning alone.
10. Confirm or provide Undo for destructive actions.

---

## 8. Three-week craft plan (with gates)

**Week 1 — repair the constitution.**
Decide glass/glow: legalise for named surfaces or delete the tokens. Collapse to one radius
family and one easing family. Replace all 75 `transition: all` with explicit properties. Delete
the composer beam. Merge the three `*-polish.css` files into their bases.
**Gate:** grep for `transition: all` = 0; `--agents-radius-*` gone; gradient/glass/glow either
gone or listed in a written allowlist with named surfaces; screenshot diff of 6 states is empty.

**Week 2 — apply the micro-specs.**
Rail (6.1), status bar (6.2), task row (6.3), metric card (6.5), log line (6.6), error pattern
(6.8). No new features.
**Gate:** axe-core clean on shell/agent/settings/task-list/terminal/empty; every colour-only cue
paired with text; one row anatomy across tasks, runs and files.

**Week 3 — motion and states.**
Run Pass 1 and Pass 4 over the 12 core controls; fix tooltip timing; add the states gallery.
Encode the 8 acceptance tests in Playwright.
**Gate:** no animation > 300 ms except splash; no animation on palette/panel/tab toggles; nine
states documented for 12 controls; Playwright green.

---

## 9. Acceptance (measurable, not vibes)

1. `transition: all` count: **0**.
2. Radius values in use: **≤ 4** (0, 2, 4, pill) with **0** raw-px bypasses.
3. Easing curves in use: **≤ 4**, all from the four roles.
4. Glass/glow references: **0** or allowlisted per surface (documented, reviewed).
5. Longest non-splash animation: **≤ 300 ms**; loops on 100+/day surfaces: **0**.
6. Every status cue readable with colour removed (screenshot in grayscale still communicates).
7. Every control has 9 states or an explicit "not applicable" note.
8. CSS lines fall ≥ 20% with a **zero-diff** screenshot of six key states.
9. A first-time user can describe what the green dot means without hovering.

---

## 10. Prompt (copy-paste)

```text
/plan

Role: design engineer. Craft pass only — no features, no layout redesign, no code in this turn.

Objective: make Selora stop reading as slop by repairing its design constitution, using
SELORA_CRAFT_SPEC.md as the brief. Most of the work is deletion and consolidation.

Answer these four first, in one short block each:
(1) What would a top design engineer do that I am not asking for? (Inventory every visual effect
    and name the state it communicates; delete the ones that name nothing. Then fix the token
    file so the constitution and the constants agree.)
(2) Rawest material? (tokens.css + the 28 CSS files, the 75 `transition: all` sites, the radius
    census, the three `*-polish.css` files, and one screen recording with a slow-motion pass on
    hover/panel/agent transitions.)
(3) Methods? Frequency gate (Emil), provenance gate (name the shipped precedent or remove),
    role/contrast gate (Vercel), nine-state coverage, 4px-grid structure pass.
(4) Proof other than us? `transition: all` = 0; radius values ≤ 4 with no raw-px bypasses;
    longest non-splash animation ≤ 300ms; grayscale screenshot still communicates every status;
    CSS lines down ≥20% with zero visual diff on six states; a stranger can say what the status
    dot means.

Read: SELORA_CRAFT_SPEC.md; apps/selora-desktop/frontend/src/styles/tokens.css;
styles/{shell,agent,base,animations,dock}.css; the three *-polish.css files;
features/agents-workspace/agents-*.css; layout/{StatusBar,ActivityBar}.tsx.

Constraints:
- Keep the sharp 0/2/4 radius identity. Do not adopt soft/rounded card language, gradients or
  glass by default, Tailwind, shadcn, Lucide, or a motion runtime.
- Cite file:line for every claim and every deletion.
- No visual change may ship without a states gallery entry (default/hover/active/focus/selected/
  disabled/loading/empty/error).
- Prefer deleting an effect to tuning it. If an effect stays, name the state it communicates.

Deliver:
A. Effect inventory: every gradient/glass/glow/shadow/animation in the app, its file, its
   surface, whether it communicates a state, and keep/delete.
B. Token repair: the final radius scale, easing roles, durations, and the allowlist (if any) for
   glass/glow, with the written rule that prevents re-entry.
C. `transition: all` → explicit-property mapping, grouped by component family.
D. The nine-state matrix for the 12 core controls, marking any state that is currently missing.
E. Motion budget table per interaction, checked against the frequency gate.
F. A deletion list with file:line, ordered so each step leaves the app shippable.
G. The three risks (visual regressions, focus/contrast regressions, dead CSS left behind) and the
   check that catches each.
H. The one change you expect to be most visible to users, and why it is worth the risk.

Stop condition: if a fix requires a new dependency, stop and propose the dependency-free
alternative first.
```

---

*Sources used: `vercel-labs/web-interface-guidelines` (MIT); `emilkowalski/skills` (animate,
review-animations, improve-animations — frequency table, duration budgets, easing order);
the attached reference screenshots (information design only); Selora source as cited. ThinkUI
and Uizze were read but not adopted — both are paid MCP catalogues. ZCode and Selora were not
run; the CSS census is from source.*
