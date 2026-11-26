# Selora IDE — UX Pain Point Audit

> A forensic account of why the product feels sloppy, jumbled and unorganised — with the
> exact code that causes each feeling, ranked, plus the deletion list and the target model.
>
> **No implementation code.** This is diagnosis and design direction.
>
> Companion document: `ZCODE_UX_TEARDOWN.md` (what to borrow). This one is about *us*.

---

## 0. How to read this

**Evidence standard.** `H` = read directly in source, cited `file:line`. `M` = inferred from
two or more verified facts. `L` = design judgement. I did not run the app; every claim below
is reachable by reading the files named. Where a claim depends on runtime behaviour I could
not observe, it says so.

**Severity.** `P0` = a user does the wrong thing or trusts something false. `P1` = daily
friction or hunting. `P2` = craft debt that keeps generating P0s.

**The three words, defined.** They are not one problem:

| Word | Actual defect | Test that exposes it |
|---|---|---|
| **Jumbled** | A single flat surface list is routed into different containers by mutable state, so an icon's destination doesn't match its position | Click every icon in the left rail and watch *where* the panel appears |
| **Unorganised** | The same concept has several homes and several names — config, sessions, "new", todo state | Ask "where do I change X?" for ten Xs; count distinct answers per concept |
| **Sloppy** | The UI asserts things that are not true, and shows text meant for developers | Ask of every pixel: "is this live, or is it decoration?" |

---

## 1. The deeper cut

**(1) What a top professional would do here that wasn't asked for.**
Not a redesign. They would build an **inventory table** first — every surface, every route,
every state, every string — and then find the places where two entries claim the same thing.
They would also refuse to accept "it feels jumbled" as a brief: they'd convert it into
countable facts (how many homes does the model setting have? how many buttons create a
session?) because that is the only form in which "jumbled" can be fixed without a rewrite.
And they would delete before they design. This product does not need a new surface; it needs
five fewer.

**(2) The rawest material.**
Not the design docs (`DESIGN §3.2` etc. describe intent, and intent is what failed). The raw
material is: the **routing function** (`state.rs::apply_toggle`), the **surface union**
(`bridge.ts` `SurfaceId`), the **DOM** (three naming prefixes on one element), the **CSS
census** (28 files, 20,871 lines), and **one screen recording of a real session** with the
user's mouse path drawn on it. Plus the strings themselves: grep every label and you find the
org chart of the product.

**(3) Methods and tools.**
Cognitive walkthrough on five jobs (change the model, start parallel work, find why a build
failed, approve a write, get back to the file I was editing); a **route census** (surface ×
container × trigger); a **duplication count** per concept; a **truthfulness pass** (every
displayed value: is it derived from live state?); a **string audit** (labels, tooltips,
error copy, developer leakage); **keystroke-level counting** on the three most frequent
flows; and Nielsen's heuristics applied to the *shell*, not the features.

**(4) What would show the result is right, other than the two of us.**
A stranger clicks all six rail icons and never once looks surprised. A user asked "where do I
change the model?" points at one place, and it is the same place the next user points at. A
grep returns exactly one label per concept. A screen recording contains no dead click. The
status bar contains no value that fails to change when the underlying thing changes. And the
CSS line count goes **down** while the app looks better.

---

## 2. The diagnosis in one paragraph

Selora has a strong editor core and a genuinely capable agent runtime wrapped in a shell that
was assembled by accretion. The shell's central data structure is a **flat list of nine
surfaces** which three different containers render depending on mutable layout state
(`apply_toggle`, `state.rs:4927`). Because the list is flat, the left rail sometimes opens a
panel at the bottom, one rail button can never light up, and two of its labels describe views
that container can never show. Because there is no ownership rule, the same concepts
accumulated multiple homes: **three** places to configure a provider, **two** to pick a model,
**two** to set reasoning effort with different words, **five** ways to start a session,
**four** ways to display the same todo list, **two** non-interchangeable "Editor" buttons, and
**two** meanings for `Ctrl+B` depending on which window you're in. And because the shell
displays a lot of static decoration as if it were live state — a Git sync that syncs nothing,
a hardcoded `Ln 1, Col 1`, a `UTF-8` that is always `UTF-8`, a hardcoded `TypeScript`, a
`core <hash>` — the product reads as *unfinished* even where it is fully working. None of
this is a feature gap. All of it is fixable by subtraction, naming, and routing.

