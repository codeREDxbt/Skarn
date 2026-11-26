---
name: skarn-decision-invariants
description: Enforce security, snapshot/TOCTOU, FSM lifecycle, calibration, privacy, and provider-adapter invariants when designing or implementing the Skarn Decision Engine, Policy Engine, Action Validator, Checkpoints, or Laya/Jev/Gemini adapters.
---

# Skarn Decision Engine Invariants

Load `references/decision-contract.md` before writing any Decision Engine, Policy Engine, Checkpoint, or provider adapter code.

## Non-Negotiable Invariants

1. **Three-tier routing with hop caps:**
   - Tier 1: Deterministic IDE tools (always preferred when solvable without inference).
   - Tier 2: Closed-ended Decision Models (Local Laya default; BYOK TypeSafe Jev & Google Gemini Decisions API behind a strict provider-neutral adapter boundary).
   - Tier 3: Existing general LLM provider (never replaced).
   - Enforce an explicit max hop count (<= 2 escalations), per-tier latency deadlines, and cancellation propagation (`CancellationToken` / `AbortSignal`).
2. **Explicit Lifecycle FSM:**
   `Snapshotted -> Routed -> Inferred -> SchemaValidated -> PolicyEvaluated -> [ApprovalPending] -> SnapshotRechecked -> Executed -> Verified | Stale | Escalated | Rejected`
3. **TOCTOU & Shared Snapshot Spine:**
   - Bind every `DecisionRequest` to an immutable `WorkspaceSnapshot` (document versions, content hashes, workspace root, git HEAD).
   - Re-verify the snapshot immediately before executing any mutating action. If state drifted mid-inference, transition to `Stale` and abort—NEVER mutate drifted buffers.
   - Reuse this same snapshot/hashing mechanism for per-turn Undo/Reapply checkpoints.
4. **Trusted vs. Untrusted Context Separation:**
   - Treat ALL model outputs AND ALL workspace-derived context (file contents, code comments, compiler diagnostics, tool outputs) as untrusted input.
   - Separate trusted policy/action definitions from untrusted workspace evidence in the request contract. Tag every evidence item with `source` and `verified_fact | inferred`.
5. **Capability-Bound Execution & Asymmetric Risk Gating:**
   - NEVER execute raw model strings or use shell interpolation.
   - Validate action ID against the controlled IDE action registry, canonicalize paths, resolve symlinks, and verify targets remain strictly inside workspace boundaries.
   - High/critical-risk actions: **0% silent auto-execution** regardless of confidence (`1.0` included).
   - Never treat raw provider confidence as calibrated across providers.
6. **Privacy & Unfinalized Seams:**
   - Workspace `local-only` privacy mode is enforced at the orchestrator boundary—cloud adapters cannot be invoked or receive context when enabled.
   - Never fabricate unverified API schemas for TypeSafe Jev or Laya. Gate unverified fields behind a documented adapter seam + mock/dev runtime and list unresolved details explicitly.
   - Scrub API keys and auth headers from all logs, errors, traces, and telemetry.

## Mandatory Verification

Before completing any Decision Engine task:
1. Run `node .agents/skills/skarn-decision-invariants/scripts/check-decision-hygiene.mjs`
2. Verify adversarial property tests (`proptest` / `fast-check`) prove 0% bypass on invalid actions, symlink escapes, stale snapshots, privacy mode, and spoofed `1.0` confidence.
