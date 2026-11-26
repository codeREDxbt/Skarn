# Selora IDE × ZCode — UX Teardown & Convergence Plan

> Review of ZCode by Z.ai (open source, `zai-org/ZCode`) against Selora IDE as it exists in
> this repo, with a prioritised plan for what to fix, what to copy, what to deliberately
> refuse, and how we will know it worked.
>
> **This document contains no implementation code.** It is a design and product argument.

---

## 1. Method, and what this is not

**Read, not run.** I studied ZCode's product documentation (v3.14 → v4.0 docs, Nov 2026
snapshot), its published changelog, and its repository layout, then read Selora's own
source: the layout shell, the agent composer, tool/approval cards, the agents window and its
navigation rail, the keymap registry, and the workflow views. I did not install ZCode, did
not run Selora, and did not compare pixels. Every claim below is either a doc claim, a code
claim with a path, or explicitly marked as judgement.

**What is verified about Selora** (paths relative to repo root):

- `apps/selora-desktop/frontend/src/features/agent/types.ts:17` —
  `PermissionMode = 'full' | 'plan_only' | 'ask_before_change'`.
- `.../components/AgentComposer.tsx:122` — the visible mode row is a *different* three:
  `'ask' | 'agent' | 'plan'`, mapped at `:147`.
- `.../components/AgentComposer.tsx:87` — the dropdown calls the same states
  “Full auto-apply / Plan only / Ask before changes”.
- `.../components/ToolCard.tsx:1` — the file calls itself “ZCode Parity Tool & Approval Card”.
- `.../components/AgentQuestionCard.tsx` — parks a turn; no timer, no timeout, no
  auto-continue.
- `.../features/agents-workspace/AgentsAutomationsView.tsx:1` — “Automations” renders
  `WorkflowsPanel`. It is a workflow-run list, not an automation surface.
- `.../features/keymap/commands.ts:115` — `selora.showTerminal` targets
  `surface: 'terminal_placeholder'`; `selora.editFindInFiles` is bound to the
  `fuzzy_find` keybind field (`:132`).
- `.../features/agents-workspace/session-navigation.ts:114` — session groups are
  `Pinned / Today / Yesterday / Earlier / Undated`.
- Repo-wide search: **no** `AGENTS.md` / `CLAUDE.md` / `.selora/rules` loader; **no**
  mention/slash/skill grammar in the agent feature; **no** mermaid rendering; **no**
  find-in-conversation; **no** session/project-scoped “always allow”.
- `.zcode/` in this repo contains `cdp-*.mjs` and PowerShell click/screenshot helpers plus
  `plans/`, `workflow-drafts/`, `workflow-runs/`.

**Everything below is opinion, held to the standard of being falsifiable.** Where a claim is
an inference rather than a read, it says so.

---

## 2. Executive verdict

1. **We already have more surface than we have language.** Four doors into the agent, two
   session lists, three names for three permission states that appear under two different
   vocabularies. The product's biggest UX problem is not missing features — it is that the
   same thing is called different things in different places.
2. **ZCode's advantage is structural, not visual.** Its task is the object; the chat is one
   view of it. Ours is the reverse: the chat is the object and the task is inferred. That
   single inversion explains most of the gap list.
3. **Our safety UX currently trains users to turn safety off.** Ask-before-changes only
   offers Accept/Reject, one action at a time, with no “allow this class for this session”.
   The rational response is to switch to Full auto-apply — the least safe option — and stay
   there. ZCode's four scoped decisions (Allow / Always allow / Reject / Always reject) are
   the fix for that specific failure, not decoration.
4. **Two of our visible strings are actively wrong** (“Show Terminal” → placeholder,
   “Automations” → workflow runs), and one is a naming trap (“Agent” is the *most*
   permissive mode, next to a dropdown that honestly calls it “Full auto-apply”).
5. **The cheapest high-trust wins are reversibility and vocabulary**, not new panels. Undo a
   turn, name a mode one way, stop the agent from parking forever, and the product will feel
   a category calmer without a single new capability.

---

