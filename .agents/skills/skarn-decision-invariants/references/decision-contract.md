# Decision Engine Contract & Verification Checklist

## Provider-Neutral Types (Adapt to Rust `serde` / TypeScript DTOs)

```ts
export interface WorkspaceSnapshot {
  snapshotId: string;
  workspaceRoot: string;
  gitHead?: string;
  documentVersions: Record<string, { revision: number; sha256: string }>;
  capturedAtMs: number;
}

export interface EvidenceItem {
  id: string;
  sourceKind: 'file' | 'symbol' | 'diagnostic' | 'tool_result' | 'policy';
  sourceRef: string;
  trust: 'verified_fact' | 'untrusted_workspace' | 'inferred';
  summary: string;
}

export interface DecisionRequest {
  requestId: string;
  taskType: string;
  snapshot: WorkspaceSnapshot;
  trustedFrame: {
    question: string;
    options: Array<{ id: string; label: string; actionId?: string }>;
    allowedActions: string[];
    permissionMode: 'read_only' | 'ask_before_changes' | 'auto_edit' | 'full_access';
  };
  untrustedEvidence: EvidenceItem[];
  deadlineMs: number;
}

export interface DecisionResult {
  provider: string;
  model: string;
  decisionType: 'binary' | 'choice' | 'score' | 'action';
  value: unknown;
  probabilities: Record<string, number>;
  rawConfidence: number;
  evidenceRefs: string[];
  risk: 'low' | 'medium' | 'high' | 'critical';
  latencyMs: number;
  reasoningSummary?: string;
  validationErrors: Array<{ code: string; field: string; message: string }>;
}
```

## Risk x Permission Mode Matrix (Fail-Closed Defaults)
- `high` / `critical` risk: **ALWAYS require user approval** (0% silent auto-exec, even at `rawConfidence === 1.0`).
- `medium` risk: require approval unless explicitly covered by `auto_edit` (file edits only) or a narrow scoped session/project grant.
- `low` risk (read-only / deterministic inspect): auto-execute when schema-valid and above per-action calibrated threshold; otherwise clarify or escalate (bounded).
- Any schema error, unregistered action, out-of-root path, or symlink escape: **Reject** immediately; never auto-retry blindly into execution.

## Required Adversarial & Shadow-Mode Proofs
1. **Zero-escape fuzzing (`proptest` / `fast-check`):** malformed JSON, unknown actions, `../` traversals, symlink escapes outside workspace root, privilege escalations, spoofed `1.0` confidence on high-risk actions -> 0% bypass.
2. **TOCTOU & Cancellation:** concurrent buffer mutation during inference -> transitions to `Stale`, zero file mutation; `AbortSignal` / cancellation mid-call -> zero dangling processes or side effects.
3. **Shadow-mode replay harness:** labeled golden trace set reporting confusion matrix, calibration/risk-coverage, escalation hop distribution, net p50/p95 latency delta, and LLM-call reduction.
4. **Secret hygiene:** assert API keys & `Authorization` headers never appear in logs, error chains, debug panels, or telemetry payloads.
