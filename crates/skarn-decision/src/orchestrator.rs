use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use crate::fsm::DecisionFsm;
use crate::policy::{evaluate_policy, CalibrationTable, PolicyVerdict, ScopedPolicyStore};
use crate::providers::{DecisionCancelToken, DecisionProvider};
use crate::scrub_secrets;
use crate::snapshot::{DriftReason, SnapshotRechecker, WorkspaceStateReader};
use crate::types::{
    ActionClass, DecisionPhase, DecisionRequest, DecisionResult, DecisionType, ExecutionTier,
    ProviderError, ProviderLocality, RiskLevel, RoutingExplanation, ValidationError,
};
use crate::validator::{validate_decision_result, ActionRegistry, ValidatedAction, ValidatedDecision};

pub const MAX_DECISION_HOPS: u8 = 2;
pub const TIER1_DEADLINE_MS: u64 = 250;
pub const TIER2_MAX_DEADLINE_MS: u64 = 1500;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalResponse {
    ApproveOnce,
    Deny,
}

#[async_trait::async_trait]
pub trait ActionExecutor: Send + Sync {
    async fn execute(
        &self,
        action: &ValidatedAction,
        request: &DecisionRequest,
    ) -> Result<String, String>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecisionOutcome {
    pub phase: DecisionPhase,
    pub explanation: RoutingExplanation,
    pub validated_decision: Option<ValidatedDecision>,
    pub validation_errors: Vec<ValidationError>,
    pub drift_reason: Option<DriftReason>,
    pub execution_output: Option<String>,
    pub executed: bool,
}

pub struct DecisionOrchestrator {
    registry: ActionRegistry,
    calibration: CalibrationTable,
    providers: Vec<Arc<dyn DecisionProvider>>,
}

impl DecisionOrchestrator {
    pub fn new(calibration: CalibrationTable, providers: Vec<Arc<dyn DecisionProvider>>) -> Self {
        Self {
            registry: ActionRegistry::default(),
            calibration,
            providers,
        }
    }

    pub fn with_registry(mut self, registry: ActionRegistry) -> Self {
        self.registry = registry;
        self
    }

    pub fn registry(&self) -> &ActionRegistry {
        &self.registry
    }

    /// Evaluate a `DecisionRequest` through the explicit Decision Lifecycle FSM:
    /// `Snapshotted -> Routed -> [Inferred] -> SchemaValidated -> PolicyEvaluated -> [ApprovalPending] -> SnapshotRechecked -> Executed -> Verified`
    /// with terminal aborts `Stale | Escalated | Rejected | Cancelled | Failed`.
    pub async fn evaluate(
        &self,
        request: &DecisionRequest,
        state_reader: &dyn WorkspaceStateReader,
        policy_store: &ScopedPolicyStore,
        approval: Option<ApprovalResponse>,
        executor: Option<&dyn ActionExecutor>,
        cancel: &DecisionCancelToken,
    ) -> DecisionOutcome {
        self.evaluate_internal(
            request,
            None,
            state_reader,
            policy_store,
            approval,
            executor,
            cancel,
        )
        .await
    }