---

## 3. Fault 1 — One flat surface list, three containers (the "jumbled" fault)

**The mechanism.** `SurfaceId` is one union of nine values (`bridge.ts:44`):
`files, search, git, problems, settings, agent, output, terminal_placeholder, workflows`.

`apply_toggle` (`state.rs:4927`) routes them by a match:

| Surface | Container it opens | Toggling it again |
|---|---|---|
| `files`, `search`, `git`, `workflows`, `settings` | Primary sidebar (left) | closes the sidebar |
| `agent` | Agent dock (right) | closes the dock |
| `problems`, `output`, `terminal_placeholder` | Bottom dock (below editor) | closes the bottom dock |

**What that produces, concretely:**

| # | Symptom the user sees | Cause | Evidence | Sev |
|---|---|---|---|---|
| J1 | The fourth icon in the **left** rail opens a panel at the **bottom** | `problems` is an activity-bar item but routes to the bottom dock | `ActivityBar.tsx:41`, `state.rs:4938` | **P0** |
| J2 | That icon **never shows as selected** — clicking it looks like nothing happened in the rail | selected state requires `activity_view === 'problems'`, but the toggle writes `bottom_view` | `ActivityBar.tsx:227` vs `state.rs:4938` | **P0** |
| J3 | `Problems` exists in the rail **and** as a bottom tab, with different surrounding chrome | two homes for one surface | `PrimarySidebar.tsx:46`, `BottomDock.tsx:26` | P1 |
| J4 | The sidebar has labels for `Output` and `Terminal` that it can never display | `VIEW_LABEL` covers all nine; only six routes exist | `PrimarySidebar.tsx:41-48` | P1 |
| J5 | A third session list ships but is unreachable | sidebar `agent` route renders `AgentSessionRail`, but `toggle('agent')` opens the dock, so `activity_view` is never `'agent'` | `PrimarySidebar.tsx:223`, `state.rs:4935` | P1 |
| J6 | `Ctrl+B` means "left sidebar" in the main window and "Agents navigation rail" in the Agents window | two keymaps, same chord, different targets | `commands.ts:399`, `AgentsWorkspaceHeader.tsx:47` | P1 |
| J7 | Settings opens in a ~300 px column and vanishes if you close the sidebar | settings is a sidebar *route*, not a surface | `SettingsPanelView.tsx` mounted at `PrimarySidebar.tsx:222` | **P0** |

**Root cause.** Surfaces were modelled as *views* (things you can show) instead of *content
types* (things that belong somewhere). The container decision lives in a match arm, so the
information architecture is literally a `switch` statement. Every new view adds a new arm and
a new possible misroute. This is why the app feels jumbled even though each individual panel
is fine.

**The fix, one rule:** a region owns a *kind* of content, and a rail icon never opens a
different region. Left = project (files, search, git). Bottom = process (terminal, problems,
output, runs). Right = task (agent). Settings = a real modal/page, not a narrow column.
Delete the sidetracked routes rather than supporting both.

---

## 4. Fault 2 — The agent has five navigations, three homes, five "new" buttons

**Navigations inside one task surface** (`AgentConversationSurface.tsx`):

1. `AgentSessionTabs` — tab strip of sessions (`:259`)
2. `AgentSessionRail` — a second, collapsible session list behind a text button (`:279-284`)
3. `Chat | Plan (n/m)` view tabs (`:174-190`)
4. A floating plan popover toggle (`:193-205`)
5. `SubagentSessionView` — opening a child swaps the body (`:294`)

Plus, in the Agents window, a **navigation rail** with conversations/automations/customizations
(`AgentsNavigationRail.tsx:80-103`) and a header with back/forward history
(`AgentsWorkspaceHeader.tsx:44-52`).

**Five ways to start a session**, two labels:

| Affordance | Label | Evidence |
|---|---|---|
| `+` in session tabs | icon only | `AgentSessionTabs.tsx:120` |
| Button in nav rail | "New Session" | `AgentsNavigationRail.tsx:89-92` |
| Agents window command | `Ctrl+N` | `commands.ts:106` |
| Palette entry | "Agents: New Session" | `commands.ts:106` |
| Button inside session rail | (internal) | `AgentSessionRail.tsx:56` |

