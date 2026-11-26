use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::scrub_secrets;

pub type SnapshotId = String;
pub type DecisionRequestId = String;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    #[serde(alias = "explore")]
    ReadOnly,
    #[serde(alias = "execute")]
    AskBeforeChanges,
    #[serde(alias = "auto")]
    AutoEdit,
    FullAccess,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionClass {
    ReadInspect,
    FileEdit,
    FileDelete,
    CommandAllowlisted,
    CommandGated,
    Workflow,
    PlanControl,
    Question,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionType {
    Binary,
    Choice,
    Score,
    Action,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSourceKind {
    File,
    Symbol,
    Diagnostic,
    ToolResult,
    Policy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceTrust {
    VerifiedFact,
    UntrustedWorkspace,
    Inferred,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DocumentVersionRecord {
    pub revision: Option<u64>,
    pub sha256: Option<String>,
    pub dirty: bool,
    pub exists: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    pub snapshot_id: SnapshotId,
    /// Canonicalized workspace root with Windows `\\?\` prefix stripped.
    pub workspace_root: String,
    pub git_head: Option<String>,
    pub document_versions: BTreeMap<String, DocumentVersionRecord>,
    pub workspace_trusted: bool,
    pub trust_epoch: u64,
    pub local_only: bool,
    pub captured_at_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvidenceItem {
    pub id: String,
    pub source_kind: EvidenceSourceKind,
    /// e.g. `"src/main.rs:12-28"` or `"tool:call_1"`.
    pub source_ref: String,
    pub trust: EvidenceTrust,
    /// Secret-scrubbed, length-capped summary.
    pub summary: String,
}

impl EvidenceItem {
    pub const MAX_SUMMARY_BYTES: usize = 512;

    pub fn new(
        id: impl Into<String>,
        source_kind: EvidenceSourceKind,
        source_ref: impl Into<String>,
        trust: EvidenceTrust,
        summary: &str,
    ) -> Self {
        let scrubbed = scrub_secrets(summary);
        let capped = truncate_utf8(&scrubbed, Self::MAX_SUMMARY_BYTES);
        Self {
            id: id.into(),
            source_kind,
            source_ref: scrub_secrets(&source_ref.into()),
            trust,
            summary: capped,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DecisionOption {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub action_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActionDescriptor {
    pub action_id: String,
    pub action_class: ActionClass,
    pub risk: RiskLevel,
    #[serde(default)]
    pub target_path: Option<String>,
}

/// Trusted policy/action frame — NEVER populated from workspace files,
/// comments, diagnostics, or workspace rule overrides.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TrustedFrame {
    pub question: String,
    pub options: Vec<DecisionOption>,
    pub allowed_actions: Vec<ActionDescriptor>,
    pub permission_mode: PermissionMode,
    pub rules_digest: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionRequest {
    pub request_id: DecisionRequestId,
    pub task_type: String,
    pub snapshot: WorkspaceSnapshot,
    pub trusted_frame: TrustedFrame,
    pub untrusted_evidence: Vec<EvidenceItem>,
    pub deadline_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValidationError {
    pub code: String,
    pub field: String,
    pub message: String,
}

impl ValidationError {
    pub fn new(
        code: impl Into<String>,
        field: impl Into<String>,
        message: impl AsRef<str>,
    ) -> Self {
        Self {
            code: code.into(),
            field: field.into(),
            message: scrub_secrets(message.as_ref()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionResult {
    pub provider: String,
    pub model: String,
    pub decision_type: DecisionType,
    pub value: serde_json::Value,
    pub probabilities: BTreeMap<String, f64>,
    pub raw_confidence: f64,
    pub evidence_refs: Vec<String>,
    pub risk: RiskLevel,
    pub latency_ms: u64,
    pub reasoning_summary: Option<String>,
    pub validation_errors: Vec<ValidationError>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderLocality {
    Local,
    Cloud,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionTier {
    Tier1Deterministic,
    Tier2DecisionModel,
    Tier3GeneralLlm,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionPhase {
    Snapshotted,
    Routed,
    Inferred,
    SchemaValidated,
    PolicyEvaluated,
    ApprovalPending,
    SnapshotRechecked,
    Executed,
    Verified,
    Stale,
    Escalated,
    Rejected,
    Cancelled,
    Failed,
}

impl DecisionPhase {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Verified
                | Self::Stale
                | Self::Escalated
                | Self::Rejected
                | Self::Cancelled
                | Self::Failed
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoutingExplanation {
    pub trace_id: String,
    pub selected_tier: ExecutionTier,
    pub candidate_tiers: Vec<ExecutionTier>,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub hop_count: u8,
    pub privacy_blocked_cloud: bool,
    pub reason_code: String,
    pub calibrated_threshold: Option<f64>,
    pub raw_confidence: Option<f64>,
    pub fsm_path: Vec<DecisionPhase>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleScope {
    Global,
    Workspace,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuleSource {
    pub path: String,
    pub scope: RuleScope,
    pub active: bool,
    pub reason: Option<String>,
    pub digest: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ProviderError {
    #[error("decision provider call cancelled")]
    Cancelled,
    #[error("decision provider timed out after {elapsed_ms}ms")]
    Timeout { elapsed_ms: u64 },
    #[error("decision provider returned invalid output: {0}")]
    InvalidOutput(String),
    #[error("decision provider unavailable: {0}")]
    Unavailable(String),
    #[error("decision provider transport error: {0}")]
    Transport(String),
}

impl ProviderError {
    pub fn invalid_output(msg: impl AsRef<str>) -> Self {
        Self::InvalidOutput(scrub_secrets(msg.as_ref()))
    }

    pub fn unavailable(msg: impl AsRef<str>) -> Self {
        Self::Unavailable(scrub_secrets(msg.as_ref()))
    }

    pub fn transport(msg: impl AsRef<str>) -> Self {
        Self::Transport(scrub_secrets(msg.as_ref()))
    }
}

fn truncate_utf8(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_owned();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_owned()
}
