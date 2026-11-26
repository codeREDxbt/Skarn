# Selora IDE — Master Research Dossier

> Everything this session produced, in one place: the ZCode reference study, the internal UX
> audit, the craft audit, the merged plan, the deletion list, the acceptance tests, the prompts,
> and an honest ledger of what is **not** covered.
>
> Companion detail files remain in the repo and are summarised here:
> `ZCODE_UX_TEARDOWN.md`, `SELORA_UX_PAINPOINTS.md`, `SELORA_CRAFT_SPEC.md`,
> `ZCODE_PARITY_PROMPTS.md`.
>
> No implementation code. This is research, diagnosis, and design direction.

---

## 0. Read this first

**The one-sentence verdict.** Selora's editor core and agent runtime are ahead of its shell: the
shell routes nine views into three containers by mutable state, gives the same concepts multiple
homes and names, violates its own design constitution in the same file that states it, and
displays decoration as if it were live data — so the product reads as unfinished even where it
works. Almost all of it is fixable by subtraction.

**The six numbers that carry the argument.**

| Number | What it means |
|---|---|
| **9 surfaces → 3 containers** | Routing is a match arm, so the left rail can open a panel at the bottom |
| **5 new-session affordances, 2 labels** | The same action has five homes inside two windows |
| **3 provider-config homes, 2 model homes, 2 effort labels** | Configuration has no owner |
| **80+ glass/glow references across 12 gradient + 8 glass + 16 shadow files** | Two lighting models interleaved file by file |
| **75 × `transition: all`** | The mechanical cause of the "cheap" feel |
| **20,871 CSS lines for 104 `.tsx` files**, incl. 3 `*-polish.css` | Patch-on-patch instead of a system |

**What to do first (all deletion, no design):** delete the fake Git sync, the hardcoded
`Ln 1, Col 1`, the `UTF-8` and `core <hash>` chips, the "Not wired yet" Output tab, the
developer-facing error text, the 75 `transition: all`, and the composer's 2,400 ms beam.

**Track B (rebrand), and its own first action.** The product is being renamed to **Skarn** and the
first-party gateway removed — §16. Its P0 is not design: the auto-updater still points at a
repository the owner may no longer control, so that is repaired before any rename ships.

---

## 1. Method, evidence standard, and limits

**Read, not run.** ZCode was studied from its product documentation (v3.14 → v4.0 docs, Nov 2026
snapshot), its changelog, and its repository layout. Selora was studied from source. Neither app
was launched; no screenshots were compared; no profiles were taken.

**Evidence tags** used throughout: `H` = read directly in source (cited `file:line`), `M` =
inferred from two or more verified facts, `L` = design judgement.

**What this dossier is not.** It is not a usability test, not a performance study, and not a
ZCode source-code audit: ZCode evidence is **documentation + repo layout**, not a line-by-line
read of `apps/zcode-cli/`. That gap is exactly what the `/research` prompt in §13 closes.

---

## 2. Part I — What ZCode is, and why it feels calm

ZCode (Z.ai, open source: `zai-org/ZCode`, ~v3.18/4.0) is not "an IDE with an AI panel". It is an
**Agentic Development Environment**: the *task* is the primary object, and the chat is one view
of it.

**Six structural decisions that produce its feel:**

1. **Task as the object.** Sidebar rows carry title, relative time, status dot (running / unread /
   failed) and the task's own `+/-` line count. Archiving, grouping, pinning and a retention window
   exist because tasks accumulate.
2. **The composer is the command line.** Typed grammar `@` file/folder, `#` past conversation, `/`
   command, `$` skill; a `+` menu for non-typists; long pastes collapse into an attachment.
3. **Modes are a promise, not a setting.** Ask before changes / Edit automatically / Plan / Full
   access, cycled with `Shift+Tab`, with risk state kept visible.
