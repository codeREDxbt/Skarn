use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::snapshot::sha256_bytes;
use crate::types::{ActionClass, PermissionMode, RiskLevel, RuleScope, RuleSource, WorkspaceSnapshot};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalibrationEntry {
    pub provider_id: String,
    pub action_class: ActionClass,
    pub risk: RiskLevel,
    pub min_raw_confidence: f64,
    pub calibrated_accuracy: f64,
}

#[derive(Clone, Debug, Default)]
pub struct CalibrationTable {
    entries: BTreeMap<(String, ActionClass, RiskLevel), CalibrationEntry>,
}

impl CalibrationTable {
    /// Fail-closed threshold for unknown `(provider, action_class, risk)` pairs
    /// or `High`/`Critical` risk actions.
    pub const FAIL_CLOSED_THRESHOLD: f64 = 1.01;

    pub fn insert(&mut self, entry: CalibrationEntry) {
        self.entries.insert(
            (entry.provider_id.clone(), entry.action_class, entry.risk),
            entry,
        );
    }

    pub fn with_default_local(provider_id: &str) -> Self {
        let mut table = Self::default();
        for (class, risk, threshold, acc) in [
            (ActionClass::ReadInspect, RiskLevel::Low, 0.70, 0.96),
            (ActionClass::FileEdit, RiskLevel::Low, 0.78, 0.93),
            (ActionClass::FileEdit, RiskLevel::Medium, 0.84, 0.91),
            (ActionClass::CommandAllowlisted, RiskLevel::Low, 0.85, 0.95),
            (ActionClass::CommandAllowlisted, RiskLevel::Medium, 0.88, 0.92),
        ] {
            table.insert(CalibrationEntry {
                provider_id: provider_id.to_owned(),
                action_class: class,
                risk,
                min_raw_confidence: threshold,
                calibrated_accuracy: acc,
            });
        }
        table
    }

    pub fn threshold(&self, provider_id: &str, action_class: ActionClass, risk: RiskLevel) -> f64 {
        if risk >= RiskLevel::High {
            return Self::FAIL_CLOSED_THRESHOLD;
        }
        self.entries
            .get(&(provider_id.to_owned(), action_class, risk))
            .map(|e| e.min_raw_confidence)
            .unwrap_or(Self::FAIL_CLOSED_THRESHOLD)
    }