**Four renderings of the same todo state** in one view: the `Plan (n/m)` tab label, the infeed
`Tasks (n/m)` progress bar (`:317`), the expandable `AgentPlanSpine` (`:335`), and the
floating plan popover (`:311`). All read `state.todos`.

What the user experiences: a chat panel with a tab strip, a hidden second session list, two
view tabs, a plan popover, a progress bar, and a plan tab — six controls competing to answer
"what is the agent doing and where am I?" The agents window then repeats the whole thing with
different chrome.

| # | Symptom | Cause | Sev |
|---|---|---|---|
| A1 | Six controls for one question ("where am I / what's running") | each slice added its own affordance instead of extending an existing one | **P0** |
| A2 | Session list exists twice in the same surface, one hidden behind a text button that reads like a card | navigation smuggled in as body content | P1 |
| A3 | The empty state differs by window: "Summarize the workspace" only appears in the Agents window | suggestions filtered by `presentation` with no explanation | P2 |
| A4 | Two buttons say "Editor" and do different things (close the dock vs return to the main window) | same word, two semantics | P1 |
| A5 | Eleven elements in one header row — eight of them interactive (orb, "Agent", session badge, open-window, armed badge, Chat, Plan (n/m), plan toggle, Stop, Editor, Split) | no hierarchy; everything is a peer | **P0** |
| A6 | Background work is announced in the **title bar** (pets) and inspected in the **feed** (results list) | ambient surface ≠ inspection surface | P1 |

---

## 5. Fault 3 — Configuration has four homes and three vocabularies

| Concept | Homes | Evidence |
|---|---|---|
| Provider + API key | **3**: Connect modal, Settings → Gateway, Customizations → Assistant providers | `App.tsx:174`, `SettingsPanelView.tsx:43`, `AgentsCustomizationsView.tsx:14,124` |
| Model | **2**: composer model menu, Customizations → Model | `AgentComposer.tsx` model menu, `AgentsCustomizationsView.tsx:99` |
| Reasoning effort | **2**, with **different labels for the same value**: `Max` vs `Maximum` | `AgentComposer.tsx:106-111` vs `AgentsCustomizationsView.tsx:111` |
| Instructions | **2**: Customizations textarea (global, `agent-preferences.json`) and… nothing for the project | `AgentsCustomizationsView.tsx:112` |
| Keybindings | 1, but only **4** are rebindable | `SettingsPanelView.tsx:63-80` |
| Theme | 1, with a legacy alias accepted in the model (`space-black` → dark) | `SettingsPanelView.tsx:33-38` |
| Disabled providers | 1 (Customizations), invisible from the composer that filters by it | `AgentsCustomizationsView.tsx:34` |

Worse, settings contains controls that exist but cannot save, each carrying a tooltip that
explains the protocol gap to the user:

- `unexposedTitle` → "stored in settings.json, but this build's settings protocol does not expose it… changes are not saved" (`SettingsPanelView.tsx:83`)
- `unsupportedTitle` → "has no writable backend settings field in this build…" (`SettingsPanelView.tsx:90`)

**This is the single most "unorganised" thing in the product**: a settings page that
advertises its own incompleteness in a hover tooltip. A user cannot tell which of the 40+
controls are real. The honest move is to *remove* the non-functional ones until they work
(a disabled control is a promise; a disabled control with an apology is worse than absence).

**"Customizations" is the second problem.** It is a whole page inside the Agents window
holding model, effort, instructions and provider visibility — i.e. *configuration*, living in
the place people go to *converse*. Nobody will look for it there, and its instructions field
is a global blob that competes conceptually with the missing `AGENTS.md`.

---

## 6. Fault 4 — The UI asserts things that are not true (the "sloppy" fault)

This is what makes a working app read as unfinished. Every item below is a visible pixel that
does not describe reality.

