use skarn_protocol::{
    AgentSessionLiveDto, AgentStateDto, AgentTimelineEntryDto, AgentTimelineEventDto, ApprovalId,
    ApprovalNeededDto, SessionId, SubagentActivityDto, SubagentRunState, SubagentSnapshotDto,
};
use skarn_protocol::{EventEnvelope, FrontendEvent};
use std::collections::BTreeMap;

/// Materialized presentation state. Unlike replay, this never evicts token prefixes.
#[derive(Default)]
pub(crate) struct AgentsProjection {
    sessions: BTreeMap<SessionId, SessionProjection>,
}

#[derive(Default)]
struct SessionProjection {
    timeline: Vec<AgentTimelineEntryDto>,
    state: Option<AgentStateDto>,
    approvals: BTreeMap<ApprovalId, ApprovalNeededDto>,
    children: BTreeMap<String, SubagentSnapshotDto>,
}

impl AgentsProjection {
    pub(crate) fn consume_approval(&mut self, approval_id: &ApprovalId) {
        for session in self.sessions.values_mut() {
            session.approvals.remove(approval_id);
        }
    }

    pub(crate) fn apply(&mut self, envelope: &EventEnvelope<FrontendEvent>) {
        use FrontendEvent::*;
        let id = match &envelope.payload {
            SubagentActivity(activity) => match activity {
                SubagentActivityDto::Started {
                    parent_session_id, ..
                }
                | SubagentActivityDto::Delta {
                    parent_session_id, ..
                }
                | SubagentActivityDto::ToolCallStarted {
                    parent_session_id, ..
                }
                | SubagentActivityDto::ToolResult {
                    parent_session_id, ..
                }
                | SubagentActivityDto::StateChanged {
                    parent_session_id, ..
                } => Some(parent_session_id),
            },
            SubagentCompleted(dto) => Some(&dto.parent_session_id),
            SubagentCancelled(dto) => Some(&dto.parent_session_id),
            _ => envelope.session_id.as_ref(),
        };
        let Some(id) = id else {
            return;
        };
        let session = self.sessions.entry(id.clone()).or_default();
        let timeline_event = match &envelope.payload {
            ChatDelta(delta) => {
                if let Some(previous) =
                    session
                        .timeline
                        .iter_mut()
                        .find_map(|entry| match &mut entry.event {
                            AgentTimelineEventDto::Message(previous)
                                if previous.message_id == delta.message_id =>
                            {
                                Some(previous)
                            }
                            _ => None,
                        })
                {
                    if delta.sequence > previous.sequence {
                        previous.text.push_str(&delta.text);
                        previous.sequence = delta.sequence;
                    }
                    return;
                }
                Some(AgentTimelineEventDto::Message(delta.clone()))
            }
            ToolCallStarted(dto) => Some(AgentTimelineEventDto::ToolCallStarted(dto.clone())),
            ToolResult(dto) => Some(AgentTimelineEventDto::ToolResult(dto.clone())),
            ChatCompleted(dto) => {
                session.approvals.retain(|_, approval| {
                    approval.request_id != dto.request_id || approval.subagent_id.is_some()
                });
                Some(AgentTimelineEventDto::ChatCompleted(dto.clone()))
            }
            ChatFailed(dto) => Some(AgentTimelineEventDto::ChatFailed(dto.clone())),
            AgentTurnDone(dto) => {
                session.approvals.retain(|_, approval| {
                    approval.request_id != dto.request_id || approval.subagent_id.is_some()
                });
                if let Some(state) = session.state.as_mut() {
                    state.state = skarn_protocol::AgentPhase::Done;
                    state.tool_name = None;
                }
                Some(AgentTimelineEventDto::AgentTurnDone(dto.clone()))
            }
            AgentStateChanged(dto) => {
                session.state = Some(dto.clone());
                None
            }
            ApprovalNeeded(dto) => {
                session
                    .approvals
                    .insert(dto.approval_id.clone(), dto.clone());
                None
            }
            QuestionCountdownUpdated(dto) => {
                if dto.timeout.timed_out {
                    session.approvals.remove(&dto.approval_id);
                } else if let Some(approval) = session.approvals.get_mut(&dto.approval_id) {
                    approval.question_timeout = Some(dto.timeout.clone());
                }
                None
            }
            SubagentActivity(activity) => {
                let child_id = match activity {
                    SubagentActivityDto::Started { child_id, .. }
                    | SubagentActivityDto::Delta { child_id, .. }
                    | SubagentActivityDto::ToolCallStarted { child_id, .. }
                    | SubagentActivityDto::ToolResult { child_id, .. }
                    | SubagentActivityDto::StateChanged { child_id, .. } => child_id,
                };
                if matches!(activity, SubagentActivityDto::Started { .. }) {
                    session.children.entry(child_id.clone()).or_insert_with(|| {
                        SubagentSnapshotDto {
                            started: activity.clone(),
                            state: SubagentRunState::Thinking,
                            tool_name: None,
                            timeline: Vec::new(),
                            completed: None,
                            cancelled: false,
                        }
                    });
                } else if let Some(child) = session.children.get_mut(child_id) {
                    match activity {
                        SubagentActivityDto::Delta { message_id, sequence, text, .. } => {
                            if let Some(SubagentActivityDto::Delta { sequence: old_sequence, text: old_text, .. }) = child.timeline.iter_mut().find(|entry| matches!(entry, SubagentActivityDto::Delta { message_id: old_id, .. } if old_id == message_id)) {
                                if sequence > old_sequence { old_text.push_str(text); *old_sequence = *sequence; }
                            } else { child.timeline.push(activity.clone()); }
                        }
                        SubagentActivityDto::StateChanged { state, tool_name, .. } => { child.state = *state; child.tool_name = tool_name.clone(); }
                        _ => child.timeline.push(activity.clone()),
                    }
                }
                None
            }
            SubagentCompleted(dto) => {
                if let Some(child) = session.children.get_mut(&dto.child_id) {
                    child.completed = Some(dto.clone());
                    child.state = SubagentRunState::Done;
                    child.tool_name = None;
                }
                session
                    .approvals
                    .retain(|_, approval| approval.subagent_id.as_ref() != Some(&dto.child_id));
                None
            }
            SubagentCancelled(dto) => {
                if let Some(child) = session.children.get_mut(&dto.child_id) {
                    child.cancelled = true;
                    child.state = SubagentRunState::Done;
                    child.tool_name = None;
                }
                session
                    .approvals
                    .retain(|_, approval| approval.subagent_id.as_ref() != Some(&dto.child_id));
                None
            }
            _ => None,
        };
        if let Some(event) = timeline_event {
            session.timeline.push(AgentTimelineEntryDto {
                sequence: envelope.sequence,
                event,
            });
        }
    }

    pub(crate) fn hydrate(&self, sessions: &mut [AgentSessionLiveDto]) {
        for session in sessions {
            if let Some(projection) = self.sessions.get(&session.session_id) {
                session.timeline = projection.timeline.clone();
                session.state = projection.state.clone();
                session.subagents = projection.children.values().cloned().collect();
                for approval in projection.approvals.values() {
                    if !session
                        .approvals
                        .iter()
                        .any(|existing| existing.approval_id == approval.approval_id)
                    {
                        session.approvals.push(approval.clone());
                    }
                }
            }
        }
    }
}

/// Replay helper retained for the event-stream slice; the empty shell has no producer yet.
#[allow(dead_code)]
pub fn replay_after(
    events: &[EventEnvelope<FrontendEvent>],
    sequence: u64,
) -> Vec<EventEnvelope<FrontendEvent>> {
    events
        .iter()
        .filter(|event| event.sequence > sequence)
        .cloned()
        .collect()
}
