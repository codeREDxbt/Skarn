# Copy-paste prompts — ZCode UX convergence

Two self-contained prompts. Each is written to be pasted into a fresh agent session with no
other context. Both assume the repo root and `ZCODE_UX_TEARDOWN.md` exist.

Use **/plan** when you want a decision-ready implementation plan. Use **/research** when you
want the contested claims settled with evidence before committing to the plan.

---

## /plan — implementation plan (no code)

```text
/plan

Role: senior product engineer + interaction designer. You are planning, not coding. Produce
no implementation code in this turn — no diffs, no patches, no file writes.

Objective: turn ZCODE_UX_TEARDOWN.md into a decision-ready implementation plan for Selora IDE
that a second engineer could execute without asking me a single design question.

Read first, in this order:
1. ZCODE_UX_TEARDOWN.md (whole file; it carries the findings and the sequencing logic)
2. apps/selora-desktop/frontend/src/features/agent/types.ts
3. apps/selora-desktop/frontend/src/features/agent/components/AgentComposer.tsx
4. apps/selora-desktop/frontend/src/features/agent/components/AgentQuestionCard.tsx
5. apps/selora-desktop/frontend/src/features/agent/components/ToolCard.tsx
6. apps/selora-desktop/frontend/src/features/agents-workspace/AgentsAutomationsView.tsx
7. apps/selora-desktop/frontend/src/features/agents-workspace/AgentsNavigationRail.tsx
8. apps/selora-desktop/frontend/src/features/keymap/commands.ts
9. apps/selora-desktop/frontend/src/layout/{ActivityBar,TitleBar,AgentDock}.tsx
10. crates/selora-protocol/src/lib.rs  (context DTOs, permission + approval payloads)
11. apps/selora-desktop/src-tauri/src/{state.rs,sessions.rs}  (only the sections that own
    approvals, questions, sessions and file writes — use search, do not read them whole)

Established facts (do not re-research; correct me only if the code contradicts them):
- PermissionMode has three values (`full`, `plan_only`, `ask_before_change`) but two visible
  vocabularies: the Ask/Agent/Plan row and the Full auto-apply/Plan only/Ask before changes
  menu. "Agent" and "Full auto-apply" are the same state.
- There is no "edits auto, commands ask" mode distinct from `full`.
- Approvals offer Accept/Reject only; a search of the approval payload and UI found no session- or project-scoped allow/deny (verify before building).
- AgentQuestionCard parks a turn with no timer and no auto-continue.
- There is no per-turn file checkpoint; Git is the only undo.
- There is no AGENTS.md / .selora/rules loader.
- Attachments are workspace paths; the composer appends protocol English to the user prompt.
- There is no @/ /$ context grammar, no selection-to-context, no side conversation, no
  find-in-conversation.
- "Automations" renders WorkflowsPanel. "Show Terminal" targets terminal_placeholder.
- Four agent entry points exist: Ctrl+L dock, activity bar, title bar pill, Agents window.

Hard constraints:
- Wave 1 only in detail (trust fixes). Wave 2 and 3 get one paragraph each and a dependency
  graph, not task cards. Depth over breadth.
- No new top-level navigation. No second changes panel. No remote/mobile/plugin-marketplace
  work.
- Reuse existing surfaces: plan spine, todo spine, approval cards, turn report, session rail.
- Every destructive action introduced must name its inverse.
- One state, one name: if your plan needs a new label, it must also delete the old one.
- Preserve the existing visual system (tokens.css); no new design language.

Deliver exactly this:
A. A state table for the four permission modes: name, what it may write, what it may execute,
   what it asks, what UI signal shows it is active. One row per mode, no synonyms.
B. A decision table for approvals: action class x scope (once / session / project), with what
   is remembered and where it is stored, and how a user revokes it.
C. The wait taxonomy: for each wait kind (approval, question, background, blocked), who owns
   it, what the UI shows, whether it times out, and what history records.
D. Checkpoint design: what is captured, when, keyed by what, how manual edits are detected,
   what "cannot restore" looks like before anything is written.
E. A task-card list for Wave 1. Each card: ID, user-visible outcome in one sentence, files it
   owns (exact paths), files it must not touch, what it deletes, acceptance steps a person can
   perform, the check command, and its risk.
F. Sequencing: dependency graph + a parallelisation map with at most three concurrent writers
   and one writer per shared file.
G. The three decisions you are least sure about, each with the cheapest experiment that would
   settle it in under a day.
H. A one-page "what we are NOT doing in this wave" list.

Rules of engagement:
- Cite file:line for every claim about current behaviour.
- If two sources disagree, say so and pick one, with reasoning.
- Do not invent backend capabilities. If a fix needs a protocol change, say which contract
  changes and what breaks.
- Do not pad. No restating the request, no motivational intro, no summary of what you read.
- End with the single riskiest assumption in the plan and how it fails.
```