| # | What the user sees | What is actually happening | Evidence | Sev |
|---|---|---|---|---|
| T1 | Click the branch name → spinner for 1.2 s | **No network, no command.** `setSyncing(true)`, `setTimeout(…1200)`, `setSyncing(false)` | `StatusBar.tsx:119-125` | **P0** |
| T2 | `Ln 1, Col 1` | hardcoded string, never updates with the cursor | `StatusBar.tsx:190` | **P0** |
| T3 | `UTF-8` | hardcoded string | `StatusBar.tsx:186` | P1 |
| T4 | A language name (often `TypeScript`) | falls back to the literal `'TypeScript'` when no buffer is open | `StatusBar.tsx:88-92` | P1 |
| T5 | `core 3f9a1c2e` | internal instance id exposed as if it were product state | `StatusBar.tsx:193` | P2 |
| T6 | A bottom panel labelled **Output** with a badge **"Not wired yet"** | ships a permanently unfinished surface in the tab strip | `OutputPanel.tsx:1,15` | **P0** |
| T7 | Route named `terminal_placeholder` + a `.terminal-placeholder-*` CSS family | an internal placeholder concept became permanent identity | `bridge.ts:44`, `dock.css:995-1031` | P1 |
| T8 | "The Workflows panel is not available in this build… `src/features/workflows/WorkflowsPanel.tsx` ships with the Phase 8 UI slice" | a **source path and phase name** shown to users | `PrimarySidebar.tsx:143-153` | **P0** |
| T9 | Tooltip: "Settings editing lands in a later slice" | build-planning language in a product tooltip | `StatusBar.tsx:64` | P1 |
| T10 | `selora connected` in lowercase next to `Selora IDE` | brand inconsistency in the most-glanced-at corner | `StatusBar.tsx:47` | P2 |

**Why this matters more than it looks.** Users calibrate trust from the cheapest signals.
One fake spinner teaches them that indicators are decoration; after that, the approval badge
and the unsaved-changes dot are also "probably decoration". The cost of T1 is not one wasted
click — it is every future status the product ever shows.

---

## 7. Fault 5 — Feedback is a firehose of 4-second toasts

- **Dozens of call sites** push local toasts (`toastStore.pushLocal`), from clipboard failures
  to protocol errors to "Cargo build finished".
- At most **3** are visible at once (`TOAST_MAX_VISIBLE = 3`, `toast-store.ts:18`) and each
  auto-dismisses after a severity-based timer (`toast-store.ts:59,224`).
- A **history** exists, but only behind the bell in the title bar (`NotificationCenter.tsx`).
- Meanwhile, genuinely blocking conditions are delivered as toasts:
  - "Editor changes are still unsynchronized. Keep this file open until the workspace reconnects." (`editor-store.ts:481`)
  - "Cannot close this file while editor changes are unsynchronized." (`:489`)
  - "The close result is unknown. This editor remains locked to protect its contents." (`:284`)

A condition that **locks a file** and requires the user to *keep it open* is not a toast. It
is a persistent state that should live on the tab, in the Problems panel, and in a status
area — the three places a user looks when something is wrong. Delivering it as a 4-second
banner means the user learns the symptom but loses the instruction.

**Error copy is engineer-copy.** "The close result is unknown" describes our uncertainty, not
the user's situation. The rule for this class of message: *what happened, what is safe right
now, one action.* Everything else is for the log.

---

## 8. Fault 6 — The visual system is layered patches, not a system

| Metric | Value | Why it matters |
|---|---|---|
| CSS files | 28 | 15 global in `styles/`, 13 feature-local |
| CSS lines | **20,871** | ~200 lines of CSS per `.tsx` file (104 tsx) |
| Files literally named `*polish*` | 3 (`agent-card-polish.css`, `agent-feed-polish.css`, `agent-session-tabs-polish.css`) | each is a second pass layered on the first pass instead of editing it |
| `!important` in `shell.css` alone | 20 | specificity war inside one file |
| Naming prefixes on the same element | up to 3: `agent-session-tabs-strip` **and** `agent-sessiontabs-strip`; `agent-decision-*` **and** `agent-card-*`; `zcode-*` for parity work | you cannot know which class actually styles a node |
| Inline `style={{…}}` in shell components | present in `PrimarySidebar` fallbacks, `AgentConversationSurface` (chevron transform), `StatusBar` (connection button) | a third styling strategy inside the same surface |