## 3. The deeper cut (the four questions worth answering first)

The request asked for a deeper version of the task, not a bigger one. So, explicitly:

**(1) What would a top professional do here that wasn't asked for?**
They would refuse to write a feature list. They would first write the *state model* — every
mode, every wait, every destructive action — and prove that the app can always answer two
questions: *who is waiting, and what reverses this?* They would also instrument the current
build before changing it, because “feels confusing” is not a bug report. And they would
delete two of our four agent entry points before adding a fifth surface anywhere.

**(2) What is the rawest material to start from?**
Not the marketing pages. In order: ZCode's **changelog** (it is a public log of what broke
and what they had to fix — the closest thing to a UX post-mortem), the **source** in
`zai-org/ZCode` (`apps/zcode-cli/`, permission gate, composer mention parser, task store),
the docs' own **“Limits and Notes”** sections (self-admitted failures), their GitHub issues,
and — on our side — `sessions.rs`, `state.rs`, `AgentComposer.tsx`, `ToolCard.tsx` plus one
**screen recording of a real 20-minute session**. Opinions from either of us are the least
raw material available.

**(3) What methods and tools?**
Heuristic evaluation (Nielsen's ten) applied to *both* products on the same scripted tasks;
a cognitive walkthrough of five concrete jobs; keystroke-level counting on the three
highest-frequency flows (send prompt, approve a change, recover from a bad turn); a
**state-table diff** of the permission models; a **vocabulary audit** (grep every
user-visible string and collapse synonyms); a **reversibility audit** (every destructive
action: what is its inverse, and is the inverse reachable?); an **interruption audit** (every
wait classified as user-owned or machine-owned); then severity × confidence scoring before
sequencing.

**(4) What, other than the two of us, would show the result is right?**
A first-time user finishing a three-step task without asking where anything is; a returning
user never touching the mode picker; a keystroke count on the three flows that goes down and
stays down; a grep proving each state has exactly one name; a test proving every destructive
action has a reachable inverse; a transcript with zero machine-authored protocol text visible
to the user; and — the strongest external signal — ZCode's next changelog fixing the same
things we fixed, which would mean we were reading the same raw material rather than guessing.

---

## 4. How ZCode earns its feel — six structural decisions

1. **The task is the primary object.** Sidebar rows carry title, relative time, status dot
   (running / unread / failed), and the task's own `+/-` line count. The chat, the diff, the
   approvals and the branch all hang off one task. Archiving, grouping, pinning and a
   retention window exist because tasks accumulate — the list is designed to survive a
   thousand of them.
2. **The composer is the command line.** Typed grammar: `@` file or folder, `#` past
   conversation, `/` command, `$` skill. A `+` menu offers the same actions for people who
   don't type symbols. Large pastes collapse into an attachment instead of flooding the box.
3. **Modes are a promise, not a setting.** Four of them — Ask before changes, Edit
   automatically, Plan, Full access — cycled with `Shift+Tab`, each stating what it will and
   won't ask about. Risk state stays visible in the toolbar when a permissive mode is on.
4. **Review is a first-class citizen.** Diff review, per-turn **Undo → Reapply** (all-or-
   nothing, with a list of what is and isn't safely restorable), edit the last user message,
   and **Reset chat + files** as one atomic action.
5. **Continuity is engineered, not hoped for.** `/goal` sets an objective with per-round
   verification and explicit iteration dividers; side conversations let you ask a question
   without derailing; fork branches a task from a message; queued messages are editable;
   subagents and commands can run in the background.
6. **Status is ambient, and waits are labelled.** Permission requests pause the turn and
   mark the task as *waiting for confirmation* in the sidebar. Ordinary questions get a
   five-minute countdown and then continue, marked “Unanswered, auto-continued”. The user can
   always tell whether the machine is waiting on them or they are waiting on the machine.

That last one is the quietest and the most important. ZCode's calm is mostly the absence of
*unlabelled ambiguity*.

---

## 5. ZCode surface inventory (the part worth stealing, in one screen)

| Area | What it does | Detail worth copying |
|---|---|---|
| Task list | Grouped / Workspace / Timeline views, custom colour groups, drag, move-to-top, collapse | Per-task `+/-`, status dots, archive with 3/7/14/30-day retention |
| Command Center | `Ctrl+K` | One panel for commands **+ conversations + files**, shortcuts shown inline |
| Composer context | `@ # / $` + `+` menu | Paste-to-attachment threshold; selection → “Add to current task” |
| Modes | 4, `Shift+Tab` | “Edit automatically” is the missing middle we lack |
| Thought level | `Ctrl+T` | Low / High / Max, next to the model name |
| Permissions | 4 decisions | Allow / Always allow / Reject / Always reject (+ session / project scope) |
| Questions | 5-min countdown | Auto-continue, marked in history; switch off to wait forever |
| Review | Per-turn diff | Undo → Reapply, all-or-nothing, with a safety list |
| Edit history | Last user message | Edit + send, plus **Reset chat + files** |
| Goal mode | `/goal` | Per-round verification, iteration dividers, pause/resume, budget stop |
| Side conversation | `/side`, `/btw`, `+` menu | Inherits history, visually blank, throwaway by design |
| Fork | Hover a completed reply | New task from that point; original keeps running |
| Subagents | Built-in + custom | Name, colour, model, thinking effort, tool allowlist, background |
| Skills | `$name`, `SKILL.md` | Metadata budget; import from Claude Code via symlink or copy |
| Commands | `/name` | Markdown + frontmatter, `$ARGUMENTS`, tool allowlist per command |
| Plugins | Marketplace | Bundles skills + commands + agents + MCP + hooks behind one switch |
| Hooks | 7 lifecycle events | `PreToolUse`, `PostToolUse`, `Stop`, etc. |
| Memory | Auto, per project | `MEMORY.md` + one file per fact; off by default |
| Browser | Panel + picker | Element → context (title, URL, text, selector, HTML summary); agent can drive it |
| Preview | PDF / HTML / Mermaid / images / tables | Mermaid expand-to-full, table copy-as-Markdown/CSV, image gallery |
| Terminal | `Ctrl+J`, also a preview tab | Background long commands; bundled fast find/grep |
| Usage | Stats page | Token spend, including memory's extra requests |

Selora already has the skeleton of several of these (plan spine, todo spine, approval cards,
turn report with tokens/cost, session rail with Pinned/Today). The gap is coherence, not
existence.

---

## 6. Selora today — the honest inventory

**What we have that is genuinely good and should not be rebuilt:** plan and todo spines with
live progress, approval cards with split/unified diffs and keyboard genealogy that refuses
to approve from inside a text field (`ToolCard.tsx:116`), turn reports with files/tokens/cost,
subagent transcripts **and** a results list, session resumption, pinned sessions, workflows
with phase graphs and checkpointed resume, a command palette with categories, a cheatsheet,
and a token-driven visual system.

**What we have that is incoherent (evidence, not vibes):**

| Observation | Evidence | Why it hurts |
|---|---|---|
| Three permission states, two vocabularies, and a mode row that hides the honest name | `types.ts:17`, `AgentComposer.tsx:87`, `:122`, `:147` | The user must relearn the same idea twice; “Agent” sounds safer than “Full auto-apply” while being the same state |
| No middle mode between “ask for everything” and “apply everything” | `MODE_OPTIONS` has no “edits auto / commands ask” entry distinct from `full` | Everyday work forces a binary choice: friction or exposure |
| Four doors to the agent | `Ctrl+L` dock, activity bar “Agent”, titlebar pill, separate Agents window | Two independent session views (rail + tabs) and two palettes; users can't tell which window holds the truth |
| “Automations” is a workflow run list | `AgentsAutomationsView.tsx` → `WorkflowsPanel` | The label promises scheduled/triggered work; the surface delivers history |
| “Show Terminal” goes to a placeholder | `commands.ts:115` → `terminal_placeholder` | A visible command that does nothing is worse than no command |
| “Find in Files” borrows the `fuzzy_find` keybind field | `commands.ts:132` | Rebinding one concept silently moves another |
| Attachments inject protocol English into the user's prompt | `AgentComposer.tsx:172-175` | The transcript shows machine text the user never wrote — visible slop |
| Attachments are paths only | `composePrompt`, backend contract | A pasted screenshot cannot be seen; the agent must re-read what the user already showed |
| A question parks the turn with no clock | `AgentQuestionCard.tsx` | An agent waiting forever is indistinguishable from a hung app |
| Approvals offer only Accept / Reject | `ToolCard` action set | Fatigue → escape into Full auto-apply → the safety feature causes the unsafe state |
| No per-turn file checkpoint | no undo path beyond Git | Git becomes the undo stack, mixing agent and human edits |
| No project instructions loader | repo-wide search | Every session re-learns the repo; this repo has no `AGENTS.md` at all |
| `.zcode/` holds dev scratch (cdp + PowerShell) beside `plans/` and `workflow-runs/` | `.zcode/` listing | App state and developer tooling share one folder, under another product's name |

---

## 7. Gap matrix

Severity: **P0** = blocks trust / causes wrong action; **P1** = daily friction; **P2** =
leverage, do after the floor is solid. Confidence is my own: H = read in code, M = inferred,
L = judgement.

| # | Gap | ZCode reference | Sev | Conf | The fix, in one line |
|---|---|---|---|---|---|
| 1 | Two vocabularies for permission states | one label set, everywhere | P0 | H | Pick the honest names once; delete the other set |
| 2 | No “edits auto, commands ask” mode | *Edit automatically* | P0 | H | Make it a real fourth state, not a relabel of `full` |
| 3 | No scoped allow/deny (none found in the approval UI or protocol search) | Allow / Always allow / Reject / Always reject | P0 | M | Three scopes: this action, this session, this project |
| 4 | Questions can wait forever, unlabelled | 5-min countdown + “auto-continued” | P0 | H | Clock + explicit “continue without me” + marked history |
| 5 | No per-turn undo/reapply | Undo → Reapply, all-or-nothing | P0 | H | Content-addressed before-snapshots per turn |
| 6 | No project instructions | `AGENTS.md` global + workspace, ordered | P0 | H | Loader + visible “rules in effect” chip |
| 7 | Machine text in the user's prompt | structured attachments | P0 | H | Attachments become typed context, not prose |
| 8 | Paths-only context, no images/selection | image paste, selection → task | P0 | H | Accept image bytes; selection as a first-class reference |
| 9 | No `@ # / $` grammar | composer is a command line | P1 | H | One parser, one menu, four sigils |
| 10 | No side conversation | `/side`, `/btw` | P1 | H | A second thread that inherits history and cannot derail the task |
| 11 | No unified Command Center | `Ctrl+K` commands+chats+files | P1 | H | Fold palette + find-files + conversations into one |
| 12 | No find-in-conversation | `Ctrl+F` in conversation | P1 | H | Scoped search inside the transcript |
| 13 | Keyboard map diverges from muscle memory | `Ctrl+N`, `Ctrl+M`, `Ctrl+T`, `Shift+Tab`, `Ctrl+[`/`]` | P1 | H | Adopt the set or document the deviation loudly |
| 14 | No archive / custom groups / per-task diff counts | grouped tasks, retention, `+/-` | P2 | H | Task list grows up |
| 15 | No fork | fork from a completed reply | P2 | H | Branch without losing the original thread |
| 16 | No goal loop with verification | `/goal` + per-round check | P2 | H | The single biggest “long task” unlock |
| 17 | No skills / commands on disk | `$skill`, `/command` | P2 | H | Filesystem contract, metadata budget cap |
| 18 | No custom subagents | name/colour/model/tools/prompt | P2 | H | We have subagents; we lack authorship |
| 19 | No MCP, no hooks | servers + 7 lifecycle events | P2 | H | Do after skills/commands exist |
| 20 | No browser panel / element picker | picker → context, agent drives browser | P2 | H | The missing sense for frontend work |
| 21 | No rich preview (Mermaid, tables, images, PDF) | conversation viewers | P2 | H | Review quality, cheap to add |
| 22 | Usage is per-turn only | stats page | P2 | M | Aggregate per session/day/provider |

**Not gaps, despite looking like them:** remote development, remote control, bot channels,
plugin marketplace, repo wiki, mobile. Those are ZCode's distribution strategy. Copying them
would be scope theatre on a Windows desktop editor.

---

## 8. Slop audit — the thirteen things that will make a user distrust us

Written as a designer's review, not a bug list. Each has a root cause, because fixing the
symptom without the cause is how a product gets *more* confusing.

**S1 — One state, two dialects.**
The composer row says **Ask / Agent / Plan**. The dropdown says **Ask before changes / Full
auto-apply / Plan only**. A user who selects “Agent” and later opens the menu is told they are
in “Full auto-apply (Armed)” and has no reason to believe those are the same thing. Root
cause: labels were written for two different audiences (marketing-calm vs engineering-honest)
and never reconciled. *Taste note: the honest one should win. “Full auto-apply” is the best
sentence in our permission UI and we hide it behind a friendlier word.*

**S2 — “Agent” is the dangerous option wearing the safe word.**
Ask = guarded, Plan = read-only, Agent = applies file writes immediately. The middle label
reads like “the normal one”. It is the permissive one. Root cause: naming by actor instead of
by effect. *Rule: a mode label must state what it will do to my files.*

**S3 — Four doors, two truths.**
`Ctrl+L`, the activity bar, the titlebar pill, and the Agents window all claim to be “the
agent”. Two of them show a session rail; two show tabs. Same session can be open in both
windows with different scroll positions and different drafts. Root cause: the Agents window
was added as a second product surface without deciding what the dock is for. *This is the
single highest-cost confusion in the app, and no feature fixes it.*

**S4 — “Automations” that do not automate.**
The page promises scheduled or triggered work and delivers a run history of workflows started
from chat. Root cause: a borrowed ZCode label attached to a surface that predates it. Either
build triggers (schedule, on-save, on-PR) or rename to “Runs”.

**S5 — “Show Terminal” that shows a placeholder.**
A command in the registry, listed in the palette, wired to `terminal_placeholder`. Root cause:
a routing slice landed before the surface did. *A dead command is a broken promise; remove it
the day it stops being true.*

**S6 — Find in Files wearing the fuzzy-find keybind.**
`editFindInFiles` reads `fuzzy_find` from settings. Rebinding “fuzzy find” moves Find in
Files. Root cause: config vocabulary lagging behind command vocabulary. Users experience this
as “my shortcuts randomly change”.

**S7 — The transcript contains sentences the user did not write.**
Attaching a file appends `Attached workspace files (read them with read_file before
answering):`. It is honest engineering, and it is visible slop: the user's own message is
mutated in the record, and the instruction leaks our tool contract into their history. Root
cause: context was modelled as text instead of a typed field.

**S8 — Two waits that look identical and behave oppositely.**
An approval parks the turn until answered. A question parks the turn until answered. One is a
gate, the other is a courtesy; neither is labelled as such and neither has a clock. Root
cause: one visual pattern reused for two different social contracts.

**S9 — Safety that promotes unsafety.**
Accept/Reject per action, no memory, no scope. By the tenth write the user is not reading the
diff anymore — they are clicking Accept to make it stop. Then they discover Full auto-apply
and never come back. Root cause: we optimized the prompt (visible) instead of the decision
(durable). *ZCode's four-way decision exists precisely because per-action prompts do not
scale.*

**S10 — Undo is Git's job, which means it is nobody's job.**
There is no inverse for “the agent changed six files”. `git checkout` also throws away the
user's own edits from the same window. Root cause: file ownership was never modelled per turn.
*The test: can a user reverse one agent turn without losing one human edit? Today, no.*

**S11 — The app re-learns the repo every morning.**
No `AGENTS.md`, no rules, no memory. Every session starts from zero and the user re-explains
pnpm, the test command, the house style. Root cause: instructions were treated as prompt
content instead of project configuration.

**S12 — Delight is load-bearing where legibility should be.**
Subagent “pets” live in the title bar as the ambient surface, while the actual list lives in
the feed. Pets are charming; a pet is not a status. Root cause: we solved “make background
work feel alive” before “make background work scannable”. *Keep the pets. Put a count and a
state next to them, and make the list the canonical surface.*

**S13 — Two namespaces under one roof.**
`.selora` is ours; `.zcode` in this repo contains CDP probes and PowerShell click scripts
next to `plans/` and `workflow-runs/`. Root cause: developer instrumentation was allowed to
use the app's state folder. A user who opens that folder learns we are not sure what belongs
to whom.

---

## 9. Where ZCode itself is wrong — do not copy these

This matters as much as the gap list. Copying a mature product's mistakes is the most
expensive way to look derivative.

| ZCode wart | Why it is bad | Our call |
|---|---|---|
| Memory: off by default, **no in-app way to review or delete** what was remembered | The user cannot audit their own agent's beliefs; “turn it off” is the only control | Memory must be a visible, editable list from day one |
| Side conversation is **unrecoverable** on close/quit | A good idea with a trapdoor; users lose work to a feature meant to be casual | Persist it as a child thread, or label the loss loudly |
| Selection capture is **single-line only** | Undercuts the whole “point at the thing” idea | Multi-line, with an explicit cap message |
| Goal mode is **unavailable in Plan mode** | Artificial conflict between “decide what” and “decide how” | Allow goal + plan; they compose |
| Plugin catalog needs an open workspace and GitHub reachability | Managing extensions shouldn't require a project | Not building plugins yet — but when we do, settings must work with no folder open |
| Marketplace vs plugin version comparison bug | Silent “no update available” | One source of truth for versions |
| Countdown auto-continue is **non-retroactive** | Toggling the setting doesn't change waiting questions | Make the setting authoritative immediately |
| Questions auto-continue after 5 minutes by default | Good for flow, bad for irreversible choices | Auto-continue only for non-destructive questions |

---

## 10. Five principles that settle future arguments

1. **One state, one name.** If two surfaces name the same state differently, that is a bug,
   not a tone choice. Grep should be able to prove this.
2. **Every wait says who owns it.** Approval = the user must act. Question = the user may act,
   and the machine will proceed. Background = nobody is waiting. Never a fourth kind of
   silence.
3. **Reversibility beats confirmation.** A confirmation you can undo is worth three you
   cannot. Prefer snapshots to prompts.
4. **One task, one home.** A task lives in exactly one list, opens in exactly one place, and
   every other entry point navigates there rather than rendering a copy.
5. **Delight rides on legibility.** Novelty may decorate a surface; it may never be the only
   way to read it.

---

## 11. The plan

Sequenced by *trust gained per unit of work*, not by how impressive it looks in a changelog.
Each item names the user-visible outcome, what it replaces, the acceptance signal, and the
risk. No item is “add a panel” unless a panel is the honest answer.

### Wave 1 — Stop the bleeding (P0, trust)

| Item | User-visible outcome | Replaces / removes | Acceptance signal | Risk |
|---|---|---|---|---|
| 1.1 Canonical permission model | Four states, one name each: **Read-only**, **Ask before changes**, **Auto-edit**, **Full access**. `Shift+Tab` cycles. | The Ask/Agent/Plan row *and* the Full-auto-apply/Plan-only menu | Grep: each state appears under one label only; a first-time user can state what each does | New fourth state needs backend semantics, not just UI |
| 1.2 Scoped decisions | Approve once, for the session, or for this project; deny permanently | Accept/Reject only | Median approvals per session drops without raising “full access” usage | Scope creep in what “same class of action” means — define it narrowly |
| 1.3 Waits are labelled and clocked | Approval blocks and says “needs you”; questions count down and continue, marked in history | Silent parking | No turn can sit unanswered with no visible clock; history shows “continued without an answer” | Auto-continuing a destructive question would be worse than waiting — restrict it |
| 1.4 Per-turn checkpoints | Undo this turn → Reapply; list of what can't be restored, before writing anything | Git-as-undo | A user reverses one agent turn and keeps their own edit in the same file | Manual edits during the turn must be detected, never silently reverted |
| 1.5 Project instructions | `AGENTS.md` (global + workspace) + `.selora/rules`, with a “rules in effect” chip | Nothing | A fresh session already knows the test command and house style | Precedence must be stated, not inferred |
| 1.6 Structured context | Attachments render as typed chips; the transcript never contains our protocol English | The injected header text | Transcript diff: zero machine-authored lines inside a user message | Backend needs a context field, not a string |
| 1.7 Honest commands | Delete “Show Terminal → placeholder”; unbind Find in Files from `fuzzy_find` | Two wrong strings | No command in the palette leads to a placeholder surface | Cheap, do it first |

**Wave 1 is done when** a new user can be handed the app with no explanation and not make a
dangerous choice by accident.

### Wave 2 — Make it flow (P1, daily friction)

| Item | Outcome | Acceptance |
|---|---|---|
| 2.1 Context grammar `@ # / $` + `+` menu | The composer becomes the command line; typing beats dragging | A user references a file, a past conversation, a command and a skill without opening a panel |
| 2.2 Selection → “add to task” / “ask aside” | Point at the thing instead of describing it | Selected assistant text arrives as a reference, not a paraphrase |
| 2.3 Side conversation | Ask without derailing; inherits history, cannot touch the goal | A question asked mid-run does not alter the task's plan or queue |
| 2.4 One task, one home | Collapse four doors to two: **dock** (transient) and **task window** (durable). One session list. | No two surfaces can show divergent state for the same session |
| 2.5 Command Center `Ctrl+K` | Commands + conversations + files in one panel | Any destination reachable in ≤ 3 keystrokes from anywhere |
| 2.6 Find in conversation | `Ctrl+F` scoped to the transcript | Long-task archaeology stops requiring scrolling |
| 2.7 Keyboard map | Adopt `Ctrl+N`, `Ctrl+M`, `Ctrl+T`, `Shift+Tab`, `Ctrl+[/]`; document deviations once | A user coming from ZCode/VS Code is not punished by muscle memory |
| 2.8 Editable queue | Fix a queued message by pulling it back into the composer | No message is sent that the user already regretted |

### Wave 3 — Leverage (P2, after the floor is solid)

Ordered by dependency, not appeal: **skills + commands on disk** (they enable everything
else) → **goal loop with verification** (the long-task unlock; needs 1.4 checkpoints to be
safe) → **custom subagents** (we have the runtime, we lack authorship) → **MCP + hooks** →
**browser panel + element picker** (frontend work's missing sense) → **rich preview**
(Mermaid, tables, images, PDF) → **task list maturity** (archive, groups, per-task `+/-`) →
**aggregate usage**.

Two guards on Wave 3: a **metadata budget** for skills (ZCode's own docs admit that enough
skills degrade to names-only and auto-triggering collapses — cap it and show the budget), and
**no new top-level navigation**. Wave 3 adds capabilities inside existing surfaces; if a new
nav item appears, the plan is wrong.

---

## 12. What we will not build (and will say out loud)

Remote control from a phone, bot channels, a plugin marketplace, a repo wiki, remote
development over SSH/Docker, and iOS/Android emulator drivers. These are coherent for an ADE
sold as a service with a model attached. Ours is a Windows desktop editor with bring-your-own
keys. Adding them now would buy attention we cannot keep.

Also refused: a second changes panel (we already have Git changes *and* agent approvals — the
fix is ownership labels on one surface, not a third view), a dark-mode switch as a feature
(theme tokens already exist), and any “AI-powered” rename of an existing panel.

---

## 13. How we will know it worked

Measured, not asserted:

| Signal | Baseline to capture first | Target |
|---|---|---|
| Approvals per session | count now | ↓ ≥ 40% with no increase in “full access” usage |
| Turns ending in silent wait > 2 min | count now | → 0 |
| Mode switches per session | count now | ↓ (less hunting for the right mode) |
| “Undo turn” used vs `git checkout` after agent work | ratio | undo dominates |
| Re-explaining project facts in prompts | sampled transcripts | ↓ after 1.5 |
| Transcripts containing our protocol text | grep `read them with read_file` | 0 |
| Labels per permission state | grep | exactly 1 |
| Time-to-first-accepted-change (new user) | 5 hallway tests | ↓ ≥ 30% |

The strongest external check remains: **a first-time user completes a three-step task
(small fix → review → revert) with no question asked of us.** If they ask “which window am I
in?” the IA work isn't done. If they ask “what does Agent mean?” the vocabulary work isn't
done. If they ask “how do I undo that?”, the reversibility work isn't done.

---

## Appendix A — Keyboard delta (ours vs the muscle memory we're competing with)

| Intent | ZCode | Selora today | Verdict |
|---|---|---|---|
| New task | `Ctrl+N` | — | Add |
| Command Center | `Ctrl+K` | `Ctrl+Shift+P` (commands only) | Add `Ctrl+K`, keep palette chord |
| Find in conversation | `Ctrl+F` | `Ctrl+F` = editor find | Scope by focus |
| Quick open file | (in Command Center) | `Ctrl+P` = Find in **Files** (search) | Rename the concept or the chord — today it lies |
| Toggle terminal | `Ctrl+J` | `Ctrl+J` | ✓ |
| Toggle sidebar | `Ctrl+B` | `Ctrl+B` | ✓ |
| Right pane | `Ctrl+Alt+B` | — | Add when a right pane exists |
| Model picker | `Ctrl+M` | — | Add |
| Thought level | `Ctrl+T` | — | Add |
| Cycle modes | `Shift+Tab` | — | Add |
| Back / forward | `Ctrl+[` / `]` | — | Add |
| Prev / next conversation | `Ctrl+Shift+[` / `]` | — | Add |
| Agent dock | (task list) | `Ctrl+L` | Keep; make it the transient door only |

## Appendix B — Canonical vocabulary (adopt verbatim, delete synonyms)

| Concept | Use | Never use |
|---|---|---|
| Permission states | Read-only · Ask before changes · Auto-edit · Full access | Ask/Agent/Plan row; “Full auto-apply (Armed)”; “Guarded” |
| A unit of work | **Task** | session, conversation, chat, thread (as synonyms) |
| Waiting on the user | **Needs approval** | “pending”, “blocked” |
| Waiting on the user, optionally | **Question** (+ countdown) | “needs attention” |
| Machine running | **Running** | “thinking”, “breathing” |
| Reversing one turn | **Undo turn / Reapply** | “reset”, “revert” (reserved for Git) |
| Agent-authored file edits | **Agent changes** | “changes” (ambiguous with Git) |
| Workflow runs | **Runs** | “Automations” until triggers exist |

## Appendix C — Ownership map for whoever implements this

Shared integration files (one writer at a time, coordinator-owned):
`crates/selora-protocol/src/lib.rs`, `apps/selora-desktop/src-tauri/src/{state,sessions}.rs`,
`frontend/src/bridge/bridge.ts`, `features/agent/{agent-store.ts,types.ts}`,
`components/AgentComposer.tsx`, `layout/{AgentDock,TitleBar,ActivityBar}.tsx`,
`features/keymap/commands.ts`, `styles/tokens.css`.

Everything else in this plan is additive: new loaders (instructions, memory, skills), new
checkpoint storage, new context parser, new decision scopes. Wave 1 touches shared files the
most; that is another reason it goes first, while the surface is still small.

---

*Sources: ZCode docs (`zcode.z.ai/en/docs`: agents, goal, memory, skill, plugin, subagents,
safety-confirm, ADE-tools, task-management, edit-history, keyboard-shortcuts), ZCode repo
(`github.com/zai-org/ZCode`, v3.18.2 README, `apps/zcode-cli/`), and Selora source as cited
inline. ZCode was not run; Selora was not run. This is source- and doc-based critique.*
