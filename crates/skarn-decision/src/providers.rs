use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;

use crate::types::{
    DecisionRequest, DecisionResult, DecisionType, ProviderError, ProviderLocality, RiskLevel,
    ValidationError,
};
use crate::validator::parse_provider_json_strict;

/// Race-free cooperative cancellation token.
/// Registers `self.inner.notify.notified()` BEFORE checking the atomic flag,
/// avoiding missed-wakeup races.
#[derive(Clone, Default)]
pub struct DecisionCancelToken {
    inner: Arc<CancelInner>,
}

#[derive(Default)]
struct CancelInner {
    cancelled: AtomicBool,
    notify: Notify,
}

impl DecisionCancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        if !self.inner.cancelled.swap(true, Ordering::SeqCst) {
            self.inner.notify.notify_waiters();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    pub async fn cancelled(&self) {
        loop {
            let notified = self.inner.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
            if self.is_cancelled() {
                return;
            }
        }
    }
}

#[async_trait::async_trait]
pub trait DecisionProvider: Send + Sync {
    fn id(&self) -> &str;
    fn locality(&self) -> ProviderLocality;
    async fn decide(
        &self,
        request: &DecisionRequest,
        cancel: &DecisionCancelToken,
    ) -> Result<DecisionResult, ProviderError>;
}

/// Strict parser for raw provider JSON strings.
pub fn parse_provider_json(raw: &str) -> Result<DecisionResult, Vec<ValidationError>> {
    parse_provider_json_strict(raw)
}

/// Deterministic local Tier 2 rule provider for simple closed-ended decisions.
pub struct DeterministicRuleProvider {
    id: String,
}

impl DeterministicRuleProvider {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

#[async_trait::async_trait]
impl DecisionProvider for DeterministicRuleProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn locality(&self) -> ProviderLocality {
        ProviderLocality::Local
    }

    async fn decide(
        &self,
        request: &DecisionRequest,
        cancel: &DecisionCancelToken,
    ) -> Result<DecisionResult, ProviderError> {
        if cancel.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }

        if let Some(action) = request.trusted_frame.allowed_actions.first() {
            let mut args = serde_json::Map::new();
            if let Some(ref target) = action.target_path {
                args.insert("path".to_owned(), serde_json::json!(target));
            }
            let mut probs = BTreeMap::new();
            probs.insert(action.action_id.clone(), 0.90);
            return Ok(DecisionResult {
                provider: self.id.clone(),
                model: "deterministic-rule-v1".to_owned(),
                decision_type: DecisionType::Action,
                value: serde_json::json!({
                    "action_id": action.action_id,
                    "args": args,
                }),
                probabilities: probs,
                raw_confidence: 0.90,
                evidence_refs: request
                    .untrusted_evidence
                    .iter()
                    .take(2)
                    .map(|e| e.id.clone())
                    .collect(),
                risk: action.risk,
                latency_ms: 1,
                reasoning_summary: Some("deterministic rule match".to_owned()),
                validation_errors: Vec::new(),
            });
        }

        if let Some(opt) = request.trusted_frame.options.first() {
            let mut probs = BTreeMap::new();
            probs.insert(opt.id.clone(), 0.85);
            return Ok(DecisionResult {
                provider: self.id.clone(),
                model: "deterministic-rule-v1".to_owned(),
                decision_type: DecisionType::Choice,
                value: serde_json::json!(opt.id),
                probabilities: probs,
                raw_confidence: 0.85,
                evidence_refs: Vec::new(),
                risk: RiskLevel::Low,
                latency_ms: 1,
                reasoning_summary: Some("selected first trusted option".to_owned()),
                validation_errors: Vec::new(),
            });
        }

        Err(ProviderError::unavailable(
            "no trusted action or option available for deterministic rule provider",
        ))
    }
}