    pub fn is_calibrated_pass(
        &self,
        provider_id: &str,
        action_class: ActionClass,
        risk: RiskLevel,
        raw_confidence: f64,
    ) -> (bool, f64) {
        let threshold = self.threshold(provider_id, action_class, risk);
        if !raw_confidence.is_finite() || risk >= RiskLevel::High {
            return (false, threshold);
        }
        (raw_confidence >= threshold, threshold)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantDecision {
    AllowOnce,
    AllowSession,
    AllowProject,
    DenyOnce,
    DenySession,
    DenyProject,
}

impl GrantDecision {
    pub fn is_deny(self) -> bool {
        matches!(self, Self::DenyOnce | Self::DenySession | Self::DenyProject)
    }

    pub fn is_allow(self) -> bool {
        !self.is_deny()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScopedGrantEntry {
    pub action_class: ActionClass,
    pub target_pattern: String,
    pub decision: GrantDecision,
    pub max_risk: RiskLevel,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PolicyError {
    #[error("scoped grant {decision:?} is forbidden on {risk:?} risk actions (only AllowOnce or Deny* permitted)")]
    GrantExceedsRiskCeiling {
        decision: GrantDecision,
        risk: RiskLevel,
    },
    #[error("untrusted workspace rules attempted prohibited policy override: {0}")]
    RulesOverrideForbidden(String),
}

#[derive(Clone, Debug, Default)]
pub struct ScopedPolicyStore {
    grants: Vec<ScopedGrantEntry>,
}

impl ScopedPolicyStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a scoped grant, enforcing Tightening Rule #2:
    /// `AllowSession` and `AllowProject` are strictly forbidden on `High`/`Critical` risk.
    pub fn insert_grant(&mut self, entry: ScopedGrantEntry) -> Result<(), PolicyError> {
        if matches!(
            entry.decision,
            GrantDecision::AllowSession | GrantDecision::AllowProject
        ) && entry.max_risk >= RiskLevel::High
        {
            return Err(PolicyError::GrantExceedsRiskCeiling {
                decision: entry.decision,
                risk: entry.max_risk,
            });
        }
        self.grants.push(entry);
        Ok(())
    }

    /// Reconstruct from persisted project grants, filtering out any malformed
    /// `AllowSession` entries or `AllowProject` entries targeting `High`/`Critical`.
    pub fn load_persisted(entries: Vec<ScopedGrantEntry>) -> Self {
        let grants = entries
            .into_iter()
            .filter(|e| {
                !matches!(e.decision, GrantDecision::AllowSession)
                    && !(matches!(e.decision, GrantDecision::AllowProject)
                        && e.max_risk >= RiskLevel::High)
            })
            .collect();
        Self { grants }
    }

    pub fn entries(&self) -> &[ScopedGrantEntry] {
        &self.grants
    }

    pub fn revoke(&mut self, action_class: ActionClass, target_pattern: &str) {
        self.grants
            .retain(|g| !(g.action_class == action_class && g.target_pattern == target_pattern));
    }

    /// Evaluate grants for `(action_class, target, risk)`.
    /// `Deny*` ALWAYS overrides `Allow*`. Persistent/session allows never match `High`/`Critical`.
    pub fn evaluate(
        &self,
        action_class: ActionClass,
        target: &str,
        risk: RiskLevel,
    ) -> Option<GrantDecision> {
        let mut allow_match: Option<GrantDecision> = None;

        for grant in &self.grants {
            if grant.action_class != action_class || !pattern_matches(&grant.target_pattern, target)
            {
                continue;
            }
            if grant.decision.is_deny() {
                return Some(grant.decision);
            }
            // Fail-closed defense even if memory was mutated without `insert_grant`:
            if risk >= RiskLevel::High
                && matches!(
                    grant.decision,
                    GrantDecision::AllowSession | GrantDecision::AllowProject
                )
            {
                continue;
            }
            if risk <= grant.max_risk && allow_match.is_none() {
                allow_match = Some(grant.decision);
            }
        }

        allow_match
    }
}

fn pattern_matches(pattern: &str, target: &str) -> bool {
    let pat = pattern.trim();
    if pat == "*" || pat.is_empty() {
        return true;
    }
    if let Some(prefix) = pat.strip_suffix('*') {
        return target.starts_with(prefix);
    }
    pat == target
}

#[derive(Clone, Debug, PartialEq)]
pub enum PolicyVerdict {
    AutoProceed {
        reason_code: String,
        calibrated_threshold: Option<f64>,
    },
    RequireApproval {
        reason_code: String,
    },
    Reject {
        reason_code: String,
    },
    Escalate {
        reason_code: String,
        calibrated_threshold: Option<f64>,
    },
}

/// Evaluate the 4-mode × 4-risk asymmetric matrix + calibration + scoped grants.
///
/// Guarantees:
/// - `High` / `Critical` risk: **0% silent auto-exec** regardless of mode, confidence (`1.0`), or grants.
/// - `Deny*` scoped grants always override `Allow*`.
/// - `ReadOnly` rejects all mutations and commands.
/// - Untrusted workspaces never auto-run commands.
pub fn evaluate_policy(
    mode: PermissionMode,
    snapshot: &WorkspaceSnapshot,
    action_class: ActionClass,
    risk: RiskLevel,
    target: &str,
    provider_id: Option<&str>,
    raw_confidence: Option<f64>,
    calibration: &CalibrationTable,
    scoped_store: &ScopedPolicyStore,
) -> PolicyVerdict {
    let grant = scoped_store.evaluate(action_class, target, risk);
    if let Some(g) = grant {
        if g.is_deny() {
            return PolicyVerdict::Reject {
                reason_code: "denied_by_scoped_grant".to_owned(),
            };
        }
    }

    if mode == PermissionMode::ReadOnly
        && !matches!(action_class, ActionClass::ReadInspect | ActionClass::Question)
    {
        return PolicyVerdict::Reject {
            reason_code: "readonly_forbids_mutation".to_owned(),
        };
    }

    if risk >= RiskLevel::High {
        return PolicyVerdict::RequireApproval {
            reason_code: "high_risk_requires_approval".to_owned(),
        };
    }

    if action_class == ActionClass::Question {
        return PolicyVerdict::RequireApproval {
            reason_code: "user_question_requires_response".to_owned(),
        };
    }

    let check_calibrated = || -> Result<Option<f64>, f64> {
        match (provider_id, raw_confidence) {
            (Some(pid), Some(conf)) => {
                let (ok, thresh) = calibration.is_calibrated_pass(pid, action_class, risk, conf);
                if ok {
                    Ok(Some(thresh))
                } else {
                    Err(thresh)
                }
            }
            // Tier 1 deterministic fast-path (no provider inference needed)
            (None, None) => Ok(None),
            _ => Err(CalibrationTable::FAIL_CLOSED_THRESHOLD),
        }
    };

    match action_class {
        ActionClass::ReadInspect => match check_calibrated() {
            Ok(thresh) => PolicyVerdict::AutoProceed {
                reason_code: "low_risk_read_auto".to_owned(),
                calibrated_threshold: thresh,
            },
            Err(thresh) => PolicyVerdict::Escalate {
                reason_code: "uncalibrated_or_low_confidence_read".to_owned(),
                calibrated_threshold: Some(thresh),
            },
        },
        ActionClass::FileEdit => match mode {
            PermissionMode::ReadOnly => PolicyVerdict::Reject {
                reason_code: "readonly_forbids_edit".to_owned(),
            },
            PermissionMode::AskBeforeChanges => {
                if matches!(
                    grant,
                    Some(GrantDecision::AllowSession | GrantDecision::AllowProject)
                ) {
                    PolicyVerdict::AutoProceed {
                        reason_code: "scoped_grant_allows_edit".to_owned(),
                        calibrated_threshold: None,
                    }
                } else {
                    PolicyVerdict::RequireApproval {
                        reason_code: "ask_before_changes_edit".to_owned(),
                    }
                }
            }
            PermissionMode::AutoEdit | PermissionMode::FullAccess => match check_calibrated() {
                Ok(thresh) => PolicyVerdict::AutoProceed {
                    reason_code: "auto_edit_checkpointed".to_owned(),
                    calibrated_threshold: thresh,
                },
                Err(thresh) => PolicyVerdict::Escalate {
                    reason_code: "uncalibrated_edit_escalated".to_owned(),
                    calibrated_threshold: Some(thresh),
                },
            },
        },
        ActionClass::CommandAllowlisted => {
            if !snapshot.workspace_trusted {
                return PolicyVerdict::RequireApproval {
                    reason_code: "untrusted_workspace_command".to_owned(),
                };
            }
            match mode {
                PermissionMode::ReadOnly => PolicyVerdict::Reject {
                    reason_code: "readonly_forbids_command".to_owned(),
                },
                PermissionMode::AskBeforeChanges | PermissionMode::AutoEdit => {
                    if matches!(
                        grant,
                        Some(GrantDecision::AllowSession | GrantDecision::AllowProject)
                    ) {
                        PolicyVerdict::AutoProceed {
                            reason_code: "scoped_grant_allows_command".to_owned(),
                            calibrated_threshold: None,
                        }
                    } else {
                        PolicyVerdict::RequireApproval {
                            reason_code: "command_requires_approval".to_owned(),
                        }
                    }
                }
                PermissionMode::FullAccess => match check_calibrated() {
                    Ok(thresh) => PolicyVerdict::AutoProceed {
                        reason_code: "full_access_trusted_command".to_owned(),
                        calibrated_threshold: thresh,
                    },
                    Err(thresh) => PolicyVerdict::Escalate {
                        reason_code: "uncalibrated_command_escalated".to_owned(),
                        calibrated_threshold: Some(thresh),
                    },
                },
            }
        }
        ActionClass::FileDelete
        | ActionClass::CommandGated
        | ActionClass::Workflow
        | ActionClass::PlanControl => PolicyVerdict::RequireApproval {
            reason_code: "gated_action_requires_approval".to_owned(),
        },
        ActionClass::Question => PolicyVerdict::RequireApproval {
            reason_code: "user_question_requires_response".to_owned(),
        },
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RulesLoadResult {
    pub sources: Vec<RuleSource>,
    pub effective_instructions: String,
    pub rules_digest: Option<String>,
}

pub struct RulesLoader;

impl RulesLoader {
    pub const MAX_RULE_BYTES: usize = 64 * 1024;

    /// Load global `AGENTS.md` (always) and workspace `AGENTS.md` / `.skarn/rules`
    /// ONLY when `workspace_trusted == true`.
    pub fn load_rules(
        global_agents_md: Option<&Path>,
        workspace_root: &Path,
        workspace_trusted: bool,
    ) -> RulesLoadResult {
        let mut sources = Vec::new();
        let mut combined = String::new();

        if let Some(global_path) = global_agents_md {
            if let Ok(bytes) = std::fs::read(global_path) {
                let bounded = &bytes[..bytes.len().min(Self::MAX_RULE_BYTES)];
                let text = String::from_utf8_lossy(bounded);
                let digest = sha256_bytes(bounded);
                combined.push_str(&text);
                combined.push('\n');
                sources.push(RuleSource {
                    path: global_path.to_string_lossy().into_owned(),
                    scope: RuleScope::Global,
                    active: true,
                    reason: None,
                    digest: Some(digest),
                });
            }
        }

        let ws_candidates = [
            workspace_root.join("AGENTS.md"),
            workspace_root.join(".skarn").join("rules"),
        ];

        for path in ws_candidates {
            if path.is_file() {
                if !workspace_trusted {
                    sources.push(RuleSource {
                        path: path.to_string_lossy().into_owned(),
                        scope: RuleScope::Workspace,
                        active: false,
                        reason: Some("workspace_untrusted".to_owned()),
                        digest: None,
                    });
                    continue;
                }
                if let Ok(bytes) = std::fs::read(&path) {
                    let bounded = &bytes[..bytes.len().min(Self::MAX_RULE_BYTES)];
                    let text = String::from_utf8_lossy(bounded);
                    let digest = sha256_bytes(bounded);
                    combined.push_str(&text);
                    combined.push('\n');
                    sources.push(RuleSource {
                        path: path.to_string_lossy().into_owned(),
                        scope: RuleScope::Workspace,
                        active: true,
                        reason: None,
                        digest: Some(digest),
                    });
                }
            }
        }

        let rules_digest = if combined.is_empty() {
            None
        } else {
            Some(sha256_bytes(combined.as_bytes()))
        };

        RulesLoadResult {
            sources,
            effective_instructions: combined,
            rules_digest,
        }
    }
}

pub struct RulesBoundaryPolicy;

impl RulesBoundaryPolicy {
    /// Enforce Tightening Rule #1: workspace rules can NEVER widen `PermissionMode`,
    /// downgrade `RiskLevel`, or override `local_only`.
    pub fn enforce_immutable_frame(
        base_mode: PermissionMode,
        requested_mode: PermissionMode,
        base_risk: RiskLevel,
        requested_risk: RiskLevel,
        base_local_only: bool,
        requested_local_only: bool,
    ) -> Result<(), PolicyError> {
        if requested_mode > base_mode {
            return Err(PolicyError::RulesOverrideForbidden(format!(
                "cannot widen PermissionMode from {base_mode:?} to {requested_mode:?}"
            )));
        }
        if requested_risk < base_risk {
            return Err(PolicyError::RulesOverrideForbidden(format!(
                "cannot downgrade RiskLevel from {base_risk:?} to {requested_risk:?}"
            )));
        }
        if base_local_only && !requested_local_only {
            return Err(PolicyError::RulesOverrideForbidden(
                "cannot disable local_only privacy gate".to_owned(),
            ));
        }
        Ok(())
    }
}