**The craft layer is specified in `SELORA_CRAFT_SPEC.md`** — it adds the measured contradiction in `tokens.css` (a constitution banning gradients/glass/shadows above 80+ uses of `--glow-*`, `--glass-*` and `--grad-*`), 75 `transition: all` occurrences, two competing radius families, and the motion policy.

**The "polish" naming is the tell.** A file named `*-polish.css` is a confession: the first
file was shipped before the design was decided, and the second file overrides it. Three of
those means the *decision* never happened. The visible result is the thing users call
"sloppy": 2 px inconsistencies, hover states that differ between two cards, radius values that
look close but aren't, and a light theme that lags the dark one.

**What to do:** not a redesign. One pass of *consolidation* — collapse each `*-polish.css` into
its base file, delete the losing rules, remove the duplicate class aliases on the same element,
move inline shell styles into the owning stylesheet, and enforce a token allowlist via a
stylelint rule. The line count should fall by ~20-30% with **zero** visual change, and that
alone will make future work faster.

---

## 9. The first sixty seconds (what a new user actually meets)

1. **Boot** → "Connecting to Rust core…" (internal language, but acceptable).
2. **Onboarding**, three cards: "The editor born connected…" → then it's gone forever. It
   never mentions the agent dock, the second window, or that `Ctrl+L` exists. The three cards
   teach the *manifesto*, not the *map*.
3. **Empty shell**: a 48 px rail with six icons, a title bar with a search pill, a workspace
   pill, a three-way view switcher (`agent/split/editor`), a bell, a status bar with a
   `core 3f9a1c2e` and a fake `Ln 1, Col 1`.
4. **They click the fourth icon** (the error glyph): a panel appears at the bottom, the icon
   stays unlit. They click it again; the panel closes. They are now unsure what the rail is for.
5. **They open Settings** with `Ctrl+,`: a narrow column with four tabs, some controls
   disabled with tooltips explaining that a protocol doesn't expose them. They cannot tell if
   the app is broken or unfinished.
6. **They press `Ctrl+L`**: the agent dock opens with an orb, "Agent", a session badge, two
   view tabs, a plan progress bar, and a composer with a three-button mode row. "Agent" is one
   of the modes. The thing they just opened is also called Agent.
7. **They send a prompt.** A card appears asking for approval. They click Accept. Another card
   appears. By the fifth they stop reading.
8. **They click the `⧉` icon** in the agent header: a *second window* opens, titled "Agents",
   with a rail, a "New Session" button, a Context pane, and a slightly different empty state.
   The same session is now open twice. Scroll one; the other doesn't move.
9. **They go back to the main window** and press `Ctrl+B`: the sidebar closes. In the other
   window, `Ctrl+B` toggles a rail. Same chord, two meanings.
10. **They try to find the model setting**: composer menu, or Customizations, or Settings →
    Gateway. Three answers, all correct.

Nothing in those sixty seconds is a *missing feature*. Every step is a coordination failure.
That is the whole argument of this document in one walkthrough.

---

## 10. The pain register (ranked)

Consolidated, deduplicated, and ordered by "how much trust does fixing this buy".

### P0 — fix before adding anything (11)

| ID | Pain | Evidence | Change |
|---|---|---|---|
| P0-1 | Rail icon opens a panel in another region | `state.rs:4938` | Region owns content kind; rail never opens the bottom |
| P0-2 | That rail icon can never appear selected | `ActivityBar.tsx:227` | Remove Problems from the rail (it belongs to the bottom dock) |
| P0-3 | Git "sync" spins and does nothing | `StatusBar.tsx:119` | Wire it or delete it this week |
| P0-4 | `Ln 1, Col 1` is a hardcoded lie | `StatusBar.tsx:190` | Bind to the real cursor, or remove |
| P0-5 | "Output" ships with a "Not wired yet" badge | `OutputPanel.tsx:15` | Remove the tab until it has content |
| P0-6 | A user-facing error prints a source path and "Phase 8 UI slice" | `PrimarySidebar.tsx:143` | Replace with one sentence + a retry |
| P0-7 | Settings lives in a 300 px sidebar route and closes with the sidebar | `PrimarySidebar.tsx:222` | Promote to a real modal/page |
| P0-8 | Six competing affordances inside one task surface | `AgentConversationSurface.tsx` | One session navigation, one progress display, one plan view |
| P0-9 | Eleven peer elements in one header row (eight interactive); two say "Editor" | `AgentConversationSurface.tsx:120-250` | Three groups: identity, run controls, view switch |
| P0-10 | Provider config in three places; model in two; effort in two with different words | §5 | One config surface, one vocabulary |
| P0-11 | Modes have two vocabularies; "Agent" is the permissive one | `types.ts:17`, `AgentComposer.tsx:87,122` | Four honest names, one set |