4. **Review is first class.** Per-turn diff, **Undo → Reapply** (all-or-nothing, with a list of
   what can't be restored), edit the last message, and **Reset chat + files** as one action.
5. **Continuity is engineered.** `/goal` with per-round verification and iteration dividers, side
   conversations (`/side`, `/btw`), fork-from-message, editable queued messages, background
   subagents.
6. **Status is ambient and waits are labelled.** Approvals block the turn and mark the task as
   waiting; ordinary questions get a 5-minute countdown, then continue and are marked
   "Unanswered, auto-continued".

**Its quietest and most important property:** the user can always tell whether the machine is
waiting on them or they are waiting on the machine.

### 2.1 Reference surface inventory (what is worth borrowing)

| Area | Detail worth copying |
|---|---|
| Task list | Grouped / Workspace / Timeline, colour groups, drag, move-to-top, archive 3/7/14/30 days, per-task `+/-` |
| Command Center (`Ctrl+K`) | Commands **+ conversations + files** in one panel, shortcuts inline |
| Composer context | `@ # / $`, selection → "Add to current task", paste threshold |
| Modes | 4 modes, `Shift+Tab`; **"Edit automatically" is the middle we lack** |
| Thought level | `Ctrl+T`, Low/High/Max next to the model |
| Permissions | Allow / Always allow / Reject / Always reject, with session/project scope |
| Questions | 5-min countdown, auto-continue, marked in history |
| Review | Per-turn Undo → Reapply, all-or-nothing |
| Edit history | Edit last message + **Reset chat + files** |
| Goal | `/goal`, per-round verification, iteration dividers, pause/resume, budget stop |
| Side conversation | `/side`, `/btw`, inherits history, visually blank, throwaway |
| Fork | New task from a completed reply; original keeps running |
| Subagents | Built-in + custom (name, colour, model, effort, tool allowlist, background) |
| Skills / Commands | `SKILL.md`, `$name`; `/name` with `$ARGUMENTS`; metadata budget |
| Plugins / Hooks | Marketplace bundles; 7 lifecycle events |
| Memory | Off by default; `MEMORY.md` + one file per fact |
| Browser | Panel + element picker → context; agent can drive it |
| Preview | PDF / HTML / Mermaid / image gallery / table copy-as-CSV |
| Usage | Token stats page |

### 2.2 Where ZCode itself is wrong (do not copy)

| Wart | Failure it causes | Our call |
|---|---|---|
| Memory off by default, **no in-app review or delete** | User can't audit the agent's beliefs | Memory must be visible and editable from day one |
| Side conversation is **unrecoverable** on close | Loses work from a "casual" feature | Persist as a child thread, or label the loss loudly |
| Selection capture **single-line only** | Undercuts "point at the thing" | Multi-line + explicit cap message |
| Goal mode **unavailable in Plan mode** | Artificial conflict between what/how | Allow both; they compose |
| Plugin store needs an open workspace + GitHub | Can't manage extensions without a project | Settings must work with no folder open |
| Marketplace vs plugin version comparison bug | Silent "no update" | One version source of truth |
| Countdown toggle is **non-retroactive** | Setting doesn't affect waiting questions | Setting is authoritative immediately |
| Questions auto-continue by default in 5 min | Risky for irreversible choices | Auto-continue non-destructive questions only |

---

## 3. Part II — Selora as it exists (not as the README says)

**Genuinely good, do not rebuild:** CodeMirror 6 editing with LSP, terminal via ConPTY, Git
including split/unified diffs and conflict resolution, plan + todo spines with live progress,
approval cards with split/unified diffs and careful keyboard genealogy (refuses to approve from
inside a text field, `ToolCard.tsx:116`), turn reports with files/tokens/cost, subagent
transcripts **and** a results list, session resumption, pinned sessions, workflows with phase
graphs and checkpointed resume, command palette with categories, cheatsheet, and a token-driven
visual system with correct reduced-motion handling.

**The shell's actual composition** (verified):

- Seven-region grid: title bar / activity bar / sidebar / editor / bottom dock / agent dock /
  status bar (`ShellGrid.tsx`).
- A second top-level window: "Agents", with its own header, rail, context pane and command
  palette (`AgentsWindowApp.tsx`, `AgentsWorkspaceHeader.tsx`).
- One flat `SurfaceId` union of **9** values (`bridge.ts:44`).
- Layout state (region sizes, open flags, active views) owned by Rust, delivered as
  `bootstrap.layout` (`state.rs::apply_toggle`).

---

## 4. Part III — The three faults

### 4.1 "Jumbled" = routing is a `switch` statement

`apply_toggle` (`state.rs:4927`) maps surfaces to containers:

| Surface | Container | Toggle again |
|---|---|---|
| `files`, `search`, `git`, `workflows`, `settings` | Primary sidebar (left) | closes sidebar |
| `agent` | Agent dock (right) | closes dock |
| `problems`, `output`, `terminal_placeholder` | Bottom dock | closes bottom dock |

Consequences, each verified:

| ID | Symptom | Cause / evidence | Sev |
|---|---|---|---|
| J1 | The fourth icon in the **left** rail opens a panel at the **bottom** | `problems` is a rail item routed to the bottom dock (`state.rs:4938`) | P0 |
| J2 | That icon **never lights up** | selected requires `activity_view === 'problems'`, but the toggle writes `bottom_view` (`ActivityBar.tsx:227`) | P0 |
| J3 | `Problems` exists in the rail **and** as a bottom tab | `PrimarySidebar.tsx:46`, `BottomDock.tsx:26` | P1 |
| J4 | Sidebar has labels for `Output` and `Terminal` it can never show | `VIEW_LABEL` covers all 9 (`PrimarySidebar.tsx:41-48`) | P1 |
| J5 | A third session list ships but is unreachable | sidebar `agent` route vs `toggle('agent')` opening the dock (`:223`) | P1 |
| J6 | `Ctrl+B` = sidebar in one window, nav rail in the other | `commands.ts:399` vs `AgentsWorkspaceHeader.tsx:47` | P1 |
| J7 | Settings opens in a ~300 px column and closes with the sidebar | settings as a sidebar *route* (`:222`) | P0 |

**Fix (one rule):** a region owns a *kind* of content. Left = project, bottom = process, right =
task, settings = a real page. A rail icon never opens a different region.

### 4.2 "Unorganised" = multiple homes and names

| Concept | Homes | Evidence |
|---|---|---|
| Provider + key | **3** (Connect modal, Settings → Gateway, Customizations) | `App.tsx:174`, `SettingsPanelView.tsx:43`, `AgentsCustomizationsView.tsx:14,124` |
| Model | **2** | composer menu, `AgentsCustomizationsView.tsx:99` |
| Reasoning effort | **2**, labels differ (`Max` vs `Maximum`) | `AgentComposer.tsx:106-111` vs `AgentsCustomizationsView.tsx:111` |
| Instructions | **1 global textarea**, nothing per project | `AgentsCustomizationsView.tsx:112` |
| New session | **5** affordances, 2 labels | tabs `+`, rail button, `Ctrl+N`, palette, internal rail button |
| Session list | **3** projections | `AgentSessionTabs`, `AgentSessionRail`, nav rail |
| Todo state | **4** renderings in one view | tab label, infeed bar, popover, spine |
| "Editor" button | **2**, different meanings | dock header vs Agents header |

Plus settings controls that **cannot save**, each explaining the protocol gap in a hover tooltip
(`unexposedTitle`, `supportedTitle`, `SettingsPanelView.tsx:83,90`) — inline help as a last
resort, used here to apologise for unfinished work.

### 4.3 "Sloppy" = the UI asserts things that aren't true

| ID | What the user sees | Reality | Evidence | Sev |
|---|---|---|---|---|
| T1 | Branch click spins 1.2 s | No command, no network — just `setTimeout` | `StatusBar.tsx:119-125` | P0 |
| T2 | `Ln 1, Col 1` | Hardcoded | `StatusBar.tsx:190` | P0 |
| T3 | `UTF-8` | Hardcoded | `StatusBar.tsx:186` | P1 |
| T4 | Language name | Falls back to literal `'TypeScript'` | `StatusBar.tsx:88-92` | P1 |
| T5 | `core 3f9a1c2e` | Internal instance id | `StatusBar.tsx:193` | P2 |
| T6 | Output tab with badge **"Not wired yet"** | Unfinished surface shipped | `OutputPanel.tsx:15` | P0 |
| T7 | `terminal_placeholder` route + `.terminal-placeholder-*` CSS | Placeholder became permanent identity | `bridge.ts:44` | P1 |
| T8 | Error prints a **source path and "Phase 8 UI slice"** | Build-planning language to users | `PrimarySidebar.tsx:143` | P0 |
| T9 | Tooltip "Settings editing lands in a later slice" | Same | `StatusBar.tsx:64` | P1 |
| T10 | `selora connected` lowercase | Brand inconsistency | `StatusBar.tsx:47` | P2 |

**Why this outranks features:** users calibrate trust from cheap signals. One fake spinner teaches
them that indicators are decoration — after which the approval badge and the unsaved-changes dot
are also "probably decoration".

### 4.4 Feedback: dozens of toasts, 3 visible

Dozens of `toastStore.pushLocal` call sites; `TOAST_MAX_VISIBLE = 3`; severity-based auto-dismiss
(`toast-store.ts:18,59,224`). Meanwhile **file-locking** conditions are delivered as toasts:
"Keep this file open until the workspace reconnects" (`editor-store.ts:481`), "Cannot close this
file while editor changes are unsynchronized" (`:489`), "The close result is unknown" (`:284`).
A blocking condition is not a 4-second banner; it belongs on the tab, in Problems, and in a
status area.

### 4.5 The first sixty seconds (why it feels off before any feature is tried)

Boot → onboarding that teaches the manifesto, not the map → a shell with six rail icons and a
`core` hash → fourth icon opens a bottom panel and stays unlit → Settings is a narrow column with
apologetic tooltips → `Ctrl+L` opens a thing called "Agent" that also has a mode called "Agent" →
five approval cards until the user stops reading → `⧉` opens a second window with the same session
and divergent scroll → `Ctrl+B` changes meaning by window → "where do I change the model?" has
three correct answers.

---

## 5. Part IV — Craft: the constitution is violated by its own constants

`tokens.css` opens with rules ("no gradients on chrome, no box-shadows, no background images;
saturated color only via status tokens") and then defines `--glass-*`, `--blur-*`, `--glow-*` and
four `--grad-*` tokens. The code follows the tokens.

| Banned effect | Files using it | Token refs |
|---|---|---|
| `gradient()` | **12** | `--grad-*` |
| `backdrop-filter` | **8** | `--glass-*` ×28 |
| `box-shadow` | **16** | — |
| `--glow-*` | — | **80+** (23 `--glow-accent`, 13 `--glow-green-subtle`, 11 `--glow-green`, …) |

**Radius census:** 98 `--radius-1`, 41 raw `4px`, 38 `--radius-pill`, 29 raw `6px` (not on the
scale), 26 `--radius-2`, 23 raw `3px`, 17 raw `8px`, 14 `var(--radius-2, 6px)` (**fallback
contradicts its token**), 14 raw `2px`, 10 `5px`, 7 `7px`, 5 `12px`, plus a **second family**
`--agents-radius-*` (22 uses). Two radius systems, ~10 bypass values.

**Motion:** **75 × `transition: all`**; six easing tokens for four roles; a **2,400 ms** composer
border beam on the most-used surface; overshoot spring on disclosure panels. The token set has
**no `ease-in`** (correct) and reduced-motion is handled on two paths (correct — protect it).

**What the reference screenshots teach (information design only, 12 rules):** active rail item is
a filled square and never depends on another region; rail carries **numbers** not colours alone;
two-line row anatomy (label + mono detail); right-aligned muted values do the grouping; metric
cards = big number + small unit + one context line + segmented history strip; tiny muted section
labels with one action; mono reserved for machine data; colour only where it means something;
separation by hairline + whitespace, not floating cards; logs with fixed columns and tabular
numerals; high density with 8–12 px intra-group and 24 px inter-group rhythm; nothing animates
except status.

**Deliberately not copied:** 10–14 px soft radii (Selora's sharp 0/2/4 is an identity), the
graph-paper backdrop, and a destructive action ("Clean up 16.6 GB") sitting as a navigation row.

---

## 6. Part V — Principles (the ten that settle arguments)

1. **One state, one name.** Two names for one state is a bug, not a tone choice.
2. **Every wait says who owns it.** Approval = must act; Question = may act, machine proceeds;
   Background = nobody is waiting. No fourth silence.
3. **Reversibility beats confirmation.** An undoable action is worth three prompts.
4. **One task, one home.** A task lives in one list and opens in one place; other entry points
   navigate there rather than render a copy.
5. **Delight rides on legibility.** Novelty may decorate; it may never be the only way to read.
6. **A region owns a kind of content.** Placement is taxonomy, not a `switch` arm.
7. **Status bar policy: live or absent.** No decoration, no internal ids.
8. **Frequency is the motion gate.** 100+/day → none; keyboard-initiated → never.
9. **Delete before design.** The fastest quality win in this repo is subtraction.
10. **Provenance or remove.** A non-obvious pattern must name the shipped product it follows.

---

## 7. Part VI — The full register

Two registers, deliberately separate: **A** is coherence (we built it wrong), **B** is capability
(we haven't built it). Four items appear in both and are marked.

### 7.1 Register A — internal coherence (31)

**P0 — fix before adding anything (11)**

| ID | Pain | Evidence | Change |
|---|---|---|---|
| A-P0-1 | Rail icon opens a panel in another region | `state.rs:4938` | Region owns content kind |
| A-P0-2 | That icon can never appear selected | `ActivityBar.tsx:227` | Remove Problems from the rail |
| A-P0-3 | Git "sync" spins, does nothing | `StatusBar.tsx:119` | Wire or delete |
| A-P0-4 | `Ln 1, Col 1` hardcoded | `StatusBar.tsx:190` | Bind or remove |
| A-P0-5 | Output ships "Not wired yet" | `OutputPanel.tsx:15` | Remove the tab |
| A-P0-6 | Error prints source path + phase name | `PrimarySidebar.tsx:143` | One sentence + retry |
| A-P0-7 | Settings is a 300 px sidebar route | `PrimarySidebar.tsx:222` | Promote to a page |
| A-P0-8 | Six competing affordances in one task surface | `AgentConversationSurface.tsx` | One nav, one progress, one plan |
| A-P0-9 | Eleven peer elements in one header row; two say "Editor" | `:120-250` | Three groups |
| A-P0-10 | Provider ×3, model ×2, effort ×2 with different words | §4.2 | One config surface |
| A-P0-11 | Modes have two vocabularies; "Agent" is permissive | `types.ts:17`, `AgentComposer.tsx:87,122` | Four honest names |

**P1 — daily friction (12)**

| ID | Pain | Evidence |
|---|---|---|
| A-P1-1 | `Ctrl+B` differs by window | `commands.ts:399` vs `AgentsWorkspaceHeader.tsx:47` |
| A-P1-2 | Five ways to start a session, two labels | §4.2 |
| A-P1-3 | Hidden second session list behind a text button | `AgentConversationSurface.tsx:279` |
| A-P1-4 | Dead sidebar routes/labels | `PrimarySidebar.tsx:41-48,223` |
| A-P1-5 | Same session in two windows, independent scroll/draft | `openAgentsWindow` from dock |
| A-P1-6 | Blocking file state as a 4 s toast | `editor-store.ts:481,489` |
| A-P1-7 | Dozens of toasts, 3 visible, history behind a bell | `toast-store.ts:18` |
| A-P1-8 | Hardcoded `UTF-8`, `TypeScript` fallback | `StatusBar.tsx:88,186` |
| A-P1-9 | "Customizations" = config inside conversation | `AgentsNavigationRail.tsx:97` |
| A-P1-10 | Non-savable controls with apology tooltips | `SettingsPanelView.tsx:83,90` |
| A-P1-11 | `terminal_placeholder` as permanent identity | `bridge.ts:44` |
| A-P1-12 | Duplicate class aliases, 3 naming prefixes | `AgentSessionTabs.tsx:52` |

**P2 — craft debt that regenerates P0s (8)**

| ID | Pain | Evidence |
|---|---|---|
| A-P2-1 | 20,871 CSS lines; 3 `*-polish.css` | census |
| A-P2-2 | 20 `!important` in `shell.css` | census |
| A-P2-3 | Inline styles in shell components | `PrimarySidebar`, `StatusBar` |
| A-P2-4 | Onboarding teaches manifesto, not map | `OnboardingOverlay.tsx:16-31` |
| A-P2-5 | Subagent status in title bar, details in feed | `TitleBar.tsx:339` |
| A-P2-6 | `core <hash>` in status bar | `StatusBar.tsx:193` |
| A-P2-7 | Empty-state suggestions differ per window | `AgentConversationSurface.tsx:102` |
| A-P2-8 | Lowercase `selora connected` | `StatusBar.tsx:47` |

### 7.2 Register B — capability gaps vs ZCode (22)

| # | Gap | ZCode ref | Sev | Overlaps A |
|---|---|---|---|---|
| B1 | Two vocabularies for permission states | one label set | P0 | A-P0-11 |
| B2 | No "edits auto, commands ask" mode | Edit automatically | P0 | A-P0-11 |
| B3 | No scoped allow/deny | Allow / Always allow / Reject / Always reject | P0 | — |
| B4 | Questions wait forever, unlabelled | 5-min countdown + "auto-continued" | P0 | — |
| B5 | No per-turn undo/reapply | Undo → Reapply, all-or-nothing | P0 | — |
| B6 | No project instructions (`AGENTS.md`) | global + workspace, ordered | P0 | — |
| B7 | Machine text injected into the user's prompt | structured attachments | P0 | — |
| B8 | Paths-only context; no images or selection | image paste, selection → task | P0 | — |
| B9 | No `@ # / $` grammar | composer as command line | P1 | — |
| B10 | No side conversation | `/side`, `/btw` | P1 | — |
| B11 | No unified Command Center | `Ctrl+K` (commands+chats+files) | P1 | A-P1-7 (partial) |
| B12 | No find-in-conversation | `Ctrl+F` in conversation | P1 | — |
| B13 | Keyboard map diverges from muscle memory | `Ctrl+N/M/T`, `Shift+Tab`, `Ctrl+[/]` | P1 | A-P1-1 |
| B14 | No archive / custom groups / per-task `+/-` | grouped tasks, retention | P2 | — |
| B15 | No fork | fork from a completed reply | P2 | — |
| B16 | No goal loop with verification | `/goal` | P2 | — |
| B17 | No skills / commands on disk | `SKILL.md`, `$name`, `/name` | P2 | — |
| B18 | No custom subagents | name/colour/model/tools/prompt | P2 | — |
| B19 | No MCP, no hooks | servers + 7 lifecycle events | P2 | — |
| B20 | No browser panel / element picker | picker → context; agent drives | P2 | — |
| B21 | No rich preview (Mermaid, tables, images, PDF) | conversation viewers | P2 | — |
| B22 | Usage is per-turn only | stats page | P2 | — |

### 7.3 Capability gaps that are deliberate non-goals

Remote control from phone, bot channels, plugin marketplace, repo wiki, SSH/Docker remote
development, iOS/Android emulator drivers. These are ZCode's distribution strategy for a hosted
ADE. Adding them to a Windows desktop editor with bring-your-own keys buys attention we cannot
keep.

---

## 8. Part VII — Vocabulary canon

| Concept | Use | Never use |
|---|---|---|
| Permission states | Read-only · Ask before changes · Auto-edit · Full access | Ask/Agent/Plan row; "Full auto-apply (Armed)"; "Guarded" |
| A unit of work | **Task** | session, conversation, chat, thread (as synonyms) |
| Waiting on the user | **Needs approval** | "pending", "blocked" |
| Waiting, optionally | **Question** (+ countdown) | "needs attention" |
| Machine running | **Running** | "thinking", "breathing" |
| Reversing one turn | **Undo turn / Reapply** | "reset", "revert" (reserved for Git) |
| Agent-authored edits | **Agent changes** | "changes" (ambiguous with Git) |
| Workflow runs | **Runs** | "Automations" until triggers exist |
| Provider config | **Providers** | "Gateway", "Assistant providers" |
| Reasoning | **Reasoning** | Max / Maximum (pick one) |
| New work | **New task** | "New Session" |

---

## 9. Part VIII — The unified plan (3 roadmaps reconciled)

Three plans were produced separately: ZCode waves (§ earlier), coherence weeks, craft weeks. They
overlap in the first two weeks and diverge later. This is the merged sequence — **eight weeks,
each with one gate**. Do not start a week before the previous gate is green.

**Week 0 (half day) — instrument.** Record one real 20-minute session with the mouse path. Baseline
six numbers: approvals/session, turns sitting >2 min unanswered, mode switches/session, undo-vs-git
ratio, prompts that re-explain project facts, time-to-first-accepted-change.

**Week 1 — remove lies + repair the constitution.** (Deletion only, different files, so parallel-safe.)
A-P0-3, A-P0-4, A-P0-5, A-P0-6, A-P2-6, A-P2-8; plus all 75 `transition: all` → explicit
properties; delete the composer beam; collapse six easings to four; merge the three `*-polish.css`
into their bases.
**Gate:** no dead-looking control; `grep` for `transition: all` = 0; six-state screenshot diff empty.

**Week 2 — routing.** J1, J2, J4, J5, J7 + remove Problems from the rail; promote Settings to its
own surface; delete dead routes and their labels.
**Gate:** every rail icon lights up and opens to its right; Settings survives closing the sidebar.

**Week 3 — collapse the agent surface.** A-P0-8, A-P0-9, A-P1-2, A-P1-3, A-P1-5 (state sharing
decision), A-P1-9 (move Customizations), two "Editor" buttons → one meaning.
**Gate:** one session navigation and one progress display per task surface; the same session in
both windows agrees on scroll and draft.

**Week 4 — one vocabulary, one config home.** A-P0-10, A-P0-11, A-P1-10, B1, B2, B3, B4.
Four mode names; three approval scopes; question clock + "continue without me"; one Providers home;
one Reasoning label; delete non-savable controls.
**Gate:** grep returns exactly one label per state; median approvals/session falls without more
"Full access" usage; no turn can wait invisibly.

**Week 5 — reversibility and context.** B5 (per-turn checkpoints, Undo/Reapply), B6 (`AGENTS.md` +
rules + "rules in effect" chip), B7 (structured context, no protocol English in the transcript),
B8 (image + selection context).
**Gate:** reverse one agent turn without losing a human edit; transcript contains no
`read them with read_file`; a fresh session already knows the test command.

**Week 6 — craft application.** Craft micro-specs: rail, status bar, task row, metric card, log
line, error pattern; radius/easing consolidation; states gallery; axe-core in CI.
**Gate:** axe clean on six screens; radii ≤ 4 values, no raw-px bypasses; longest non-splash
animation ≤ 300 ms.

**Week 7 — flow.** B9 (`@ # / $`), B10 (side conversation), B11 (`Ctrl+K` Command Center),
B12 (find in conversation), B13 (keyboard map).
**Gate:** any destination in ≤ 3 keystrokes; a user can reference file/conversation/command/skill
without opening a panel.

**Week 8 — encode and measure.** Playwright for the acceptance tests; re-measure Week 0's six
numbers; decide Wave 3 (B14–B22) on evidence, not appetite.
**Gate:** the eight structural tests pass in CI; the six metrics moved in the predicted direction.

**Wave 3 (post-week-8, ordered by dependency):** skills + commands on disk → goal loop (needs
checkpoints) → custom subagents → MCP + hooks → browser picker → rich preview → task-list maturity
→ aggregate usage.

---

## 10. Part IX — Deletion list (the fastest quality win)

1. Git sync button (or its fake handler) · 2. `Ln 1, Col 1`, `UTF-8`, `core <epoch>` · 3. the
`TypeScript` default language · 4. the **Output** tab · 5. `Problems` in the activity bar ·
6. sidebar labels for `problems`/`output`/`terminal_placeholder`/`agent` · 7. the sidebar `agent`
route + `AgentSidebarView` · 8. the "Session history" text button and embedded rail · 9. the
floating-plan toggle and infeed progress bar · 10. the duplicate permission menu + its vocabulary ·
11. Connect modal as a separate config path · 12. Customizations' model/effort/provider sections ·
13. every control backed by `unexposedTitle`/`supportedTitle` · 14. the three `*-polish.css` ·
15. duplicate class aliases · 16. the 2,400 ms beam · 17. all 75 `transition: all` ·
18. `.zcode/` dev scratch and the stray `nul` file from the repo root.

---

## 11. Part X — Acceptance (merged, measurable)

**Structural (8):** rail test (every icon opens right, lights up) · no dead clicks · status truth
test (change cursor/file/branch, everything that claims to describe them changes) · one-home test
(three people name the same place for "change the model") · label test (grep returns one label per
concept) · window test (state shared or declared snapshot) · blocking test (instruction survives 30
seconds on the tab and in Problems) · CSS test (−20% lines, zero visual diff).

**Craft (9):** `transition: all` = 0 · radii ≤ 4 with 0 bypasses · easings ≤ 4 · glass/glow 0 or
allowlisted · longest non-splash ≤ 300 ms · status readable in grayscale · nine states per control ·
CSS −20% zero-diff · a stranger can say what the status dot means.

**Behaviour metrics (from Week 0):** approvals/session ↓ ≥40% without more Full-access · silent
waits >2 min → 0 · mode switches ↓ · undo > `git checkout` after agent work · re-explaining ↓ ·
protocol text in transcripts = 0.

**The single best external check:** a first-time user completes *small fix → review → revert* with
no questions. If they ask "which window am I in?", IA isn't done. If "what does Agent mean?",
vocabulary isn't done. If "how do I undo that?", reversibility isn't done.

---

## 12. Part XI — Reference material used this session

**ZCode (product):** `zcode.z.ai/en/docs` — agents, goal, memory, skill, plugin, subagents,
safety-confirm, ADE-tools, task-management, edit-history, keyboard-shortcuts; plus changelog and
`github.com/zai-org/ZCode` (README v3.18.2, repo layout, `apps/zcode-cli/`).

**Free methods adopted:** `vercel-labs/web-interface-guidelines` (MIT; ships `AGENTS.md`) ·
`emilkowalski/skills` (frequency table, duration budgets, easing decision order).

**Paid catalogues — read, not adopted:** ThinkUI (`thinkui.xyz`, MCP, from $7.99/mo) and Uizze
(`uizze.com`, $9/mo). Correction: "Anti UI Slop" is Uizze's landing-page claim, not a published
skill. A reference library solves "no taste of my own"; our problem is a constitution we violate.

**Tool verdicts (11 repos):**

| Verdict | Tools |
|---|---|
| Adopt | Vercel guidelines (as repo `AGENTS.md`), Emil's skills (as review pass), axe-core, Playwright |
| Adapt | ui-ux-pro-max-skill (checklist), Storybook (a dev-only states gallery, not a full install) |
| Defer | Radix primitives (our ARIA is careful), `npx skills add …unslop-ui -g` (global install, not needed) |
| Reject | Tailwind, shadcn/ui, Lucide, Motion runtime, Magic UI, Penpot |

**Not run / not found:** `gvs` skill library — only the root of `X:\` was listed; no `gvs` there.
If it exists deeper on that drive, give the path and it can be folded in.

---

## 13. Part XII — Prompts (all four, copy-paste)

### 13.1 `/research` — settle the six contested decisions

```text
/research

Objective: produce an evidence-backed decision memo on the six contested UX decisions in
SELORA_MASTER_DOSSIER.md, so we stop arguing from taste. Research only. No code, no plan.

Contested decisions:
1. Converge on ZCode's four permission modes and its vocabulary, or keep three with honest labels?
2. Merge the Agents window and the agent dock into one surface, or keep two with a clear division?
3. Is an auto-continuing question timer (5 min) acceptable for an agent that writes files?
4. Are per-turn file checkpoints worth the storage/conflict complexity, or is a pre-turn Git
   snapshot enough?
5. Should context be typed (@ # / $ grammar) or attachment-driven (drag, chips, menus)?
6. Does ZCode's plugin/skill ecosystem matter for a bring-your-own-key Windows editor?

Raw material, in priority order — not marketing copy:
- ZCode source: github.com/zai-org/ZCode — apps/zcode-cli/ (permission gate, composer mention
  parsing, task/session store, question timeout), skills/commands loaders, plugin manifests.
- ZCode changelog: read as a defect log; every fix is an admission. Extract entries touching
  permissions, waits, undo, context, task list.
- ZCode docs "Limits and Notes" sections: self-admitted failures are the most honest UX data.
- ZCode GitHub issues: search confusion language ("which window", "always allow", "lost my
  conversation", "undo").
- Our source: the files cited in the dossier, plus sessions.rs / state.rs for what is actually
  persisted about approvals and writes.
- One 20-minute screen recording of a real Selora session, transcribed into a confusion log
  (timestamp + attempt + outcome). If unavailable, say so and use a scripted walkthrough — never
  simulate data.

Method: heuristic evaluation of both products on five task scripts (small fix; multi-file refactor
with review; recover from a bad turn; ask a side question mid-run; hand off to a background agent);
keystroke count on three flows (send, approve, undo); vocabulary audit grouped by concept;
reversibility audit; interruption audit (owner + timeout per wait).

Deliver: (A) verdict per decision — adopt/adapt/reject, one paragraph, strongest evidence for and
against; (B) evidence table — claim | source | what it says | confidence; (C) the confusion log,
verbatim; (D) three things ZCode does that we should NOT copy, with the failure each causes;
(E) what would falsify each verdict; (F) open questions only a user test can answer.

Rules: quote when wording matters; distinguish "docs say" from "code does" from "I infer"; no
roadmap; no code.
```

### 13.2 `/plan` — structure and routing (internal coherence)

```text
/plan

Role: product engineer + interaction designer. Planning only — no code, no diffs, no file writes.

Objective: reduce Selora's shell to one home per concept, using SELORA_MASTER_DOSSIER.md as the
brief. Deletion-and-routing plan, not a feature plan.

Read: SELORA_MASTER_DOSSIER.md; state.rs (apply_toggle + layout DTO); bridge.ts (SurfaceId +
Layout); layout/{ActivityBar,PrimarySidebar,ShellGrid,StatusBar,TitleBar}.tsx;
AgentConversationSurface.tsx; AgentComposer.tsx (MODE_OPTIONS + COMPOSER_MODES);
SettingsPanelView.tsx; AgentsNavigationRail.tsx; AgentsCustomizationsView.tsx;
AgentsWorkspaceHeader.tsx.

Produce:
A. Surface census: surface | container(s) | trigger | selected-state rule | reachable at all.
B. Concept census: concept | every home | every label | what survives.
C. Truthfulness table: every displayed value | live or constant | keep/remove.
D. Target region model as an ownership rule, plus the rule that prevents future violations.
E. Deletion list with file:line, ordered so each step leaves the app shippable.
F. Routing change list: each toggle arm before/after, and what happens to persisted layouts
   referencing a removed surface.
G. Risk notes: stored activity_view, stored bottom_view, muscle memory, the second window — and
   the migration story for each.
H. The three decisions you are least sure about, each with a one-day experiment.

Constraints: deletion over addition; no new top-level navigation; no visual redesign; keep tokens;
every removed string gets one replacement or none; do not preserve both vocabularies.
Stop condition: if a change touches more than two shared integration files, split it into a
sequencing step instead of a big bang.
```

### 13.3 `/plan` — craft pass

```text
/plan

Role: design engineer. Craft pass only — no features, no layout redesign, no code in this turn.

Objective: make Selora stop reading as slop by repairing its design constitution, using
SELORA_MASTER_DOSSIER.md (Part IV) as the brief. Most work is deletion and consolidation.

Read: SELORA_MASTER_DOSSIER.md; tokens.css; styles/{shell,agent,base,animations,dock}.css;
the three *-polish.css files; agents-*.css; layout/{StatusBar,ActivityBar}.tsx.

Constraints: keep the sharp 0/2/4 radius identity; no soft/rounded card language; no gradients or
glass by default; no Tailwind, shadcn, Lucide, or motion runtime; cite file:line for every claim
and deletion; no visual change ships without a states-gallery entry (default/hover/active/focus/
selected/disabled/loading/empty/error); if an effect stays, name the state it communicates.

Deliver:
A. Effect inventory: every gradient/glass/glow/shadow/animation, file, surface, state it
   communicates, keep/delete.
B. Token repair: final radius scale, easing roles, durations, glass/glow allowlist (if any), and
   the written rule preventing re-entry.
C. `transition: all` → explicit-property mapping by component family.
D. Nine-state matrix for the 12 core controls, marking missing states.
E. Motion budget per interaction, checked against the frequency gate.
F. Deletion list with file:line.
G. Three risks (visual regression, focus/contrast regression, dead CSS) and the check catching each.
H. The change most visible to users, and why it is worth the risk.

Stop condition: if a fix needs a new dependency, stop and propose the dependency-free alternative.
```

### 13.4 The meta-prompt that shaped all three (keep for future sessions)

```text
I want a deeper version of this task, not a bigger one. Before rewriting it:
(1) say what a top professional in this field would do here that I'm not asking for;
(2) name the rawest material they would start from instead of what has already been digested for me;
(3) name the specific methods and tools they would use;
(4) say what, other than you or me, would show the result is right.
Then rewrite my prompt to ask for exactly that.
```

---

## 14. Part XIII — Coverage ledger (what is and isn't in this dossier)

**Produced this session — all five documents exist and remain the detail layer:**

| File | Contents | In master? |
|---|---|---|
| `ZCODE_UX_TEARDOWN.md` (490 lines) | ZCode study, 22-row gap matrix, 13-point slop audit, ZCode's own warts, waves, metrics, keyboard delta, ownership map | **Summarised** — full keyboard delta and ownership map stay there |
| `SELORA_UX_PAINPOINTS.md` (539 lines) | Structural faults, 31-item register, first-60-seconds walkthrough, target model, delete list, acceptance | **Summarised** — walkthrough abbreviated |
| `SELORA_CRAFT_SPEC.md` (401 lines) | Tokens contradiction, 12 reference rules, motion policy, five passes, tool verdicts, 10 micro-specs, craft plan | **Summarised** — micro-spec pixel values stay there |
| `ZCODE_PARITY_PROMPTS.md` (167 lines) | The two original prompts | **Merged** into §13 |
| `REBRAND_PLAN.md` | Naming brief with six rejected candidates, measured rename census (358 files), the five breaking identities, gateway removal checklist, repo bootstrap, six-phase work order, 10-test matrix | **Integrated** — §16 is the executable summary; the long tables stay there |
| `SELORA_MASTER_DOSSIER.md` | This file: synthesis + unified plan | — |

**Deliberately left out of the master (present in the detail files):**

1. **The 10 detailed micro-specs** (rail hit areas, row anatomy to the pixel, log columns, metric
   card block counts, approval card layout). Kept in `SELORA_CRAFT_SPEC.md` §6 — reproducing them
   here would duplicate a spec that implementers should read in full.
2. **The keyboard delta table** (13 rows) — `ZCODE_UX_TEARDOWN.md` Appendix A.
3. **The ownership map** of shared integration files — `ZCODE_UX_TEARDOWN.md` Appendix C; it is
   the file-ownership contract for parallel work.
4. **ZCode's plugin/marketplace field references** (JSON schemas for `plugin.json`,
   `marketplace.json`, `hooks.json`, MCP declarations). Researched, but implementation-level and
   only relevant if Wave 3 reaches plugins — currently a non-goal.
5. **The 13-point slop audit (S1–S13)** as written. Each finding is preserved in Register A or
   Part IV, but the designer's prose sketches are only in the teardown.
6. **The "first sixty seconds" full narrative** — condensed to eight steps in §4.5.

**Never researched this session (be explicit with stakeholders):**

- ZCode **source code** beyond repo layout/README — its permission gate and mention parser were
  not read line by line. The `/research` prompt (§13.1) exists to close this.
- Selora surfaces outside the shell/agent/settings/terminal-chrome path: `FileTreeView`,
  `GitPanelView`, `SearchPanelView`, `ProblemsPanel`, `TerminalPanel`, `WorkflowsPanel`, the
  CodeMirror integration, and the Rust side beyond layout/approval/session ownership. Findings
  about those areas would be new research, not a missed summary.
- Any runtime observation: no launches, no profiles, no screenshots, no recordings. All timing and
  behaviour claims are source-derived.
- Accessibility testing with a real screen reader; colour contrast was not measured numerically.
- Localisation, keyboard layouts beyond QWERTY, and Windows high-contrast mode.
- `gvs` skill library on `X:\` — only the drive root was listed; not found there.
- **Legal name clearance.** `REBRAND_PLAN.md` §1.2 recommends a name but ran no trademark
  search (USPTO/EUIPO classes 9/42), no domain check and no handle check — those are listed as a
  thirty-minute protocol, deliberately left to the owner.
- **The rebrand migration itself.** No rename was applied and no upgrade path was exercised. The
  settings/keyring/installer migration in `REBRAND_PLAN.md` §3 is a design to be tested, not an
  observed result.

**One judgement call worth flagging:** the craft findings were originally P2 ("craft debt"). The
tokens contradiction (constitution vs 80+ banned-token uses) is severe enough that Week 1 now
carries it alongside the P0 lies, because two lighting models in one product is a *system
integrity* failure, not polish.

---

## 15. Open questions (unresolved, and who can answer them)

| Question | Why it matters | Answered by |
|---|---|---|
| Is the agent dock or the Agents window the real home? | Determines Week 3's shape and whether state sharing is required | Product decision + the §13.1 research |
| Do users want four permission modes, or is three with honest names enough? | Changes the backend permission contract | `/research` decision 1 |
| Is per-turn undo worth the storage/conflict cost vs a pre-turn Git snapshot? | Blocks B5 and the goal loop | `/research` decision 4 |
| Should questions auto-continue? | Irreversible actions could proceed unattended | `/research` decision 3 + user test |
| What is the actual felt cost of the 75 `transition: all`? | The one claim not measured | A five-minute recording with slow-motion |
| Does the second window serve a real workflow? | If not, deleting it removes a fifth of the IA | User interviews |

---

## 16. Part XIV — Rebrand: Selora → **Skarn** (and de-brand)

Track B, parallel to the UX work and owned separately. Detail file: **`REBRAND_PLAN.md`**. This
Part is the executable summary and is sufficient on its own for P0–P2.

**Why it is track B and not a find-and-replace.** The name sits in 358 files, in the installer's
uninstall identity, in the Windows credential store, and in the auto-updater's trust anchor. Three
of those four can lose user data; the fourth can execute code from a namespace the owner no longer
controls.

### 16.1 The name

**`Skarn`** — a real geological term: the zone where rock is transformed at the contact between
magma and surrounding stone. The ore is at the boundary, not in either body, which is the same
claim this dossier makes about the product.

Six candidates were checked live and **all six were already taken**, five by coding agents or
editors — which is the actual finding:

| Rejected | Occupied by |
|---|---|
| Cairn | four AI-coding products (`cairn/cairn` terminal agent, `cairn.computer`, `cairn-dev`, `cairn-framework`) |
| Burin | `burin.ai` agent, "Burin Code" workbench, a Rust GUI crate |
| Umber | a GPU painting app in Rust |
| Mullion | a macOS window manager built on 6-pane grids |
| Kerf | `kerf.fyi` Monaco editor, a Tauri/MCP editor, a CAD app |
| Chamfer | an AI CAD agent in the Microsoft Store |

**Conclusion for any future naming:** every short craft noun is now an AI coding product. Coin the
name; do not borrow one. The pattern that produced Skarn — a rare geological/nautical/optical root,
1–2 syllables, hard consonant, no suffix — is repeatable. Never `-AI`, `-GPT`, `-ify`, `-ly`, `-io`,
or "Code/Dev/Studio" as the whole name (which is why the recorded name is `Skarn`, not `Skarnify`).

**Still outstanding, and deliberately the owner's job:** the thirty-minute clearance run — USPTO and
EUIPO classes **9** and **42**, the category search, GitHub org, npm/crates/PyPI, domain, store
name, handles. Nothing is printed, registered, or published before that table exists in
`docs/brand/NAME_DECISION.md`.

**Fixed identity tokens:** `Skarn` · `skarn` · `app.skarn.desktop` · `Skarn-Setup-<version>.exe` ·
`%APPDATA%\skarn` · keyring service `skarn-ide` · env prefix `SKARN_` · first release **1.0.0**.

### 16.2 Census (measured)

358 files; occurrences: `Selora` 659 · `selora` 593 · `selora_ai::` 446 · `selora-desktop` 419 ·
`SeloraError` 309 · `selora_protocol::` 214 · `selora-ui` 212 · `selora_settings::` 205 ·
`SeloraStatus` 133 · `Selora-IDE-Setup-` 33 · `selora-dark` 30 · `selora:` 29.

**The five identities that break** (everything else is mechanical):

| # | Identity | Current value | What breaks |
|---|---|---|---|
| 1 | Tauri `identifier` | `lol.selora.ide` | App identity + WebView2 user-data dir (window state, cache) |
| 2 | Installer `AppId` | `{7A3E9C41-5D2B-4E8F-B6A1-9C0D2E4F6A8B}` | Uninstall identity: change it and Windows sees a second product |
| 3 | Settings dir | `%APPDATA%\selora-ide` | Every setting, including the plaintext `.api_key` fallback |
| 4 | Keyring service | `selora-ide` (`api-key`, `brave-key`) | Every stored API key becomes invisible |
| 5 | Env vars | `SELORA_API_KEY`, `SELORA_WORKSPACE_ROOT`, `SELORA_SETTINGS_DIR`, `SELORA_ISCC` | Dev shells, CI, release pipeline |

The table hides two more data contracts, and both fail as **silent resets** rather than errors: the
settings file's **top-level JSON key is literally `selora`** (`settings.rs:135` —
`pub selora: SeloraSettings`), and **eleven `localStorage` keys** hold the frontend's state. See
§16.3.

### 16.3 Migration rules (read-old → write-new → delete-old)

- **Settings:** if `%APPDATA%\skarn` is absent and the old dir exists, copy the tree and write
  `migrated_from` + timestamp. Keep the old dir readable for exactly one release, then stop.
- **Keyring:** read old service → write new service → **only after a successful write**, delete the
  old entry. On a failed read, say so once ("your saved key could not be carried over") instead of
  showing a connected-looking app with no key. Never leave a plaintext copy behind.
- **Identifier:** copy the old Tauri data dir on first launch; accept a one-time loss of window
  geometry, never a loss of settings.
- **Installer:** keep the `AppId` GUID and the current `DefaultDirName` for 1.0 so the upgrade is
  in place with one uninstall entry. Move the folder in **1.1** with an explicit `[InstallDelete]` —
  changing the path in the same release strands the old files, because Inno does not delete them.
- **Env vars:** add `SKARN_*`, keep reading `SELORA_*` for one release with a deprecation log, drop
  the aliases in 1.1.
- **The settings file's key, not just its folder:** the JSON is `{"selora": {…}}`. Copying the
  directory does nothing if the new build looks for a new key — deserialization fails and the whole
  profile resets. Rename the field, put `#[serde(alias = "selora")]` on the new name for one
  release, keep writing the new key, drop the alias in 1.1. This is the single most damaging item
  in the list, because it presents as "it forgot my theme" rather than as a migration failure.
- **Eleven `localStorage` keys:** `model_settings`, `agent-pinned-sessions`, `agent-draft:*`,
  `editor-session:*`, `last-editor-workspace`, `editor-preferences`, `recent-commands`,
  `agents-workspace:v1:*`, `ui-sounds`, `notification_history`, `onboarding_seen`. Copy on first
  read, leave the old key for one release, and do it table-driven — three names are parameterised
  and contain a path or JSON that must survive verbatim. Letting `onboarding_seen` reset re-onboards
  every existing user.
- **Five event channels:** `selora://event`, `selora://agent-preferences`,
  `selora://agents-navigate`, `selora://main-session-navigate`, `selora:update-progress` — emitted
  in Rust, listened to in JS, eight sites. Rename them in **one commit**; a partial rename does not
  error, the listener simply never fires, so the event log, preference sync and agents-window
  navigation go silent while the app still builds.

### 16.4 Removing the gateway (the de-brand half)

Gone: the preset `id: 'selora'` ("Selora API Gateway"), `https://api.selora.lol`, the `sk-selora-` /
`sk-gw-` validation and placeholders, both defaults (`useState('selora')` and the base URL),
`provider-visibility.ts:37`, the `'selora'` / `selora-fast` test fixtures, the `'gateway'` settings
tab id, the `'Recommended'` tag, and the copy: "Model Provider & Gateway Connection", "Gateway Base
URL", "Gateway Connected", "Testing Gateway…", "Settings > AI Gateway", "Appended to Selora's
system prompt", and the status bar's "selora connected".

Replaced by: **no preselection** and four cards (Anthropic · OpenAI · Local/Ollama · Custom
endpoint); "Bring your own key. Keys stay in Windows Credential Manager."; OpenRouter re-tagged
`'Aggregator'` so "gateway" stops meaning two things; the model list comes from the provider's
`/models` with a custom-id escape hatch. The CSP needs no change — provider calls are Rust-side —
but `api.github.com` stays listed only while the update check exists.

Product judgement, stated once: deleting a preset is a first-run redesign, not a deletion. A hole
in onboarding is worse than the preset was.

### 16.5 New repository bootstrap (owner is creating it)

1. Create `<owner>/skarn`, private first. README line 2: "This project began as Selora IDE; the
   original copyright notice is retained in LICENSE."
2. `[workspace.package] repository` in the root `Cargo.toml` is **the single source**; the updater
   already reads `CARGO_PKG_REPOSITORY`. Then delete the hardcoded copies: `state.rs:6055` and
   `:6058`, `App.tsx:94`, `AgentsWindowApp.tsx:177`, `package.json` `repository.url`, the installer
   `RepoUrl`. CI greps for a second occurrence.
3. Release assets `Skarn-Setup-<version>.exe` + `.sha256`; matcher at `updates.rs:149` and its
   fixtures at `:397-453` change in the same commit as the installer.
4. Secrets under new names (`SKARN_SIGNING_CERT_BASE64`, `SKARN_SIGNING_CERT_PASSWORD`,
   `TAURI_SIGNING_PRIVATE_KEY`); no reuse of `selora-signing.pfx`; the old certificate must not sign
   the new brand. **Start OV/EV issuance in P0** — it takes days to weeks.
5. No old releases imported, no old tags copied. First tag `v1.0.0` (also guarantees the updater
   never prefers an old `0.x`).
6. Branch protection with required checks (cargo test, typecheck, npm test, build) so a half-landed
   rename cannot merge.
7. Point the docs host at the About dialog, `F1`, and the installer URL — three separate hardcoded
   strings today.
8. Archive the old repo after 1.0.0. If the owner no longer controls it, P0 is urgent, not tidy.

### 16.6 Work order

| Phase | Work | Gate |
|---|---|---|
| **P0** (1d) | Repoint/disable updater, delete hardcoded URLs, rename asset matcher, gateway off, cert started | Built binary has no `api.selora.lol` and no old slug; fixture where old repo publishes `9.9.9` produces **no** update offer |
| **P1** (0.5d) | Clearance run, freeze tokens, write `NAME_DECISION.md` | Signed table with date + chosen identifier |
| **P2** (2–3d) | Mechanical rename: `git mv` crates → module paths → public types → binary → frontend ids → lockfile | `cargo check/test --workspace`, typecheck, `npm test`, build; no logic diff |
| **P3** (2–3d) | Identity + **every** migration: settings dir **and its `selora` JSON key alias**, keyring, identifier, installer, env vars, 11 `localStorage` keys, 5 event channels, theme id `selora-dark` | Upgrade 0.1.0 → 1.0.0 on a clean VM: settings kept with no reset, key kept, pinned sessions and open files restored, one uninstall entry, no old folder |
| **P4** (1–2d) | All copy, docs, icons, new palette, CI artifact names, signing | `grep -ri selora` = LICENSE + CHANGELOG only; signed installer builds |
| **P5** (1d) | Ship 1.0.0, archive old repo, CHANGELOG "renamed from Selora IDE", delete `.zcode/` and the stray `nul` | Fresh + upgrade install both pass §16.7 |

**Mechanism correction for P0 — the wrong model leads to the wrong fix.** There is no Tauri updater
in this app: `tauri.conf.json` has `bundle.active: false` and no `plugins.updater` block, so no
pubkey and no signature verification. `updates.rs` is a custom client — GitHub API → `tag_name`
version compare → the `Selora-IDE-Setup-<tag>.exe` asset plus its `.sha256`, **both from the same
release**. A checksum that travels with the binary proves integrity, not authenticity: whoever
controls the release controls both files. Windows Authenticode on the installer is the only
remaining gate, and by default it is the old publisher's certificate that opens it. Treat the
existing update path as untrusted; fix it by repointing, not by trusting the hash.

### 16.7 The ten tests that matter

Fresh install clean of the old name · upgrade keeps settings **and browser state** (JSON key
aliased, 11 `localStorage` keys copied, no re-onboarding, pinned sessions and open files restored) ·
keyring migrated and old entry removed · settings migrated with marker · updater ignores an
old-repo `9.9.9` · binary strings clean · repo grep clean · gateway absent and Ollama works offline
with nothing preselected · uninstall leaves no orphan install · signature valid and issued to the
new publisher.

### 16.8 Risks, and the one ownership conflict

Highest: old repo still reachable by old builds (remote code execution — the only catastrophic one)
· settings-file key renamed without an alias, which resets every profile and gets reported as data
loss · `localStorage` keys or event names renamed piecemeal, which loses pinned sessions and open
files with no error at all · identifier change orphans webview state · installer GUID/folder
mismatch duplicates the app · keyring service change makes users appear disconnected · certificate
lead time blocks 1.0 signing.

**Conflict with Part IV:** `tokens.css` §1.8 is an explicitly brand-derived palette ("Selora
Monochrome Space System", `--space-*`, `--moonlight-*`). The rebrand replaces it, which removes half
the two-lighting-models problem — so one owner must hold `tokens.css` and the two tracks must not
edit it in parallel.

---

*Nothing here is a substitute for running the app. The dossier's strongest claims are structural
and checkable with `grep`; its weakest are about felt quality, which only a recording or a user
test can settle.*
