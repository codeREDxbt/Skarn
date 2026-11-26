use crate::types::DecisionPhase;

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("invalid FSM transition from {from:?} to {to:?}")]
pub struct FsmTransitionError {
    pub from: DecisionPhase,
    pub to: DecisionPhase,
}

#[derive(Clone, Debug)]
pub struct DecisionFsm {
    current: DecisionPhase,
    history: Vec<DecisionPhase>,
}

impl DecisionFsm {
    pub fn new() -> Self {
        Self {
            current: DecisionPhase::Snapshotted,
            history: vec![DecisionPhase::Snapshotted],
        }
    }

    pub fn current(&self) -> DecisionPhase {
        self.current
    }

    pub fn history(&self) -> &[DecisionPhase] {
        &self.history
    }

    pub fn transition(&mut self, next: DecisionPhase) -> Result<(), FsmTransitionError> {
        if !is_allowed_transition(self.current, next) {
            return Err(FsmTransitionError {
                from: self.current,
                to: next,
            });
        }
        self.current = next;
        self.history.push(next);
        Ok(())
    }
}

impl Default for DecisionFsm {
    fn default() -> Self {
        Self::new()
    }
}

pub fn is_allowed_transition(from: DecisionPhase, to: DecisionPhase) -> bool {
    use DecisionPhase::*;
    match from {
        Snapshotted => matches!(to, Routed | Cancelled | Rejected),
        Routed => matches!(to, Inferred | SchemaValidated | Escalated | Cancelled | Rejected),
        Inferred => matches!(to, SchemaValidated | Escalated | Cancelled | Rejected),
        SchemaValidated => matches!(to, PolicyEvaluated | Rejected | Escalated | Cancelled),
        PolicyEvaluated => matches!(
            to,
            ApprovalPending | SnapshotRechecked | Rejected | Escalated | Cancelled
        ),
        ApprovalPending => matches!(to, SnapshotRechecked | Rejected | Cancelled),
        SnapshotRechecked => matches!(to, Executed | Stale | Rejected | Cancelled),
        Executed => matches!(to, Verified | Failed),
        Verified | Stale | Escalated | Rejected | Cancelled | Failed => false,
    }
}