### P1 — daily friction (12)

| ID | Pain | Evidence |
|---|---|---|
| P1-1 | `Ctrl+B` means different things in the two windows | `commands.ts:399` vs `AgentsWorkspaceHeader.tsx:47` |
| P1-2 | Five ways to start a session, two labels | §4 |
| P1-3 | A hidden second session list behind a text button | `AgentConversationSurface.tsx:279` |
| P1-4 | Dead sidebar routes and labels for views it can't show | `PrimarySidebar.tsx:41-48,223` |
| P1-5 | Same session open in two windows with independent scroll/draft | `openAgentsWindow` from dock header |
| P1-6 | Blocking file state delivered as a 4-second toast | `editor-store.ts:481,489` |
| P1-7 | Dozens of toasts, 3 visible, history buried in a bell | `toast-store.ts:18` |
| P1-8 | `UTF-8` hardcoded; language falls back to `TypeScript` | `StatusBar.tsx:88,186` |
| P1-9 | "Customizations" is configuration living in the conversation window | `AgentsNavigationRail.tsx:97` |
| P1-10 | Settings ships non-savable controls with apologetic tooltips | `SettingsPanelView.tsx:83,90` |
| P1-11 | `terminal_placeholder` is a permanent, visible identity | `bridge.ts:44` |
| P1-12 | Duplicate class names and 3 naming prefixes per element | `AgentSessionTabs.tsx:52`, `AgentQuestionCard.tsx:47` |

### P2 — craft debt that regenerates P0s (8)

| ID | Pain | Evidence |
|---|---|---|
| P2-1 | 20,871 CSS lines; 3 `*-polish.css` files | census |
| P2-2 | 20 `!important` in `shell.css` | census |
| P2-3 | Inline styles inside shell components | `PrimarySidebar` fallbacks, `StatusBar` |
| P2-4 | Onboarding teaches the manifesto, not the map | `OnboardingOverlay.tsx:16-31` |
| P2-5 | Subagent status in the title bar, details in the feed | `TitleBar.tsx:339`, `SubagentResults.tsx` |
| P2-6 | `core <hash>` in the status bar | `StatusBar.tsx:193` |
| P2-7 | Empty-state suggestions differ per window, unexplained | `AgentConversationSurface.tsx:102-108` |
| P2-8 | Lowercase `selora connected` vs `Selora IDE` | `StatusBar.tsx:47` |

---

## 11. Order of operations (and what to delete first)

**Week 1 — remove lies (all P0-3…P0-6, P2-6, P2-8).**
Delete the fake sync, the hardcoded coordinates, the `UTF-8` chip, the `TypeScript` fallback,
the `core` chip, the "Not wired yet" tab, and the phase-name error text. Nothing is added.
The product instantly stops lying, and every later change inherits credibility.

**Week 2 — fix routing (P0-1, P0-2, P0-7, P1-4).**
Establish the rule "a region owns a content kind". Move Problems out of the rail. Promote
Settings to its own surface. Delete the dead routes and their labels. This is the change that
makes the app stop feeling jumbled, and it is mostly deletion.

**Week 3 — collapse the agent surface (P0-8, P0-9, P1-2, P1-3).**
One session navigation (chooser in the header), one progress display (the plan tab), one
"new" affordance per window, delete the floating plan toggle and the session-history button.
Group the header into identity / run / view. Two "Editor" buttons become one word, one meaning.

**Week 4 — one configuration home (P0-10, P0-11, P1-9, P1-10).**
One surface for provider, model, effort, instructions. Delete the duplicate menus. Remove
non-savable controls. Choose four honest mode names and delete the second vocabulary.

**Week 5 — truth and feedback (P1-6, P1-7, P1-8).**
Bind status values to live state. Move blocking states from toasts to persistent surfaces.
Introduce one error pattern: what happened / what is safe / one action.