/// Scripted mock provider with invocation counting and cooperative cancellation.
pub struct ScriptedMockProvider {
    id: String,
    locality: ProviderLocality,
    delay: Duration,
    result: Result<DecisionResult, ProviderError>,
    invocations: Arc<AtomicUsize>,
    started_notify: Option<Arc<Notify>>,
    finished_cleanly: Arc<AtomicBool>,
}

impl ScriptedMockProvider {
    pub fn new(
        id: impl Into<String>,
        locality: ProviderLocality,
        result: Result<DecisionResult, ProviderError>,
    ) -> Self {
        Self {
            id: id.into(),
            locality,
            delay: Duration::ZERO,
            result,
            invocations: Arc::new(AtomicUsize::new(0)),
            started_notify: None,
            finished_cleanly: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    pub fn with_started_notify(mut self, notify: Arc<Notify>) -> Self {
        self.started_notify = Some(notify);
        self
    }

    pub fn invocation_count(&self) -> usize {
        self.invocations.load(Ordering::SeqCst)
    }

    pub fn finished_cleanly(&self) -> bool {
        self.finished_cleanly.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl DecisionProvider for ScriptedMockProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn locality(&self) -> ProviderLocality {
        self.locality
    }

    async fn decide(
        &self,
        _request: &DecisionRequest,
        cancel: &DecisionCancelToken,
    ) -> Result<DecisionResult, ProviderError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        if let Some(ref notify) = self.started_notify {
            notify.notify_waiters();
        }
        if cancel.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        if !self.delay.is_zero() {
            tokio::select! {
                _ = cancel.cancelled() => {
                    return Err(ProviderError::Cancelled);
                }
                _ = tokio::time::sleep(self.delay) => {}
            }
        }
        self.finished_cleanly.store(true, Ordering::SeqCst);
        self.result.clone()
    }
}

#[derive(Clone, Debug)]
pub enum AdversarialMode {
    RawJson(String),
    Result(DecisionResult),
    Error(ProviderError),
    Sleep(Duration),
}

/// Adversarial provider for property testing malformed payloads, capability
/// escalations, spoofed confidence, secret leaks, and timeouts.
pub struct AdversarialMockProvider {
    id: String,
    locality: ProviderLocality,
    mode: AdversarialMode,
    invocations: Arc<AtomicUsize>,
}

impl AdversarialMockProvider {
    pub fn new(
        id: impl Into<String>,
        locality: ProviderLocality,
        mode: AdversarialMode,
    ) -> Self {
        Self {
            id: id.into(),
            locality,
            mode,
            invocations: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn invocation_count(&self) -> usize {
        self.invocations.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl DecisionProvider for AdversarialMockProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn locality(&self) -> ProviderLocality {
        self.locality
    }

    async fn decide(
        &self,
        _request: &DecisionRequest,
        cancel: &DecisionCancelToken,
    ) -> Result<DecisionResult, ProviderError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        if cancel.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        match &self.mode {
            AdversarialMode::RawJson(raw) => match parse_provider_json(raw) {
                Ok(res) => Ok(res),
                Err(errs) => Ok(DecisionResult {
                    provider: self.id.clone(),
                    model: "adversarial-raw".to_owned(),
                    decision_type: DecisionType::Action,
                    value: serde_json::Value::Null,
                    probabilities: BTreeMap::new(),
                    raw_confidence: 0.0,
                    evidence_refs: Vec::new(),
                    risk: RiskLevel::Low,
                    latency_ms: 1,
                    reasoning_summary: None,
                    validation_errors: errs,
                }),
            },
            AdversarialMode::Result(res) => Ok(res.clone()),
            AdversarialMode::Error(err) => Err(err.clone()),
            AdversarialMode::Sleep(dur) => {
                tokio::select! {
                    _ = cancel.cancelled() => Err(ProviderError::Cancelled),
                    _ = tokio::time::sleep(*dur) => Err(ProviderError::Timeout {
                        elapsed_ms: dur.as_millis() as u64,
                    }),
                }
            }
        }
    }
}
