//! Provider-neutral Decision Engine kernel for Skarn.
//!
//! This crate houses the standalone decision lifecycle FSM, workspace snapshot
//! TOCTOU defense, closed capability/path validator, calibration-aware policy
//! matrix, scoped grant ceilings, content-addressed turn checkpoints,
//! race-free cancellation, and offline shadow-mode replay harness.

pub mod checkpoint;
pub mod fsm;
pub mod orchestrator;
pub mod policy;
pub mod providers;
pub mod shadow;
pub mod snapshot;
pub mod types;
pub mod validator;

pub use checkpoint::{
    CheckpointConflict, CheckpointError, FileCheckpointRecord, PruneStats, TurnCheckpointManifest,
    TurnCheckpointStore, TurnFileChange, MAX_CHECKPOINT_BYTES_TOTAL,
    MAX_CHECKPOINT_TURNS_PER_WORKSPACE,
};
pub use fsm::{DecisionFsm, FsmTransitionError};
pub use orchestrator::{
    ActionExecutor, ApprovalResponse, DecisionOrchestrator, DecisionOutcome,
    MAX_DECISION_HOPS, TIER1_DEADLINE_MS, TIER2_MAX_DEADLINE_MS,
};
pub use policy::{
    evaluate_policy, CalibrationEntry, CalibrationTable, GrantDecision, PolicyError,
    PolicyVerdict, RulesBoundaryPolicy, RulesLoadResult, RulesLoader, ScopedGrantEntry,
    ScopedPolicyStore,
};
pub use providers::{
    parse_provider_json, AdversarialMode, AdversarialMockProvider, DecisionCancelToken,
    DecisionProvider, DeterministicRuleProvider, ScriptedMockProvider,
};
pub use shadow::{
    evaluate_shadow_traces, CalibrationBin, ConfusionMatrix, GoldenTrace, HopHistogram,
    LatencySummary, RiskCoveragePoint, ShadowReplayReport,
};
pub use snapshot::{
    sha256_bytes, sha256_file, BufferSnapshotState, DriftReason, FilesystemWorkspaceReader,
    SnapshotCapturer, SnapshotRechecker, WorkspaceStateReader,
};
pub use types::{
    ActionClass, ActionDescriptor, DecisionOption, DecisionPhase, DecisionRequest,
    DecisionRequestId, DecisionResult, DecisionType, DocumentVersionRecord, EvidenceItem,
    EvidenceSourceKind, EvidenceTrust, ExecutionTier, PermissionMode, ProviderError,
    ProviderLocality, RiskLevel, RoutingExplanation, RuleScope, RuleSource, SnapshotId,
    TrustedFrame, ValidationError, WorkspaceSnapshot,
};
pub use validator::{
    canonical_root, de_verbatim, is_secret_path, is_within, rel_display, resolve_rooted,
    same_path, ActionRegistry, ActionSpec, ValidatedAction, ValidatedDecision,
};

const REDACTED: &str = "[REDACTED]";

/// Scrub secret tokens (`sk-*`, `Bearer *`, `Basic *`, `Authorization` headers,
/// common provider tokens, and inline `api_key=` values) from logs, errors,
/// traces, and explanations.
pub fn scrub_secrets(input: &str) -> String {
    scrub_secrets_with_keys(input, &[])
}