**Week 6 — craft consolidation (P2-1…P2-3).**
Collapse `*-polish.css` into base files, delete duplicate class aliases, move inline shell
styles, add a token allowlist rule. Target: −25% CSS lines, zero visual diff.

**Do not** start any new feature (skills, goal mode, browser, MCP) before Week 3 lands. Every
one of them would add a surface to a shell that cannot yet say where things live.

---

## 12. The target model, in one page

**Regions own content kinds, permanently.**

| Region | Owns | Never contains |
|---|---|---|
| Left rail + sidebar | Project: files, search, git | Problems, output, terminal, agent |
| Bottom dock | Process: terminal, problems, output, runs | Files, search, git, agent |
| Right dock | Task: one conversation, its plan, its approvals | Project browsing, settings |
| Second window ("Agents") | The *same* task surface, expanded, sharing state | A second session list or a second palette |
| Settings | All configuration, as a real page | Anything else |
| Command center (`Ctrl+K`) | Every destination, as text | Duplicated controls |

**One home per concept.**

| Concept | Home | Forbidden duplicates |
|---|---|---|
| Provider / key | Settings → Providers | Connect modal (becomes a shortcut *to* it), Customizations |
| Model / effort | Composer (the thing you change mid-thought) | Customizations, Settings |
| Instructions | `AGENTS.md` + rules (project), Settings → Global (user) | Customizations textarea |
| New session | One per window, one label ("New task") | `+`, rail button, palette entry, internal button |
| Session list | One per window | Tabs *and* rail *and* history button |
| Plan/todos | One display, in the task | Tab label + infobar + popover + spine |
| Errors | One pattern, persistent when blocking | Toasts for blocking states |

**Status bar policy:** every value must be live, or absent. No decoration. No internal ids.
No lowercase brand.

**Vocabulary:** one row per concept, delete the losers (see `ZCODE_UX_TEARDOWN.md` Appendix B;
this audit adds: "New task" not "New Session"; "Providers" not "Gateway"/"Assistant providers";
"Reasoning" one label, not Max/Maximum).

---

## 13. Delete list (the fastest quality win available)