    /// Evaluate a deterministic Tier 1 action proposal (0 model hops) or route
    /// through Tier 2 providers if not Tier 1 eligible.
    pub async fn evaluate_with_tier1_proposal(
        &self,
        request: &DecisionRequest,
        tier1_proposal: Option<DecisionResult>,
        state_reader: &dyn WorkspaceStateReader,
        policy_store: &ScopedPolicyStore,
        approval: Option<ApprovalResponse>,
        executor: Option<&dyn ActionExecutor>,
        cancel: &DecisionCancelToken,
    ) -> DecisionOutcome {
        self.evaluate_internal(
            request,
            tier1_proposal,
            state_reader,
            policy_store,
            approval,
            executor,
            cancel,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn evaluate_internal(
        &self,
        request: &DecisionRequest,
        tier1_proposal: Option<DecisionResult>,
        state_reader: &dyn WorkspaceStateReader,
        policy_store: &ScopedPolicyStore,
        approval: Option<ApprovalResponse>,
        executor: Option<&dyn ActionExecutor>,
        cancel: &DecisionCancelToken,
    ) -> DecisionOutcome {
        let trace_id = format!("trace-{}", Uuid::new_v4());
        let mut fsm = DecisionFsm::new();
        let mut hop_count: u8;
        let mut privacy_blocked_cloud = false;
        let mut candidate_tiers = Vec::new();
        let mut selected_tier = ExecutionTier::Tier2DecisionModel;
        let mut provider_id: Option<String> = None;
        let mut model_id: Option<String> = None;
        let mut calibrated_threshold: Option<f64> = None;
        let mut raw_confidence: Option<f64> = None;

        let build_outcome = |phase: DecisionPhase,
                             selected_tier: ExecutionTier,
                             candidate_tiers: Vec<ExecutionTier>,
                             provider_id: Option<String>,
                             model_id: Option<String>,
                             hop_count: u8,
                             privacy_blocked_cloud: bool,
                             reason_code: String,
                             calibrated_threshold: Option<f64>,
                             raw_confidence: Option<f64>,
                             fsm: &DecisionFsm,
                             validated_decision: Option<ValidatedDecision>,
                             validation_errors: Vec<ValidationError>,
                             drift_reason: Option<DriftReason>,
                             execution_output: Option<String>,
                             executed: bool| -> DecisionOutcome {
            let scrubbed_errors = validation_errors
                .into_iter()
                .map(|e| ValidationError::new(e.code, e.field, e.message))
                .collect();
            DecisionOutcome {
                phase,
                explanation: RoutingExplanation {
                    trace_id: scrub_secrets(&trace_id),
                    selected_tier,
                    candidate_tiers,
                    provider_id: provider_id.map(|s| scrub_secrets(&s)),
                    model_id: model_id.map(|s| scrub_secrets(&s)),
                    hop_count: hop_count.min(MAX_DECISION_HOPS),
                    privacy_blocked_cloud,
                    reason_code: scrub_secrets(&reason_code),
                    calibrated_threshold,
                    raw_confidence,
                    fsm_path: fsm.history().to_vec(),
                },
                validated_decision,
                validation_errors: scrubbed_errors,
                drift_reason,
                execution_output: execution_output.map(|s| scrub_secrets(&s)),
                executed,
            }
        };

        if cancel.is_cancelled() {
            let _ = fsm.transition(DecisionPhase::Cancelled);
            return build_outcome(
                DecisionPhase::Cancelled,
                ExecutionTier::Tier1Deterministic,
                candidate_tiers,
                None,
                None,
                0,
                false,
                "cancelled_before_routing".to_owned(),
                None,
                None,
                &fsm,
                None,
                Vec::new(),
                None,
                None,
                false,
            );
        }

        let _ = fsm.transition(DecisionPhase::Routed);

        // Tier 1 deterministic fast-path check
        let is_tier1 = request.task_type.starts_with("tier1")
            || tier1_proposal.is_some()
                && request
                    .trusted_frame
                    .allowed_actions
                    .iter()
                    .all(|a| matches!(a.action_class, ActionClass::ReadInspect | ActionClass::CommandAllowlisted));

        let (result_for_validation, used_tier2_provider) = if is_tier1 {
            candidate_tiers.push(ExecutionTier::Tier1Deterministic);
            selected_tier = ExecutionTier::Tier1Deterministic;
            hop_count = 0;

            let synthetic = match tier1_proposal {
                Some(p) => p,
                None => build_tier1_deterministic_result(request),
            };
            let _ = fsm.transition(DecisionPhase::SchemaValidated);
            (synthetic, None)
        } else {
            candidate_tiers.push(ExecutionTier::Tier2DecisionModel);
            candidate_tiers.push(ExecutionTier::Tier3GeneralLlm);

            let mut eligible: Option<Arc<dyn DecisionProvider>> = None;
            for p in &self.providers {
                if request.snapshot.local_only && p.locality() == ProviderLocality::Cloud {
                    privacy_blocked_cloud = true;
                    continue;
                }
                eligible = Some(Arc::clone(p));
                break;
            }

            let Some(provider) = eligible else {
                selected_tier = ExecutionTier::Tier3GeneralLlm;
                hop_count = 1;
                let reason = if privacy_blocked_cloud {
                    "blocked_by_local_only".to_owned()
                } else {
                    "no_eligible_tier2_provider".to_owned()
                };
                let _ = fsm.transition(DecisionPhase::Escalated);
                return build_outcome(
                    DecisionPhase::Escalated,
                    selected_tier,
                    candidate_tiers,
                    None,
                    None,
                    hop_count,
                    privacy_blocked_cloud,
                    reason,
                    None,
                    None,
                    &fsm,
                    None,
                    Vec::new(),
                    None,
                    None,
                    false,
                );
            };

            provider_id = Some(provider.id().to_owned());
            hop_count = 1;
            let _ = fsm.transition(DecisionPhase::Inferred);

            let deadline = Duration::from_millis(request.deadline_ms.clamp(1, TIER2_MAX_DEADLINE_MS));
            let infer_res = tokio::select! {
                _ = cancel.cancelled() => Err(ProviderError::Cancelled),
                res = tokio::time::timeout(deadline, provider.decide(request, cancel)) => match res {
                    Ok(inner) => inner,
                    Err(_) => Err(ProviderError::Timeout { elapsed_ms: deadline.as_millis() as u64 }),
                }
            };

            match infer_res {
                Ok(res) => {
                    model_id = Some(res.model.clone());
                    raw_confidence = Some(res.raw_confidence);
                    let _ = fsm.transition(DecisionPhase::SchemaValidated);
                    (res, Some(provider.id().to_owned()))
                }
                Err(ProviderError::Cancelled) => {
                    let _ = fsm.transition(DecisionPhase::Cancelled);
                    return build_outcome(
                        DecisionPhase::Cancelled,
                        ExecutionTier::Tier2DecisionModel,
                        candidate_tiers,
                        provider_id,
                        model_id,
                        hop_count,
                        privacy_blocked_cloud,
                        "cancelled_during_inference".to_owned(),
                        None,
                        None,
                        &fsm,
                        None,
                        Vec::new(),
                        None,
                        None,
                        false,
                    );
                }
                Err(err) => {
                    selected_tier = ExecutionTier::Tier3GeneralLlm;
                    hop_count = MAX_DECISION_HOPS;
                    let _ = fsm.transition(DecisionPhase::Escalated);
                    return build_outcome(
                        DecisionPhase::Escalated,
                        selected_tier,
                        candidate_tiers,
                        provider_id,
                        model_id,
                        hop_count,
                        privacy_blocked_cloud,
                        format!("tier2_escalated: {err}"),
                        None,
                        None,
                        &fsm,
                        None,
                        Vec::new(),
                        None,
                        None,
                        false,
                    );
                }
            }
        };

        // Schema, Registry, Capability, and Path Validation
        let validated = match validate_decision_result(request, &result_for_validation, &self.registry) {
            Ok(v) => v,
            Err(errs) => {
                let _ = fsm.transition(DecisionPhase::Rejected);
                return build_outcome(
                    DecisionPhase::Rejected,
                    selected_tier,
                    candidate_tiers,
                    provider_id,
                    model_id,
                    hop_count,
                    privacy_blocked_cloud,
                    "schema_or_capability_validation_failed".to_owned(),
                    None,
                    raw_confidence,
                    &fsm,
                    None,
                    errs,
                    None,
                    None,
                    false,
                );
            }
        };

        let _ = fsm.transition(DecisionPhase::PolicyEvaluated);

        // Policy evaluation
        let (action_class, effective_risk, target, is_mutating, target_paths) = match &validated {
            ValidatedDecision::Action(act) => {
                let track_snapshot_file = matches!(
                    act.action_id.as_str(),
                    "read_file" | "write_file" | "edit_file" | "delete_file"
                );
                let paths = if track_snapshot_file {
                    act.target_rel_path
                        .as_ref()
                        .map(|p| vec![p.as_str()])
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                (
                    act.action_class,
                    act.effective_risk,
                    act.target_rel_path.clone().unwrap_or_else(|| act.action_id.clone()),
                    act.is_mutating,
                    paths,
                )
            }
            ValidatedDecision::Choice {
                option_id,
                action_id,
                effective_risk,
            } => {
                let class = action_id
                    .as_deref()
                    .and_then(|id| self.registry.get(id))
                    .map(|s| s.action_class)
                    .unwrap_or(ActionClass::ReadInspect);
                (class, *effective_risk, option_id.clone(), false, Vec::new())
            }
            ValidatedDecision::Binary { effective_risk, .. }
            | ValidatedDecision::Score { effective_risk, .. } => (
                ActionClass::ReadInspect,
                *effective_risk,
                request.task_type.clone(),
                false,
                Vec::new(),
            ),
        };

        let verdict = evaluate_policy(
            request.trusted_frame.permission_mode,
            &request.snapshot,
            action_class,
            effective_risk,
            &target,
            used_tier2_provider.as_deref(),
            if used_tier2_provider.is_some() {
                raw_confidence
            } else {
                None
            },
            &self.calibration,
            policy_store,
        );

        match verdict {
            PolicyVerdict::Reject { reason_code } => {
                let _ = fsm.transition(DecisionPhase::Rejected);
                return build_outcome(
                    DecisionPhase::Rejected,
                    selected_tier,
                    candidate_tiers,
                    provider_id,
                    model_id,
                    hop_count,
                    privacy_blocked_cloud,
                    reason_code,
                    calibrated_threshold,
                    raw_confidence,
                    &fsm,
                    Some(validated),
                    Vec::new(),
                    None,
                    None,
                    false,
                );
            }
            PolicyVerdict::Escalate {
                reason_code,
                calibrated_threshold: thresh,
            } => {
                calibrated_threshold = thresh;
                selected_tier = ExecutionTier::Tier3GeneralLlm;
                hop_count = hop_count.saturating_add(1).min(MAX_DECISION_HOPS);
                let _ = fsm.transition(DecisionPhase::Escalated);
                return build_outcome(
                    DecisionPhase::Escalated,
                    selected_tier,
                    candidate_tiers,
                    provider_id,
                    model_id,
                    hop_count,
                    privacy_blocked_cloud,
                    reason_code,
                    calibrated_threshold,
                    raw_confidence,
                    &fsm,
                    Some(validated),
                    Vec::new(),
                    None,
                    None,
                    false,
                );
            }
            PolicyVerdict::RequireApproval { reason_code } => {
                let _ = fsm.transition(DecisionPhase::ApprovalPending);
                if cancel.is_cancelled() {
                    let _ = fsm.transition(DecisionPhase::Cancelled);
                    return build_outcome(
                        DecisionPhase::Cancelled,
                        selected_tier,
                        candidate_tiers,
                        provider_id,
                        model_id,
                        hop_count,
                        privacy_blocked_cloud,
                        "cancelled_while_awaiting_approval".to_owned(),
                        calibrated_threshold,
                        raw_confidence,
                        &fsm,
                        Some(validated),
                        Vec::new(),
                        None,
                        None,
                        false,
                    );
                }
                match approval {
                    None => {
                        return build_outcome(
                            DecisionPhase::ApprovalPending,
                            selected_tier,
                            candidate_tiers,
                            provider_id,
                            model_id,
                            hop_count,
                            privacy_blocked_cloud,
                            reason_code,
                            calibrated_threshold,
                            raw_confidence,
                            &fsm,
                            Some(validated),
                            Vec::new(),
                            None,
                            None,
                            false,
                        );
                    }
                    Some(ApprovalResponse::Deny) => {
                        let _ = fsm.transition(DecisionPhase::Rejected);
                        return build_outcome(
                            DecisionPhase::Rejected,
                            selected_tier,
                            candidate_tiers,
                            provider_id,
                            model_id,
                            hop_count,
                            privacy_blocked_cloud,
                            "user_denied_approval".to_owned(),
                            calibrated_threshold,
                            raw_confidence,
                            &fsm,
                            Some(validated),
                            Vec::new(),
                            None,
                            None,
                            false,
                        );
                    }
                    Some(ApprovalResponse::ApproveOnce) => {
                        let _ = fsm.transition(DecisionPhase::SnapshotRechecked);
                    }
                }
            }
            PolicyVerdict::AutoProceed {
                calibrated_threshold: thresh,
                ..
            } => {
                calibrated_threshold = thresh;
                let _ = fsm.transition(DecisionPhase::SnapshotRechecked);
            }
        }

        // SnapshotRechecked (TOCTOU defense immediately before execution)
        if cancel.is_cancelled() {
            let _ = fsm.transition(DecisionPhase::Cancelled);
            return build_outcome(
                DecisionPhase::Cancelled,
                selected_tier,
                candidate_tiers,
                provider_id,
                model_id,
                hop_count,
                privacy_blocked_cloud,
                "cancelled_before_snapshot_recheck".to_owned(),
                calibrated_threshold,
                raw_confidence,
                &fsm,
                Some(validated),
                Vec::new(),
                None,
                None,
                false,
            );
        }

        if let Err(drift) =
            SnapshotRechecker::recheck(&request.snapshot, state_reader, &target_paths, is_mutating)
        {
            let _ = fsm.transition(DecisionPhase::Stale);
            return build_outcome(
                DecisionPhase::Stale,
                selected_tier,
                candidate_tiers,
                provider_id,
                model_id,
                hop_count,
                privacy_blocked_cloud,
                "snapshot_drifted_mid_flight".to_owned(),
                calibrated_threshold,
                raw_confidence,
                &fsm,
                Some(validated),
                Vec::new(),
                Some(drift),
                None,
                false,
            );
        }

        let _ = fsm.transition(DecisionPhase::Executed);

        if let (Some(exec), ValidatedDecision::Action(ref act)) = (executor, &validated) {
            match exec.execute(act, request).await {
                Ok(out) => {
                    let _ = fsm.transition(DecisionPhase::Verified);
                    build_outcome(
                        DecisionPhase::Verified,
                        selected_tier,
                        candidate_tiers,
                        provider_id,
                        model_id,
                        hop_count,
                        privacy_blocked_cloud,
                        "verified_execution".to_owned(),
                        calibrated_threshold,
                        raw_confidence,
                        &fsm,
                        Some(validated),
                        Vec::new(),
                        None,
                        Some(out),
                        true,
                    )
                }
                Err(err) => {
                    let _ = fsm.transition(DecisionPhase::Failed);
                    build_outcome(
                        DecisionPhase::Failed,
                        selected_tier,
                        candidate_tiers,
                        provider_id,
                        model_id,
                        hop_count,
                        privacy_blocked_cloud,
                        format!("execution_failed: {err}"),
                        calibrated_threshold,
                        raw_confidence,
                        &fsm,
                        Some(validated),
                        Vec::new(),
                        None,
                        None,
                        false,
                    )
                }
            }
        } else {
            let _ = fsm.transition(DecisionPhase::Verified);
            build_outcome(
                DecisionPhase::Verified,
                selected_tier,
                candidate_tiers,
                provider_id,
                model_id,
                hop_count,
                privacy_blocked_cloud,
                "verified_decision".to_owned(),
                calibrated_threshold,
                raw_confidence,
                &fsm,
                Some(validated),
                Vec::new(),
                None,
                None,
                true,
            )
        }
    }
}

fn build_tier1_deterministic_result(request: &DecisionRequest) -> DecisionResult {
    if let Some(action) = request.trusted_frame.allowed_actions.first() {
        let mut args = serde_json::Map::new();
        if let Some(ref target) = action.target_path {
            args.insert("path".to_owned(), serde_json::json!(target));
        }
        let mut probs = BTreeMap::new();
        probs.insert(action.action_id.clone(), 1.0);
        return DecisionResult {
            provider: "tier1-deterministic".to_owned(),
            model: "deterministic-ide".to_owned(),
            decision_type: DecisionType::Action,
            value: serde_json::json!({
                "action_id": action.action_id,
                "args": args,
            }),
            probabilities: probs,
            raw_confidence: 1.0,
            evidence_refs: Vec::new(),
            risk: action.risk,
            latency_ms: 1,
            reasoning_summary: None,
            validation_errors: Vec::new(),
        };
    }

    DecisionResult {
        provider: "tier1-deterministic".to_owned(),
        model: "deterministic-ide".to_owned(),
        decision_type: DecisionType::Binary,
        value: serde_json::json!(true),
        probabilities: BTreeMap::new(),
        raw_confidence: 1.0,
        evidence_refs: Vec::new(),
        risk: RiskLevel::Low,
        latency_ms: 1,
        reasoning_summary: None,
        validation_errors: Vec::new(),
    }
}