---

## /research — settle the contested claims with evidence

```text
/research

Objective: produce an evidence-backed decision memo on the six contested UX decisions in
ZCODE_UX_TEARDOWN.md, so we stop arguing from taste. Research only. No code, no plan, no
implementation.

Contested decisions:
1. Should Selora converge on ZCode's four permission modes and its vocabulary, or keep a
   three-mode model with honest labels?
2. Should the Agents window and the agent dock merge into one surface, or stay two with a
   clear division of labour?
3. Is an auto-continuing question timer (ZCode: 5 minutes) acceptable for a coding agent that
   can write files, or does it create irreversible mistakes?
4. Are per-turn file checkpoints worth their storage and conflict complexity, or is a
   pre-turn Git snapshot enough?
5. Should context be typed (@ # / $ grammar) or stay attachment-driven (drag, chips, menus)?
6. Does ZCode's plugin/skill ecosystem matter for a bring-your-own-key Windows editor, or is
   it a distribution strategy we should ignore?

Raw material, in priority order — do not substitute marketing copy:
- ZCode source: github.com/zai-org/ZCode — apps/zcode-cli/ (permission gate, composer mention
  parsing, task/session store, question timeout handling), skills/commands loaders, plugin
  manifests.
- ZCode changelog (zcode.z.ai/en/changelog): read it as a defect log. Every fix is an
  admission. Extract entries that touch permissions, waits, undo, context, task list.
- ZCode docs "Limits and Notes" sections (memory, skill, subagent, edit-history, goal,
  safety-confirm): self-admitted failures are the most honest UX data available.
- ZCode GitHub issues and discussions: search for confusion language ("which window",
  "always allow", "lost my conversation", "undo").
- Our source: the files listed in the teardown, plus sessions.rs / state.rs for what is
  actually persisted about approvals and writes.
- One 20-minute screen recording of a real Selora session, transcribed into a confusion log
  (timestamps + what the user tried + what happened). If a recording is unavailable, say so
  and use a scripted cognitive walkthrough instead — do not simulate data.

Method:
- Heuristic evaluation (Nielsen's ten) of both products against the same five task scripts:
  small fix; multi-file refactor with review; recover from a bad agent turn; ask a side
  question mid-run; hand off to a background agent.
- Keystroke-level count for the three highest-frequency flows (send prompt, approve a change,
  undo a turn) in both products.
- Vocabulary audit: every user-visible string naming a mode, a wait, or a change, grouped by
  concept, with duplicates called out.
- Reversibility audit: every destructive action in our app, its inverse, and whether the
  inverse is reachable without Git.
- Interruption audit: every wait, classified by owner and by timeout behaviour.

Deliver exactly this:
A. Verdict per decision: adopt / adapt / reject, one paragraph each, with the single strongest
   piece of evidence for and against.
B. An evidence table: claim | source (path or URL) | what it actually says | confidence
   (high/medium/low).
C. The confusion log from the recording or walkthrough, verbatim observations only.
D. Three things ZCode does that we should explicitly NOT copy, with the failure each causes.
E. What would falsify each verdict: the observation that should change our mind.
F. Open questions that no source can answer and that only a user test can.

Rules:
- Quote, don't paraphrase, when the exact wording matters.
- Distinguish "the docs say" from "the code does" from "I infer".
- No recommendations beyond the six decisions. No roadmap. No code.
```

---

### How to run them

1. `/plan` first if you already trust the findings and want execution shape.
2. `/research` first if you want the six decisions defended before anyone writes a task card.
3. Run `/research` → `/plan` when the team disagrees on mode count or surface merge — those two
   arguments are the ones that silently fork a codebase.

*Note: no `gvs` skill library was found at the root of `X:\` (only the top level was listed,
not the whole drive). These prompts therefore depend on nothing outside this repo. If the
library lives deeper on that drive, give me the path and I will fold its conventions in.*