/// Scrub both standard secret patterns and caller-registered literal keys.
pub fn scrub_secrets_with_keys(input: &str, registered_keys: &[&str]) -> String {
    if input.is_empty() {
        return String::new();
    }

    let mut working = input.to_owned();
    for key in registered_keys {
        if !key.is_empty() {
            working = working.replace(key, REDACTED);
        }
    }

    working = redact_prefix_tokens(&working, "sk-", 4, is_token_char);
    working = redact_prefix_tokens(&working, "AIza", 10, is_token_char);
    working = redact_prefix_tokens(&working, "ghp_", 10, is_alnum_or_underscore);
    working = redact_prefix_tokens(&working, "github_pat_", 10, is_alnum_or_underscore);
    working = redact_prefix_tokens(&working, "xoxb-", 6, is_token_char);
    working = redact_prefix_tokens(&working, "xoxp-", 6, is_token_char);
    working = redact_prefix_tokens(&working, "xoxa-", 6, is_token_char);

    working = redact_case_insensitive_scheme(&working, "bearer ");
    working = redact_case_insensitive_scheme(&working, "basic ");
    working = redact_header_or_kv(&working, "authorization");
    working = redact_header_or_kv(&working, "x-api-key");
    working = redact_header_or_kv(&working, "api_key");
    working = redact_header_or_kv(&working, "apikey");

    working
}

fn is_token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')
}

fn is_alnum_or_underscore(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_boundary_before(bytes: &[u8], idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    let prev = bytes[idx - 1];
    !prev.is_ascii_alphanumeric() && prev != b'_'
}

fn redact_prefix_tokens(
    input: &str,
    prefix: &str,
    min_tail_len: usize,
    is_body: fn(u8) -> bool,
) -> String {
    let bytes = input.as_bytes();
    let p_bytes = prefix.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i..].starts_with(p_bytes) && is_boundary_before(bytes, i) {
            let mut j = i + p_bytes.len();
            while j < bytes.len() && is_body(bytes[j]) {
                j += 1;
            }
            if j - (i + p_bytes.len()) >= min_tail_len {
                out.push_str(REDACTED);
                i = j;
                continue;
            }
        }
        let ch = input[i..].chars().next().unwrap_or('\0');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn redact_case_insensitive_scheme(input: &str, scheme_with_space: &str) -> String {
    let bytes = input.as_bytes();
    let lower = input.to_ascii_lowercase();
    let l_bytes = lower.as_bytes();
    let s_bytes = scheme_with_space.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;

    while i < bytes.len() {
        if l_bytes[i..].starts_with(s_bytes) && is_boundary_before(bytes, i) {
            let token_start = i + s_bytes.len();
            let mut j = token_start;
            while j < bytes.len()
                && !bytes[j].is_ascii_whitespace()
                && !matches!(bytes[j], b'"' | b'\'' | b',' | b';' | b')' | b']' | b'}')
            {
                j += 1;
            }
            if j > token_start && &input[token_start..j] != REDACTED {
                out.push_str(&input[i..token_start]);
                out.push_str(REDACTED);
                i = j;
                continue;
            }
        }
        let ch = input[i..].chars().next().unwrap_or('\0');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn redact_header_or_kv(input: &str, key_lower: &str) -> String {
    let bytes = input.as_bytes();
    let lower = input.to_ascii_lowercase();
    let l_bytes = lower.as_bytes();
    let k_bytes = key_lower.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;

    while i < bytes.len() {
        if l_bytes[i..].starts_with(k_bytes) && is_boundary_before(bytes, i) {
            let mut sep = i + k_bytes.len();
            if sep < bytes.len() && matches!(bytes[sep], b'"' | b'\'') {
                sep += 1;
            }
            while sep < bytes.len() && bytes[sep] == b' ' {
                sep += 1;
            }
            if sep < bytes.len() && matches!(bytes[sep], b':' | b'=') {
                sep += 1;
                while sep < bytes.len() && matches!(bytes[sep], b' ' | b'"' | b'\'') {
                    sep += 1;
                }
                let mut end = sep;
                while end < bytes.len()
                    && !matches!(bytes[end], b'\r' | b'\n' | b'"' | b'\'' | b',' | b';' | b'}')
                {
                    end += 1;
                }
                if end > sep && input[sep..end].trim() != REDACTED {
                    out.push_str(&input[i..sep]);
                    out.push_str(REDACTED);
                    i = end;
                    continue;
                }
            }
        }
        let ch = input[i..].chars().next().unwrap_or('\0');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}