1. The Git sync button (or its fake handler).
2. `Ln 1, Col 1`, `UTF-8`, `core <epoch>` from the status bar.
3. The `TypeScript` default language.
4. The **Output** tab and `OutputPanel` until it has content.
5. The `Problems` item in the activity bar.
6. Sidebar `VIEW_LABEL` entries for `problems`, `output`, `terminal_placeholder`, `agent`.
7. The sidebar `agent` route and `AgentSidebarView`.
8. The `Session history` text button and its embedded rail.
9. The floating-plan toggle and the infeed progress bar (keep the Plan tab).
10. The duplicate permission-mode menu; the `Full auto-apply / Armed / Guarded` vocabulary.
11. Connect modal as a *separate* configuration path (make it a deep link into Settings).
12. Customizations' model/effort/provider sections (keep instructions, moved to Settings).
13. Every settings control backed by `unexposedTitle` / `unsupportedTitle`.
14. The three `*-polish.css` files (merge, don't layer).
15. The duplicate class aliases on the same element.
16. `.zcode/` dev scratch from the repo root (move to `tools/`), and the stray `nul` file.

---

## 14. What not to do

- **Do not redesign.** The tokens, spacing and type scale are fine; the *placement* is not.
- **Do not add a "workspace hub", "home", or "dashboard".** The command center is the home.
- **Do not unify the windows by embedding one in the other** before the state is shared; you
  would ship two copies of the same bug.
- **Do not fix the CSS by adding overrides.** The `*-polish.css` pattern is exactly that, and
  it is the reason the paint looks uneven.
- **Do not keep both vocabularies "for compatibility".** There is no compatibility burden —
  the product is pre-1.0 and the strings are internal.

---

## 15. Acceptance tests (observable, by someone who didn't write the code)

1. **Rail test.** Click each rail icon in turn. Every panel appears to its right; every icon
   lights up; no panel appears below.
2. **Control test.** Click every control that looks interactive. Each one changes something,
   opens something, or is visibly absent. Zero dead clicks.
3. **Truth test.** Change the cursor position, the file, the branch, the workspace. Every
   status value that claims to describe them changes. No value is a constant.
4. **One-home test.** Ask three people "where do I change the model?" — same answer, ≤2 clicks
   from anywhere, and the other two old locations no longer exist.
5. **Label test.** Grep the UI strings for each concept in §12. Exactly one label per concept
   survives; the removed ones return nothing.
6. **Window test.** Open the same task in both windows, scroll one, type in the other. State
   matches in both directions, or the second window clearly declares itself a snapshot.
7. **Blocking test.** Force a file into the "unsynchronized" state. The instruction is still
   on screen after 30 seconds, on the tab and in Problems — not only in a toast.
8. **CSS test.** Line count drops ≥20%, duplicate-class lint passes, and a screenshot diff of
   six key states shows no unintended change.

---

## 16. The rewritten prompt (copy-paste)

```text
/plan

Role: product designer + frontend architect doing an audit, not a redesign. No code.

Objective: reduce Selora's shell to one home per concept, using SELORA_UX_PAINPOINTS.md as the
brief. The deliverable is a deletion-and-routing plan, not a feature plan.

Frame the work with these four questions before proposing anything:
(1) What would a senior product designer do here that I am not asking for? Answer: build the
    surface/route/string inventory first, convert "feels jumbled" into counts, and delete
    before designing.
(2) What is the rawest material? Answer: state.rs::apply_toggle, bridge.ts SurfaceId,
    PrimarySidebar VIEW_LABEL, ActivityBar selected-state logic, the CSS census, the literal
    UI strings, and one recorded session with the mouse path.
(3) Which methods? Cognitive walkthrough on five jobs; route census (surface x container x
    trigger); duplication count per concept; truthfulness pass on every displayed value;
    string audit; keystroke-level count on the three most frequent flows.
(4) What proves it worked, other than us? A stranger clicks every rail icon without surprise;
    three people name the same place for "change the model"; grep returns one label per
    concept; no status value is a constant; CSS lines fall >=20% with no visual diff.

Read in this order: SELORA_UX_PAINPOINTS.md; then
apps/selora-desktop/src-tauri/src/state.rs (apply_toggle and the layout DTO only);
apps/selora-desktop/frontend/src/bridge/bridge.ts (SurfaceId + Layout);
layout/{ActivityBar,PrimarySidebar,ShellGrid,StatusBar,TitleBar}.tsx;
features/agent/components/AgentConversationSurface.tsx;
features/agent/components/AgentComposer.tsx (MODE_OPTIONS + COMPOSER_MODES);
features/sidebar/SettingsPanelView.tsx (tab list + the unexposed/unsupported paths);
features/agents-workspace/{AgentsNavigationRail,AgentsCustomizationsView,AgentsWorkspaceHeader}.tsx.

Produce:
A. A surface census table: surface | container(s) it can reach | trigger | selected-state rule
   | can it be reached at all. Flag every row where the container contradicts the trigger.
B. A concept census table: concept | every home | every label | what survives after the plan.
C. A truthfulness table: every displayed value | live or constant | keep/remove.
D. The target region model as an ownership rule (one sentence per region), plus the rule that
   makes future violations impossible to ship.
E. A deletion list, ordered, with the file:line that removes the entry.
F. A routing change list: each toggle arm before/after, and what happens to persisted layouts
   that reference a removed surface.
G. Risk notes: what breaks for existing users (stored activity_view, stored bottom_view,
   muscle memory, the second window), and the migration story for each.
H. Three decisions you are least sure about, each with a one-day experiment.

Constraints: deletion over addition; no new top-level navigation; no visual redesign; keep the
existing tokens; every removed string must have a single replacement or none; do not preserve
both vocabularies.

Stop condition: if a proposed change requires touching more than two shared integration files
at once, split it into a sequencing step instead of a big bang.
```

---

*Method note: repo census at time of writing — 104 `.tsx`, 28 `.css`, 20,871 CSS lines, 3
`*-polish.css`, 20 `!important` in `shell.css`, 9 `SurfaceId` values, 3 containers, 5
new-session affordances, 3 provider-configuration homes. ZCode was not run; Selora was not
run. Findings are source-derived and marked `H`/`M`/`L` where used.*
