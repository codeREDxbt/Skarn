//! Phase Z3 — the agent dock's engine (docs/ui/PHASE_Z_MIGRATION.md).
//!
//! Owns the chat/agent sessions behind the dock: the in-memory transcripts
//! (the bootstrap's `chat` snapshot), the in-flight requests keyed by
//! `RequestId`, the single-use approval slots, and the two relays that turn
//! `skarn-ai`'s event streams into protocol events on `skarn://event`
//! through [`crate::state::EventLog`].
//!
//! The routing vocabulary is the four `FrontendCommand` arms this slice
//! un-stubs:
//! - `SendChat` → [`SessionEngine::send_chat`] → `SkarnClient::stream_chat`
//!   behind a cancel watch; deltas arrive as `ChatDelta`, the terminal as
//!   `ChatCompleted`/`ChatFailed` (a cancelled stop ends *silently* with the
//!   partial bubble finalized into the transcript — a user stop is never an
//!   error, Phase R1's precedent).
//! - `SendAgent` → [`SessionEngine::send_agent`] →
//!   `skarn_ai::spawn_agent_turn` over `ClientChatStream` +
//!   `RouterToolRunner` (the `ToolContext` rooted at the workspace root, so
//!   every tool path is contained, §17.2). The loop's events map to
//!   `ToolCallStarted` / `ToolResult` / `ApprovalNeeded` /
//!   `AgentStateChanged` / `ChatDelta` / `AgentTurnDone`.
//! - `AgentDecision` → [`SessionEngine::agent_decision`] → the parked
//!   approval is validated and consumed (`validate_and_consume_approval`)
//!   *before* the decision is forwarded on the turn's decision channel —
//!   single-use, strictly in Rust. `Stop` needs no pending card and is
//!   forwarded regardless (§17.4: the Stop button is live for the whole
//!   turn).
//! - `StopChat` routes to either kind: a chat's cancel watch flips (the
//!   request future is dropped, the SSE connection closes); an agent turn's
//!   decision channel gets `Stop` (the loop kills bash children through the
//!   router's abort handle). One stop story.
//!
//! Invariants (the Z3 brief's gate list):
//! - approval ids are consumed exactly once, on the matching decision only;
//! - the API key never crosses into an event (every error string that could
//!   carry it is redacted at the seam);
//! - one in-flight request per session (`session_busy` otherwise);
//! - every event rides the shared monotonic sequence (the `EventLog`).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use skarn_ai::{
    AgentSessionMode, AgentSessionState, AgentSessionStore, AgentSummary, AgentTurnState,
    ApprovalCard, ChatEvent, ChatMessage, ChatReceipt, ChatRequest, CompactionConfig,
    CompactionError, Model, ParkedCard, RouterToolRunner, ToolCapability, ToolContext,
    ToolProbeOutcome, ToolRunner, ToolSpec, TranscriptEntry, TurnCaps, TurnInput,
};
use skarn_core::SkarnError;
use skarn_decision::{
    ActionClass, CheckpointError, DecisionCancelToken, DecisionPhase, ExecutionTier, GrantDecision,
    PermissionMode, RiskLevel, RuleScope, RulesBoundaryPolicy, RulesLoader, ScopedGrantEntry,
    ScopedPolicyStore, TurnCheckpointManifest, TurnCheckpointStore, WorkspaceStateReader,
};
use skarn_protocol::{
    validate_and_consume_approval, ActionClassDto, AgentCompactionFailedDto, AgentModeDto,
    AgentParkSummaryDto, AgentPhase, AgentSessionSummaryDto, AgentStateDto, AgentTurnDoneDto,
    ApprovalDto, ApprovalId, ApprovalNeededDto, AttachmentKindDto, ChatCompletedDto, ChatDeltaDto,
    ChatFailedDto, ChatRequestDto, ChatStateDto, ConnectionStatus, DecisionDto, DecisionPhaseDto,
    DecisionPromptDto, DecisionTraceDto, ExecutionTierDto, FrontendEvent, GrantDecisionDto,
    MessageDto, MessageId, MessageRole, ModelDto, ModelsDto, PlanApprovalDto, PlanPhaseDto,
    PlanStateDto, ProtocolErrorDto, QuestionCountdownUpdatedDto, QuestionPromptOptionDto,
    QuestionTimeoutDto, RequestId, RiskLevelDto, RuleScopeDto, RuleSourceDto, RulesLoadedDto,
    ScopedGrantDto,
    ScopedGrantsChangedDto, SessionDto, SessionId, SkarnStatusDto, SubagentActivityDto,
    SubagentArtifactDto, SubagentCancelledDto, SubagentCompletedDto, SubagentPolicyDto,
    SubagentRunState, ToastDto, ToastSeverity, TodoDto, TodoStatusDto, TodosDto,
    ToolCallStartedDto, ToolCapabilityDto, ToolResultDto, TurnCheckpointFileDto,
    TurnCheckpointSummaryDto, TurnId, UndoConflictDto, WorkflowApprovalDto,
};
use tokio::sync::{mpsc, watch};
use uuid::Uuid;

use crate::state::{workspace_id_for, EventLog};
use crate::workspace::{DesktopWorkspaceReader, LiveTrustState, Workspace};
use skarn_protocol::{
    AgentQueueChangedDto, AgentQueueItemDto, AgentQueueStatusDto, AgentSessionLiveDto,
    AgentSessionRenamedDto,
};

pub(crate) enum QueueMutation {
    Update(RequestId, Box<skarn_protocol::AgentRequestDto>),
    Remove(RequestId),
    Reorder(Vec<RequestId>),
    Clear,
    Resume,
}

pub(crate) fn idempotency_conflict() -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "idempotency_conflict".into(),
        message: "This request identity was already accepted with different content.".into(),
        retryable: false,
    }
}

fn invalid_queue(message: &str) -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "invalid_agent_queue".into(),
        message: message.into(),
        retryable: false,
    }
}

/// The read-only half of the §17.1 manifest — `explore` mode arms exactly
/// these (the model cannot request a side effect). Phase 9b's todo tools
/// are session state, not workspace writes, so they stay available.
const EXPLORE_TOOLS: [&str; 12] = [
    "read_file",
    "list_dir",
    "fuzzy_find",
    "search_files",
    "web_search",
    "web_fetch",
    "todo_read",
    "todo_write",
    "manage_todo",
    "wait",
    "enter_plan_mode",
    "exit_plan_mode",
];

/// Full presentation history is retained; request prompts remain independently bounded.
/// How many trailing transcript messages ride each request (the egui
/// restore's 20-turn precedent keeps prompts bounded without a settings
/// field).
const REQUEST_HISTORY_MESSAGES: usize = 20;

/// Where the gateway key comes from. Production reads the OS keyring at
/// send time (the key's only home, ARCHITECTURE.md §7.2); the hermetic
/// tests pin a fixed key (`None` = first-run, never connected) so no test
/// ever touches the real Credential Manager.
#[derive(Clone)]
pub(crate) enum ApiKeySource {
    Keyring,
    #[cfg(test)]
    Fixed(Option<String>),
    #[cfg(test)]
    Mutable(Arc<Mutex<Result<Option<String>, ProtocolErrorDto>>>),
}

impl ApiKeySource {
    pub(crate) fn get(&self) -> Result<Option<String>, ProtocolErrorDto> {
        match self {
            Self::Keyring => {
                match skarn_settings::keyring::api_key_get() {
                    Ok(Some(key)) => {
                        let trimmed = key.trim();
                        if !trimmed.is_empty() {
                            return Ok(Some(trimmed.to_string()));
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        return Err(ProtocolErrorDto {
                            code: "keyring_unavailable".into(),
                            message: format!("The OS keyring could not be read: {error}"),
                            retryable: true,
                        });
                    }
                }
                if let Some(key) = skarn_settings::env_var_with_legacy_fallback(
                    "SKARN_API_KEY",
                    "SELORA_API_KEY", // skarn-v1-migration-compat
                ) {
                    let trimmed = key.trim();
                    if !trimmed.is_empty() {
                        return Ok(Some(trimmed.to_string()));
                    }
                }
                if let Ok(key) = std::env::var("OPENAI_API_KEY") {
                    let trimmed = key.trim();
                    if !trimmed.is_empty() {
                        return Ok(Some(trimmed.to_string()));
                    }
                }
                Ok(None)
            }
            #[cfg(test)]
            Self::Fixed(key) => Ok(key.clone()),
            #[cfg(test)]
            Self::Mutable(key) => key.lock().unwrap().clone(),
        }
    }

    pub(crate) fn set(&self, value: &str) -> Result<(), ProtocolErrorDto> {
        match self {
            Self::Keyring => skarn_settings::keyring::api_key_set(value).map_err(|err| {
                ProtocolErrorDto {
                    code: "keyring_write_failed".into(),
                    message: format!("Could not save API key to secure credential manager: {err}"),
                    retryable: true,
                }
            }),
            #[cfg(test)]
            Self::Fixed(_) => Err(ProtocolErrorDto {
                code: "keyring_write_failed".into(),
                message: "Cannot overwrite fixed test API key.".into(),
                retryable: false,
            }),
            #[cfg(test)]
            Self::Mutable(slot) => {
                *slot.lock().unwrap() = Ok(Some(value.to_owned()));
                Ok(())
            }
        }
    }

    pub(crate) fn delete(&self) -> Result<(), ProtocolErrorDto> {
        match self {
            Self::Keyring => skarn_settings::keyring::api_key_delete().map_err(|err| {
                ProtocolErrorDto {
                    code: "keyring_delete_failed".into(),
                    message: format!("Could not remove API key from keyring: {err}"),
                    retryable: true,
                }
            }),
            #[cfg(test)]
            Self::Fixed(_) => Ok(()),
            #[cfg(test)]
            Self::Mutable(slot) => {
                *slot.lock().unwrap() = Ok(None);
                Ok(())
            }
        }
    }

    pub(crate) fn is_test_override(&self) -> bool {
        match self {
            Self::Keyring => false,
            #[cfg(test)]
            Self::Fixed(_) | Self::Mutable(_) => true,
        }
    }
}

/// Process-wide cache of runtime tool-capability probe outcomes, keyed by
/// `(normalized_base_url, lowercase_model_id)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CachedCapabilityEntry {
    Supported,
    Unsupported(String),
}

#[derive(Default)]
pub(crate) struct ToolCapabilityCache {
    entries: Mutex<HashMap<(String, String), CachedCapabilityEntry>>,
}

impl ToolCapabilityCache {
    fn key(base_url: &str, model_id: &str) -> (String, String) {
        (
            base_url.trim().trim_end_matches('/').to_ascii_lowercase(),
            model_id.trim().to_ascii_lowercase(),
        )
    }

    pub(crate) fn get(&self, base_url: &str, model_id: &str) -> Option<CachedCapabilityEntry> {
        self.entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&Self::key(base_url, model_id))
            .cloned()
    }

    pub(crate) fn insert(&self, base_url: &str, model_id: &str, entry: CachedCapabilityEntry) {
        self.entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(Self::key(base_url, model_id), entry);
    }

    pub(crate) fn clear_base_url(&self, base_url: &str) {
        let normalized = base_url.trim().trim_end_matches('/').to_ascii_lowercase();
        self.entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|(url, _), _| url != &normalized);
    }
}

/// Everything the engine needs from the loaded settings, extracted once at
/// construction (base URL and the `agent.*` caps are not editable through
/// the web shell in Z3).
pub(crate) struct EngineConfig {
    pub base_url: String,
    pub active_model: Option<String>,
    pub agent: skarn_settings::AgentSettings,
    pub key_source: ApiKeySource,
}

/// One in-flight request. Registered synchronously in `dispatch` (so the
/// busy/approval guards are race-free) and removed by the relay at its
/// terminal event.
enum ActiveRequest {
    Chat {
        session_id: SessionId,
        turn_id: TurnId,
        cancel: watch::Sender<bool>,
    },
    Agent {
        session_id: SessionId,
        turn_id: TurnId,
        decisions: mpsc::UnboundedSender<skarn_ai::TurnDecision>,
    },
}

impl ActiveRequest {
    fn session_id(&self) -> &SessionId {
        match self {
            Self::Chat { session_id, .. } | Self::Agent { session_id, .. } => session_id,
        }
    }
    fn turn_id(&self) -> &TurnId {
        match self {
            Self::Chat { turn_id, .. } | Self::Agent { turn_id, .. } => turn_id,
        }
    }
}

/// One live transcript record waiting for the OS keyring to become readable
/// again. Records are kept in arrival order so the durable transcript never
/// reorders a tool call after the result that answers it.
struct PendingTranscriptRecord {
    session_id: SessionId,
    entry: TranscriptEntry,
}

/// Why a live tool record did not reach disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TranscriptAppend {
    /// The keyring could not be read; the record is queued for retry. The
    /// turn may continue: no durable data was lost.
    Queued,
    /// There is no store, or the append itself failed. The turn must fail
    /// visibly.
    Failed,
}

/// One finalized transcript entry (the streaming half lives in the relay
/// until it lands here at the terminal event).
struct StoredMessage {
    message_id: MessageId,
    role: MessageRole,
    text: String,
    /// How this user message was dispatched, so `RetryMessage` can re-send the
    /// same content through the same path exactly once. Assistant messages
    /// carry `None`.
    retry: Option<RetrySpec>,
    /// Assistant-side tool calls (restored sessions keep the pairing).
    tool_calls: Option<Vec<skarn_ai::ToolCall>>,
    /// The call a tool-result message answers.
    tool_call_id: Option<String>,
    ok: bool,
    timestamp: u64,
}

/// The dispatch path a user message took. Capability fields are copied from
/// the original request — a retry must not silently change mode, model, or
/// reasoning depth.
#[derive(Clone)]
enum RetrySpec {
    Chat {
        model_id: Option<String>,
    },
    Agent {
        model_id: Option<String>,
        mode: AgentModeDto,
        reasoning_effort: Option<String>,
        system_instructions: Option<String>,
        temperature: Option<f64>,
    },
}

/// Which stored user message a retry re-dispatches. The send paths truncate
/// the tail and register the new turn under one engine lock, so a concurrent
/// send cannot slip between the truncation and the dispatch.
#[derive(Clone)]
struct RetryTarget {
    session_id: SessionId,
    message_id: MessageId,
}

struct ChatSession {
    title: String,
    messages: Vec<StoredMessage>,
    active_request: Option<RequestId>,
    /// The turn's parked approval (§17.4) — `None` while none is pending.
    /// Consumed exactly once by a matching `AgentDecision`.
    pending_approval: Option<ApprovalDto>,
    /// Phase 9b: session-scoped plan mode + todos. Shared with the turn's
    /// `ToolContext`, so tool writes land here and the bootstrap hydrates
    /// them after a reload.
    plan_store: skarn_ai::PlanStore,
    /// The parked card itself (needed to persist/restore the resume view).
    pending_card: Option<ApprovalCard>,
    pending_tool_name: Option<String>,
    /// The restored park's underlying card (resume renders it; a decision
    /// expires without reaching any turn).
    restored_park: bool,
    /// Monotonic recency stamp (bootstrap's `active_session_id` = the most
    /// recently touched session).
    last_activity: u64,
    // ---- Phase 9d: durable session fields -----------------------------
    created_at: u64,
    updated_at: u64,
    model: Option<String>,
    mode: AgentModeDto,
    /// Session full access. Deliberately never written to disk (the store
    /// has no field for it) and reset on switch/new/restart.
    auto_approve: bool,
    /// How many `messages` entries have already been appended to the
    /// transcript. Tool-call/tool-result records are appended live and are
    /// not part of this count.
    durable_entries: usize,
    /// `true` for an agent session (persisted under `.selora/agent-sessions`).
    /// Plain chat sessions stay in memory.
    durable: bool,
    /// The last successful compaction cursor (full transcript stays on disk).
    compaction: skarn_ai::CompactionCursor,
    queue: Vec<AgentQueueItemDto>,
    queue_paused: bool,
}

impl ChatSession {
    fn new(title: String, now: u64) -> Self {
        Self {
            title,
            messages: Vec::new(),
            active_request: None,
            pending_approval: None,
            pending_card: None,
            pending_tool_name: None,
            restored_park: false,
            last_activity: now,
            created_at: now,
            updated_at: now,
            model: None,
            mode: AgentModeDto::AskBeforeChanges,
            plan_store: skarn_ai::new_store(),
            auto_approve: false,
            durable_entries: 0,
            durable: false,
            compaction: skarn_ai::CompactionCursor::default(),
            queue: Vec::new(),
            queue_paused: false,
        }
    }
}

/// §19.3: one child card's routing facts. The runtime owns the single-use
/// decision slot; this record lets the engine refuse a stale session or the
/// wrong card type before forwarding.
#[derive(Clone)]
struct SubagentCardRoute {
    request_id: RequestId,
    session_id: SessionId,
    is_question: bool,
    child_id: String,
}

struct EngineInner {
    base_url: String,
    active_model: Option<String>,
    sessions: BTreeMap<SessionId, ChatSession>,
    active: BTreeMap<RequestId, ActiveRequest>,
    models: Vec<Model>,
    connection: ConnectionStatus,
    last_connection_error: Option<String>,
    activity_counter: u64,
    /// The session the user explicitly selected (new/switch/send). While a
    /// turn is live, `chat_state` prefers that turn's session so the frontend
    /// always routes events to the stream it can see.
    active_session: Option<SessionId>,
}

/// The Z3 session engine. Every mutation locks [`EngineInner`] briefly on
/// the calling thread — never across an await (the relays only call the
/// `finish_*`/`set_*` seams between awaits).
pub(crate) struct SessionEngine {
    inner: Mutex<EngineInner>,
    /// Serializes dispatch, queue claims, and decisions without holding the model lock over I/O.
    operations: Mutex<()>,
    accepted_sends: Mutex<BTreeMap<RequestId, serde_json::Value>>,
    accepted_decisions: Mutex<BTreeMap<ApprovalId, serde_json::Value>>,
    metadata_io: Mutex<()>,
    queue_allowed: AtomicBool,
    pub(crate) action_operations: Mutex<()>,
    pub(crate) action_results:
        Mutex<BTreeMap<RequestId, (serde_json::Value, skarn_protocol::CommandResultDto)>>,
    config: EngineConfig,
    tool_capability_cache: Arc<ToolCapabilityCache>,
    /// Phase 9e: the session-scoped background subagent runtime. It
    /// outlives individual turns, so a child started by one turn can finish
    /// while the user edits in another. The `wait` tool reads it through
    /// `ToolContext`.
    subagents: skarn_ai::SubagentRuntime,
    /// §19.3: the one relay receiver for the runtime's completion and card
    /// events. Taken by the first agent turn; the relay then lives for the
    /// engine's lifetime so a child may finish while the user edits.
    subagent_events: Mutex<Option<mpsc::Receiver<skarn_ai::SubagentEvent>>>,
    /// Child cards waiting on the parent's decision rail. The runtime owns the
    /// single-use decision slot; this map only validates request/session/card
    /// type before forwarding.
    subagent_cards: Mutex<BTreeMap<ApprovalId, SubagentCardRoute>>,
    /// Keys captured by child backends, including keys rotated before activity arrives.
    subagent_redaction_keys: Mutex<Vec<String>>,
    /// Tool records that arrived while the keyring was unavailable. Drained in
    /// arrival order by the next store access; never dropped.
    pending_transcript: Mutex<Vec<PendingTranscriptRecord>>,
    /// The agent tools' root. The Z3 shell has no folder picker (that lands
    /// in Z4 with the workbench), so `SELORA_WORKSPACE_ROOT` is the one way
    /// to point the tools at a real tree today; without it `SendAgent`
    /// answers the typed `no_workspace` error.
    workspace_root: Mutex<Option<PathBuf>>,
    /// Stage 3: live trust and privacy state shared with `DesktopState` & `WorkspaceStateReader`.
    trust: Arc<LiveTrustState>,
    /// Stage 3: desktop workspace reference for live buffer snapshots and post-mutation refreshes.
    workspace: Mutex<Option<Arc<Mutex<Option<Workspace>>>>>,
    refresh_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    sync_grants_hook: Mutex<Option<Arc<dyn Fn(&[ScopedGrantEntry]) + Send + Sync>>>,
    scoped_policy: Arc<Mutex<ScopedPolicyStore>>,
    app_data_dir: Mutex<PathBuf>,
    checkpoint_store: Mutex<Arc<TurnCheckpointStore>>,
    turn_checkpoints: Mutex<Vec<TurnCheckpointSummaryDto>>,
    active_cancel_tokens: Mutex<BTreeMap<RequestId, DecisionCancelToken>>,
    question_timeouts: Mutex<BTreeMap<ApprovalId, QuestionTimeoutDto>>,
    rules_state: Mutex<(Vec<RuleSourceDto>, String, Option<String>)>,
}

impl SessionEngine {
    pub(crate) fn new(config: EngineConfig) -> Self {
        let initial_key = config.key_source.get().ok().flatten();
        // Never fabricate models or report Connected before real discovery.
        let initial_connection = ConnectionStatus::Disconnected;
        let initial_models = Vec::new();
        let initial_active_model = config
            .active_model
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_owned);
        let trust = Arc::new(LiveTrustState::new(
            config.agent.workspace_trusted,
            config.agent.local_only,
        ));
        let scoped_store = ScopedPolicyStore::load_persisted(config.agent.scoped_grants.clone());
        let app_data_dir = skarn_settings::Settings::default_path()
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let checkpoint_store = Arc::new(TurnCheckpointStore::new(app_data_dir.join("checkpoints")));

        let runtime = skarn_ai::SubagentRuntime::new();
        let (tx, rx) = mpsc::channel(256);
        runtime.set_event_sink(tx);
        Self {
            operations: Mutex::new(()),
            accepted_sends: Mutex::new(BTreeMap::new()),
            accepted_decisions: Mutex::new(BTreeMap::new()),
            metadata_io: Mutex::new(()),
            queue_allowed: AtomicBool::new(true),
            action_operations: Mutex::new(()),
            action_results: Mutex::new(BTreeMap::new()),
            inner: Mutex::new(EngineInner {
                base_url: config.base_url.clone(),
                active_model: initial_active_model,
                sessions: BTreeMap::new(),
                active: BTreeMap::new(),
                models: initial_models,
                connection: initial_connection,
                last_connection_error: None,
                activity_counter: 0,
                active_session: None,
            }),
            config,
            tool_capability_cache: Arc::new(ToolCapabilityCache::default()),
            workspace_root: Mutex::new(None),
            trust,
            workspace: Mutex::new(None),
            refresh_hook: Mutex::new(None),
            sync_grants_hook: Mutex::new(None),
            scoped_policy: Arc::new(Mutex::new(scoped_store)),
            app_data_dir: Mutex::new(app_data_dir),
            checkpoint_store: Mutex::new(checkpoint_store),
            turn_checkpoints: Mutex::new(Vec::new()),
            active_cancel_tokens: Mutex::new(BTreeMap::new()),
            question_timeouts: Mutex::new(BTreeMap::new()),
            rules_state: Mutex::new((Vec::new(), String::new(), None)),
            subagents: runtime,
            subagent_events: Mutex::new(Some(rx)),
            subagent_cards: Mutex::new(BTreeMap::new()),
            subagent_redaction_keys: Mutex::new(initial_key.into_iter().collect()),
            pending_transcript: Mutex::new(Vec::new()),
        }
    }

    /// Share `LiveTrustState`, `Workspace`, `app_data_dir`, refresh hook, and
    /// grant persistence callback from `DesktopState`.
    pub(crate) fn configure_desktop(
        &self,
        trust: Arc<LiveTrustState>,
        workspace: Arc<Mutex<Option<Workspace>>>,
        app_data_dir: PathBuf,
        refresh_hook: Arc<dyn Fn() + Send + Sync>,
        sync_grants_hook: Arc<dyn Fn(&[ScopedGrantEntry]) + Send + Sync>,
    ) {
        // Keep the same `Arc<LiveTrustState>`? Replace internal state? Wait:
        // `SessionEngine` holds `trust: Arc<LiveTrustState>`. Wait — if `SessionEngine`
        // uses `trust` initialized in `new`, let's overwrite `self.trust`? No,
        // `trust` is not behind a Mutex unless we either pass `trust` into `EngineConfig`
        // OR expose `engine.trust()`. Look! Even cleaner: `DesktopState` shares
        // `let trust = engine.live_trust()` OR `DesktopState` initializes `trust` from
        // `sessions.live_trust()`! Let's keep both working cleanly: sync initial values
        // into `self.trust` AND expose `pub(crate) fn live_trust(&self) -> Arc<LiveTrustState>`!
        self.trust.set_trusted(trust.is_trusted());
        self.trust.set_global_local_only(trust.global_local_only());
        self.trust.set_workspace_local_only(trust.workspace_local_only());
        *self
            .workspace
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(workspace);
        *self
            .refresh_hook
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(refresh_hook);
        *self
            .sync_grants_hook
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(sync_grants_hook);
        *self
            .app_data_dir
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = app_data_dir.clone();
        *self
            .checkpoint_store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Arc::new(TurnCheckpointStore::new(app_data_dir.join("checkpoints")));
    }

    pub(crate) fn live_trust(&self) -> Arc<LiveTrustState> {
        Arc::clone(&self.trust)
    }

    pub(crate) fn set_app_data_dir(&self, dir: PathBuf) {
        *self
            .app_data_dir
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = dir.clone();
        *self
            .checkpoint_store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Arc::new(TurnCheckpointStore::new(dir.join("checkpoints")));
    }

    pub(crate) fn refresh_workspace_buffers(&self) {
        let hook = self
            .refresh_hook
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(hook) = hook {
            hook();
        }
    }

    pub(crate) fn revoke_live_trust(&self) {
        let tokens: Vec<DecisionCancelToken> = self
            .active_cancel_tokens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .cloned()
            .collect();
        for token in tokens {
            token.cancel();
        }
    }

    pub(crate) fn global_agents_md_path(&self) -> PathBuf {
        self.app_data_dir
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .join("AGENTS.md")
    }

    pub(crate) fn load_and_cache_rules(&self, root: &Path, trusted: bool) -> RulesLoadedDto {
        let global_path = self.global_agents_md_path();
        let loaded = RulesLoader::load_rules(Some(global_path.as_path()), root, trusted);
        let sources: Vec<RuleSourceDto> = loaded.sources.iter().map(rule_source_dto).collect();
        *self
            .rules_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = (
            sources.clone(),
            loaded.effective_instructions,
            loaded.rules_digest,
        );
        let session_id = self.active_session_id();
        RulesLoadedDto {
            session_id,
            workspace_trusted: trusted,
            sources,
        }
    }

    pub(crate) fn clear_rules_cache(&self) {
        *self
            .rules_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            (Vec::new(), String::new(), None);
    }

    pub(crate) fn rules_in_effect_dto(&self) -> Vec<RuleSourceDto> {
        self.rules_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .0
            .clone()
    }

    pub(crate) fn scoped_grants_dto(&self) -> Vec<ScopedGrantDto> {
        self.scoped_policy
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entries()
            .iter()
            .map(scoped_grant_dto)
            .collect()
    }

    pub(crate) fn turn_checkpoints_dto(&self) -> Vec<TurnCheckpointSummaryDto> {
        self.turn_checkpoints
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub(crate) fn sync_persisted_grants(&self) {
        let entries = self
            .scoped_policy
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entries()
            .to_vec();
        let hook = self
            .sync_grants_hook
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(hook) = hook {
            hook(&entries);
        }
    }

    pub(crate) fn revoke_scoped_grant(
        &self,
        action_class: ActionClassDto,
        target_pattern: &str,
    ) -> Vec<ScopedGrantDto> {
        {
            let mut store = self
                .scoped_policy
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            store.revoke(action_class_from_dto(action_class), target_pattern);
        }
        self.sync_persisted_grants();
        self.scoped_grants_dto()
    }

    pub(crate) fn record_turn_checkpoint(
        &self,
        session_id: &SessionId,
        manifest: &TurnCheckpointManifest,
    ) -> TurnCheckpointSummaryDto {
        let summary = checkpoint_summary_dto(session_id, manifest, false, Vec::new());
        let mut guard = self
            .turn_checkpoints
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.retain(|item| !(item.session_id == *session_id && item.turn_id == summary.turn_id));
        guard.push(summary.clone());
        summary
    }

    fn workspace_reader_for(&self, root: &Path) -> Arc<dyn WorkspaceStateReader> {
        let ws = self
            .workspace
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        match ws {
            Some(workspace) => Arc::new(DesktopWorkspaceReader::new(
                root.to_path_buf(),
                workspace,
                Arc::clone(&self.trust),
            )),
            None => Arc::new(skarn_decision::FilesystemWorkspaceReader::new(
                root,
                self.trust.is_trusted(),
                self.trust.effective_local_only(),
            )),
        }
    }

    pub(crate) fn undo_or_reapply_turn(
        &self,
        session_id: &SessionId,
        turn_id: &TurnId,
        undo: bool,
    ) -> Result<TurnCheckpointSummaryDto, (Option<TurnCheckpointSummaryDto>, ProtocolErrorDto)>
    {
        let root = self
            .workspace_root
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .ok_or_else(|| (None, no_workspace()))?;
        let ws_id = workspace_id_for(&root);
        let store = self
            .checkpoint_store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let manifest = store
            .load_manifest(ws_id.as_str(), session_id.as_str(), turn_id.as_str())
            .map_err(|err| {
                (
                    None,
                    ProtocolErrorDto {
                        code: "checkpoint_not_found".into(),
                        message: format!("Turn checkpoint not found: {err}"),
                        retryable: false,
                    },
                )
            })?;
        let reader = self.workspace_reader_for(&root);
        let op = if undo {
            store.undo_turn(
                &root,
                ws_id.as_str(),
                session_id.as_str(),
                turn_id.as_str(),
                Some(reader.as_ref()),
            )
        } else {
            store.reapply_turn(
                &root,
                ws_id.as_str(),
                session_id.as_str(),
                turn_id.as_str(),
                Some(reader.as_ref()),
            )
        };
        match op {
            Ok(()) => {
                self.refresh_workspace_buffers();
                let summary = checkpoint_summary_dto(session_id, &manifest, undo, Vec::new());
                let mut guard = self
                    .turn_checkpoints
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                guard.retain(|item| {
                    !(item.session_id == *session_id && item.turn_id == *turn_id)
                });
                guard.push(summary.clone());
                Ok(summary)
            }
            Err(CheckpointError::Conflict(conflicts)) => {
                let conflict_dtos: Vec<UndoConflictDto> = conflicts
                    .iter()
                    .map(|c| UndoConflictDto {
                        path: c.rel_path.clone(),
                        reason: c.reason.clone(),
                    })
                    .collect();
                let current_undone = self
                    .turn_checkpoints
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .iter()
                    .find(|item| item.session_id == *session_id && item.turn_id == *turn_id)
                    .is_some_and(|item| item.undone);
                let summary = checkpoint_summary_dto(
                    session_id,
                    &manifest,
                    current_undone,
                    conflict_dtos,
                );
                Err((
                    Some(summary),
                    ProtocolErrorDto {
                        code: "checkpoint_conflict".into(),
                        message: "Cannot apply turn checkpoint due to local file or buffer edits."
                            .into(),
                        retryable: false,
                    },
                ))
            }
            Err(err) => Err((
                None,
                ProtocolErrorDto {
                    code: "checkpoint_failed".into(),
                    message: format!("Turn checkpoint operation failed: {err}"),
                    retryable: false,
                },
            )),
        }
    }

    pub(crate) fn set_question_countdown_paused(
        &self,
        request_id: &RequestId,
        approval_id: &ApprovalId,
        paused: bool,
    ) -> Result<(), ProtocolErrorDto> {
        let loop_id = Uuid::parse_str(approval_id.as_str()).map_err(|_| ProtocolErrorDto {
            code: "approval_mismatch".into(),
            message: "The approval id does not name a pending approval.".into(),
            retryable: false,
        })?;
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let matches_pending = inner.sessions.values().any(|session| {
            session.pending_approval.as_ref().is_some_and(|pending| {
                &pending.request_id == request_id && &pending.approval_id == approval_id
            })
        });
        if !matches_pending {
            return Err(ProtocolErrorDto {
                code: "approval_not_pending".into(),
                message: "No matching question card is currently pending.".into(),
                retryable: false,
            });
        }
        let Some(ActiveRequest::Agent { decisions, .. }) = inner.active.get(request_id) else {
            return Err(ProtocolErrorDto {
                code: "approval_expired".into(),
                message: "The turn this question belongs to has ended.".into(),
                retryable: false,
            });
        };
        decisions
            .send((loop_id, skarn_ai::AgentDecision::SetQuestionCountdownPaused(paused)))
            .map_err(|_| ProtocolErrorDto {
                code: "approval_expired".into(),
                message: "The turn this question belongs to has ended.".into(),
                retryable: false,
            })
    }

    pub(crate) fn set_workspace_root(&self, root: Option<PathBuf>) {
        let mut current = self
            .workspace_root
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *current != root {
            let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            for active in inner.active.values() {
                match active {
                    ActiveRequest::Chat { cancel, .. } => {
                        let _ = cancel.send(true);
                    }
                    ActiveRequest::Agent { decisions, .. } => {
                        let _ = decisions.send((Uuid::nil(), skarn_ai::AgentDecision::Stop));
                        self.subagents.cancel_parent(active.session_id().as_str());
                    }
                }
            }
            inner.sessions.clear();
            inner.active.clear();
            inner.active_session = None;
            self.turn_checkpoints
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clear();
        }
        match root.as_ref() {
            Some(path) => {
                let trusted = self.trust.is_trusted();
                let _ = self.load_and_cache_rules(path, trusted);
            }
            None => self.clear_rules_cache(),
        }
        *current = root;
    }

    pub(crate) fn set_queue_allowed(&self, allowed: bool) {
        self.queue_allowed.store(allowed, Ordering::SeqCst);
    }

    // -----------------------------------------------------------------
    // Phase 9d — durable sessions
    // -----------------------------------------------------------------

    /// The store rooted at the live workspace. No workspace means no agent
    /// session can exist (the same rule as `SendAgent`).
    fn store(&self) -> Option<AgentSessionStore> {
        self.store_with_keys().ok().flatten()
    }

    /// [`Self::store`] that distinguishes a transient keyring failure
    /// (`Err`) from "no workspace" (`Ok(None)`). The keyring error must never
    /// be mistaken for a missing store: writing without the active key would
    /// leak it into the durable transcript.
    fn store_with_keys(&self) -> Result<Option<AgentSessionStore>, ()> {
        let root = self
            .workspace_root
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let Some(root) = root else {
            return Ok(None);
        };
        let current = self.config.key_source.get().map_err(|_| ())?;
        if let Some(key) = current {
            self.register_subagent_redaction_key(&key);
        }
        let keys = self
            .subagent_redaction_keys
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(Some(
            keys.iter()
                .fold(AgentSessionStore::new(root), |store, key| {
                    store.with_redaction_key(key)
                }),
        ))
    }

    /// Append one live transcript record. A transient keyring failure queues
    /// the record instead of dropping it; the next successful store access
    /// retries the whole queue in arrival order.
    fn append_transcript(
        &self,
        session_id: &SessionId,
        entry: TranscriptEntry,
    ) -> Result<(), TranscriptAppend> {
        let store = match self.store_with_keys() {
            Ok(Some(store)) => store,
            Ok(None) => return Err(TranscriptAppend::Failed),
            Err(()) => {
                self.pending_transcript
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(PendingTranscriptRecord {
                        session_id: session_id.clone(),
                        entry,
                    });
                return Err(TranscriptAppend::Queued);
            }
        };
        // Earlier records must land first: a queued call cannot follow its
        // own result on disk.
        self.drain_pending_transcript(&store)?;
        store
            .append(session_id.as_str(), &entry)
            .map_err(|_| TranscriptAppend::Failed)
    }

    /// Append every queued record, keeping the queue intact when an append
    /// fails so a later retry still delivers it.
    fn drain_pending_transcript(&self, store: &AgentSessionStore) -> Result<(), TranscriptAppend> {
        let mut pending = self
            .pending_transcript
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while let Some(record) = pending.first() {
            store
                .append(record.session_id.as_str(), &record.entry)
                .map_err(|_| TranscriptAppend::Failed)?;
            pending.remove(0);
        }
        Ok(())
    }

    /// Retry queued tool records. `Ok(())` when nothing is queued, even while
    /// the keyring is unavailable.
    fn persist_pending_transcript(&self) -> Result<(), ()> {
        if self
            .pending_transcript
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty()
        {
            return Ok(());
        }
        let store = self.store_with_keys().map_err(|_| ())?.ok_or(())?;
        self.drain_pending_transcript(&store).map_err(|_| ())
    }

    /// The resume picker's list (read-only; newest first).
    pub(crate) fn agent_sessions(&self) -> Vec<AgentSessionSummaryDto> {
        let Some(store) = self.store() else {
            return Vec::new();
        };
        store
            .list()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|summary| {
                Some(AgentSessionSummaryDto {
                    session_id: SessionId::new(summary.session_id).ok()?,
                    title: summary.title,
                    updated_at: summary.updated_at,
                    model: summary.model,
                    mode: summary.mode.as_str().to_owned(),
                    pending: summary.pending.map(|pending| AgentParkSummaryDto {
                        kind: pending.kind,
                        summary: pending.summary,
                    }),
                    compacted: summary.compacted,
                    message_count: summary.message_count,
                })
            })
            .collect()
    }

    /// Persist one session's new messages, metadata, and todos.
    ///
    /// Appends are incremental (one fsynced record per new message); the
    /// state/todos files are atomic replacements. A failed append leaves the
    /// durable counter unchanged so the next flush retries it.
    fn flush_session(&self, session_id: &SessionId) {
        let _metadata = self.metadata_io.lock().unwrap_or_else(|p| p.into_inner());
        let Some(store) = self.store() else {
            return;
        };
        // Queued tool records keep their arrival order relative to the message
        // tail; when they cannot land yet, leave the tail in memory too.
        if self.drain_pending_transcript(&store).is_err() {
            return;
        }
        let (start, entries, state, todos) = {
            let inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(session) = inner.sessions.get(session_id) else {
                return;
            };
            if !session.durable {
                return;
            }
            let start = session.durable_entries.min(session.messages.len());
            let entries: Vec<TranscriptEntry> = session.messages[start..]
                .iter()
                .map(message_entry)
                .collect();
            (
                start,
                entries,
                session_state(session_id, session),
                skarn_ai::plan::snapshot(&session.plan_store).todos,
            )
        };
        let mut appended = 0usize;
        for entry in &entries {
            if store.append(session_id.as_str(), entry).is_err() {
                break;
            }
            appended += 1;
        }
        let _ = store.save_state(&state);
        let _ = store.save_todos(session_id.as_str(), &todos);
        if appended > 0 {
            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(session) = inner.sessions.get_mut(session_id) {
                let end = (start + appended).min(session.messages.len());
                session.durable_entries = end;
            }
        }
    }

    /// Append a live tool-call batch to the transcript (not part of the
    /// in-memory message tail).
    fn record_tool_calls(
        &self,
        session_id: &SessionId,
        calls: Vec<skarn_ai::ToolCall>,
    ) -> Result<(), TranscriptAppend> {
        self.append_transcript(
            session_id,
            TranscriptEntry::ToolCalls {
                id: Uuid::new_v4().to_string(),
                timestamp: skarn_ai::now_millis(),
                calls,
            },
        )
    }

    /// Append one tool result to the transcript.
    fn record_tool_result(
        &self,
        session_id: &SessionId,
        result: &skarn_ai::ToolResult,
    ) -> Result<(), TranscriptAppend> {
        self.append_transcript(
            session_id,
            TranscriptEntry::Tool {
                id: Uuid::new_v4().to_string(),
                timestamp: skarn_ai::now_millis(),
                call_id: result.id.clone(),
                ok: result.ok,
                content: result.content.clone(),
            },
        )
    }

    /// Record a successful compaction cursor. The transcript is not rewritten.
    fn record_compaction(&self, session_id: &SessionId, cursor: skarn_ai::CompactionCursor) {
        {
            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(session) = inner.sessions.get_mut(session_id) {
                session.compaction = cursor;
                session.updated_at = skarn_ai::now_millis();
            }
        }
        self.flush_session(session_id);
    }

    /// Switch the dock to a session, restoring it from disk when it is not in
    /// memory. Every session's auto-approve flag is cleared first: trust never
    /// transfers across a switch.
    pub(crate) fn switch_session(&self, session_id: &SessionId) -> Result<(), ProtocolErrorDto> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !inner.sessions.contains_key(session_id) {
            let loaded = self.load_session_from_disk(session_id)?;
            inner.sessions.insert(session_id.clone(), loaded);
        }
        for session in inner.sessions.values_mut() {
            session.auto_approve = false;
        }
        inner.activity_counter += 1;
        let activity = inner.activity_counter;
        if let Some(session) = inner.sessions.get_mut(session_id) {
            session.last_activity = activity;
        }
        inner.active_session = Some(session_id.clone());
        Ok(())
    }

    /// Create a fresh session and make it active. Its auto-approve flag is
    /// off by construction.
    pub(crate) fn new_session(&self) -> Result<SessionId, ProtocolErrorDto> {
        let store = self.store().ok_or_else(no_workspace)?;
        let session_id =
            SessionId::new(format!("sess-{}", Uuid::new_v4())).expect("uuid session id is valid");
        let title = "New Agent Session".to_owned();
        let now = skarn_ai::now_millis();
        let state = store
            .create(
                session_id.as_str(),
                &title,
                self.active_model_id().as_deref(),
                skarn_ai::AgentSessionMode::Execute,
            )
            .map_err(|error| session_persistence_error(&error))?;
        let mut session = ChatSession::new(title, now);
        session.mode = AgentModeDto::AskBeforeChanges;
        session.model = state.model;
        session.durable = true;
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for session in inner.sessions.values_mut() {
            session.auto_approve = false;
        }
        inner.activity_counter += 1;
        session.last_activity = inner.activity_counter;
        inner.sessions.insert(session_id.clone(), session);
        inner.active_session = Some(session_id.clone());
        Ok(session_id)
    }

    /// Delete a session. Durable agent sessions also lose their on-disk store,
    /// so a deleted tab cannot be resumed later. A running turn refuses the
    /// delete (the user must stop it first).
    ///
    /// The active session changes only when the deleted session *was* active:
    /// deleting a background tab leaves the user's selection alone. The
    /// replacement is a real session the engine still owns (the most recently
    /// touched one), never a synthesized placeholder.
    pub(crate) fn delete_session(&self, session_id: &SessionId) -> Result<(), ProtocolErrorDto> {
        let store = self.store();
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let open = inner.sessions.get(session_id);
        if let Some(session) = open {
            if session.active_request.is_some()
                || inner
                    .active
                    .values()
                    .any(|request| request.session_id() == session_id)
            {
                return Err(session_busy());
            }
        }
        let known = open.is_some();
        let durable = open.is_none_or(|session| session.durable);
        match store.as_ref() {
            Some(store) => {
                // A disk-only durable session is known through its store; a
                // session that exists in neither place is a stale tab.
                if !known && !store.exists(session_id.as_str()) {
                    return Err(unknown_session(session_id));
                }
                if durable {
                    store
                        .delete(session_id.as_str())
                        .map_err(|error| session_persistence_error(&error))?;
                }
            }
            None if !known => return Err(unknown_session(session_id)),
            None => {}
        }
        // A queued retry must not resurrect a deleted session's directory.
        self.pending_transcript
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|record| record.session_id != *session_id);
        inner.sessions.remove(session_id);
        if inner.active_session.as_ref() == Some(session_id) {
            inner.active_session = inner
                .sessions
                .iter()
                .max_by_key(|(_, candidate)| candidate.last_activity)
                .map(|(id, _)| id.clone());
        }
        // Normalize any dangling pointer (a session deleted without an active
        // marker must not keep leaking the old id into the bootstrap).
        if inner
            .active_session
            .as_ref()
            .is_some_and(|id| !inner.sessions.contains_key(id))
        {
            inner.active_session = None;
        }
        Ok(())
    }

    /// Arm/disarm session full access. The flag is session memory only.
    pub(crate) fn set_auto_approve(
        &self,
        session_id: &SessionId,
        enabled: bool,
    ) -> Result<(), ProtocolErrorDto> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(session) = inner.sessions.get_mut(session_id) else {
            return Err(ProtocolErrorDto {
                code: "session_not_found".into(),
                message: "That agent session is not open.".into(),
                retryable: false,
            });
        };
        session.auto_approve = enabled;
        Ok(())
    }

    /// Mirror the live plan/todo store into the durable session record after
    /// a plan or todo event. Phase 9b's events are the only writer of these
    /// fields, so this is where a reload's plan spine is captured.
    fn sync_plan_store(&self, session_id: &SessionId) {
        {
            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(session) = inner.sessions.get_mut(session_id) else {
                return;
            };
            session.updated_at = skarn_ai::now_millis();
        }
        self.flush_session(session_id);
    }

    /// Restore one session from disk into the in-memory shape.
    fn load_session_from_disk(
        &self,
        session_id: &SessionId,
    ) -> Result<ChatSession, ProtocolErrorDto> {
        let store = self.store().ok_or_else(no_workspace)?;
        let loaded = store
            .load(session_id.as_str())
            .map_err(|error| session_persistence_error(&error))?
            .ok_or_else(|| ProtocolErrorDto {
                code: "session_not_found".into(),
                message: "That agent session does not exist on disk.".into(),
                retryable: false,
            })?;
        let now = skarn_ai::now_millis();
        let mut session = ChatSession::new(loaded.state.title.clone(), now);
        session.created_at = loaded.state.created_at;
        session.updated_at = loaded.state.updated_at;
        session.durable = true;
        session.model = loaded.state.model.clone();
        session.mode = mode_from_disk(loaded.state.mode);
        // Phase 9b's live store hydrates from the durable record: the
        // resume picker must reopen the same plan phase, draft and todos.
        {
            let mut live = skarn_ai::plan::lock(&session.plan_store);
            live.phase = if loaded.state.plan.status == skarn_ai::PlanStatus::Proposed {
                skarn_ai::PlanPhase::Planning
            } else {
                skarn_ai::PlanPhase::Executing
            };
            live.plan = loaded
                .state
                .plan
                .plan
                .as_deref()
                .and_then(plan_draft_from_disk);
            live.todos = loaded.todos.clone();
        }
        session.compaction = loaded.state.compaction;
        session.messages = stored_transcript(&loaded.transcript.entries);
        session.durable_entries = session.messages.len();
        // A restored park is shown once. It has no live request, so any
        // decision expires instead of executing (see `agent_decision`).
        if let Some(park) = loaded.state.parks.first() {
            match park {
                ParkedCard::Approval {
                    approval_id,
                    tool_name,
                    card,
                    request_id,
                    turn_id,
                    ..
                } => {
                    if let Ok(approval_id) = ApprovalId::new(approval_id.clone()) {
                        let request_id = request_id
                            .as_deref()
                            .and_then(|id| RequestId::new(id).ok())
                            .unwrap_or_else(|| {
                                RequestId::new(format!("restored-{}", approval_id.as_str()))
                                    .expect("restored request id")
                            });
                        let turn_id = turn_id
                            .as_deref()
                            .and_then(|id| TurnId::new(id).ok())
                            .unwrap_or_else(|| {
                                TurnId::new(format!("restored-{}", approval_id.as_str()))
                                    .expect("restored turn id")
                            });
                        session.pending_approval = Some(ApprovalDto {
                            approval_id: approval_id.clone(),
                            request_id,
                            turn_id,
                            session_id: session_id.clone(),
                            summary: park.summary(),
                            can_approve_all_this_turn: false,
                            plan: approval_plan(card),
                            decision_prompt: approval_prompt(card),
                        });
                        session.pending_card = Some(card.clone());
                        session.pending_tool_name = Some(tool_name.clone());
                        session.restored_park = true;
                    }
                }
                ParkedCard::Question { .. } => {}
            }
        }
        Ok(session)
    }

    pub(crate) fn base_url(&self) -> String {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .base_url
            .clone()
    }

    /// The live agent request/turn for a session, when one is running. Child
    /// cards group under it; a background child may outlive the turn, in
    /// which case the relay mints its own ids.
    fn active_request_for_session(&self, session_id: &SessionId) -> Option<(RequestId, TurnId)> {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner
            .active
            .iter()
            .find_map(|(request_id, active)| match active {
                ActiveRequest::Agent {
                    session_id: owner,
                    turn_id,
                    ..
                } if owner == session_id => Some((request_id.clone(), turn_id.clone())),
                _ => None,
            })
    }

    pub(crate) fn set_base_url(&self, base_url: String) {
        self.tool_capability_cache.clear_base_url(&base_url);
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.tool_capability_cache.clear_base_url(&inner.base_url);
        inner.base_url = base_url;
    }

    pub(crate) fn key_source(&self) -> &ApiKeySource {
        &self.config.key_source
    }

    pub(crate) fn tool_capability_cache(&self) -> Arc<ToolCapabilityCache> {
        Arc::clone(&self.tool_capability_cache)
    }

    pub(crate) fn catalog_models(&self) -> Vec<Model> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .models
            .clone()
    }

    pub(crate) fn last_connection_error(&self) -> Option<String> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .last_connection_error
            .clone()
    }

    pub(crate) fn set_last_connection_error(&self, error: Option<String>) {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .last_connection_error = error;
    }

    pub(crate) fn clear_capability_cache_for(&self, base_url: &str) {
        self.tool_capability_cache.clear_base_url(base_url);
    }

    pub(crate) fn update_catalog_tool_capability(
        &self,
        model_id: &str,
        capability: ToolCapability,
    ) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for model in &mut inner.models {
            if model.id == model_id || model.id.eq_ignore_ascii_case(model_id) {
                model.tool_capability = capability;
            }
        }
    }

    pub(crate) fn set_models_and_connection(
        &self,
        models: Vec<Model>,
        connection: ConnectionStatus,
        active_model: Option<String>,
    ) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.models = models;
        inner.connection = connection;
        if connection == ConnectionStatus::Connected {
            inner.last_connection_error = None;
        }
        inner.active_model = active_model;
    }

    // -----------------------------------------------------------------
    // Snapshot accessors (the bootstrap)
    // -----------------------------------------------------------------

    pub(crate) fn connection(&self) -> ConnectionStatus {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .connection
    }

    pub(crate) fn models_dto(&self) -> Vec<ModelDto> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .models
            .iter()
            .map(model_dto)
            .collect()
    }

    pub(crate) fn active_model_id(&self) -> Option<String> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .active_model
            .clone()
    }

    /// Replace the process-wide fallback model used by plain chat, workflows,
    /// and any request that does not carry an explicit model id.
    ///
    /// The desktop shell owns this value so every request path observes the
    /// same model. `None` clears the fallback; an explicit per-request model
    /// still wins in the normal resolver.
    pub(crate) fn set_active_model_id(&self, model: Option<String>) {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .active_model = model;
    }

    /// The bootstrap's `chat` block: every session, the flattened
    /// transcript, the live request/turn ids, the newest parked approval,
    /// and the active session's authoritative todos.
    pub(crate) fn chat_state(&self) -> ChatStateDto {
        let key_available = self
            .config
            .key_source
            .get()
            .map(|key| {
                if let Some(key) = key {
                    self.register_subagent_redaction_key(&key);
                }
            })
            .is_ok();
        let keys = self
            .subagent_redaction_keys
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut sessions = Vec::new();
        let mut messages = Vec::new();
        let mut active_session_id = inner
            .active_session
            .clone()
            .filter(|id| inner.sessions.contains_key(id));
        let mut active_request_id = None;
        let mut active_turn_id = None;
        let mut pending_approval = None;
        let mut pending_approvals: Vec<(SessionId, ApprovalDto)> = Vec::new();
        // Recency order (most recent last): `active_session_id` picks the
        // most recently touched session that is still running a request.
        let mut ordered: Vec<(&SessionId, &ChatSession)> = inner.sessions.iter().collect();
        ordered.sort_by_key(|(_, session)| session.last_activity);
        for (session_id, session) in ordered {
            let plan = skarn_ai::plan::snapshot(&session.plan_store);
            sessions.push(SessionDto {
                session_id: session_id.clone(),
                title: if key_available {
                    redact_with_keys(&session.title, &keys)
                } else {
                    "[REDACTED]".into()
                },
                message_ids: session
                    .messages
                    .iter()
                    .map(|message| message.message_id.clone())
                    .collect(),
                active: session.active_request.is_some(),
                plan_phase: plan_phase_dto(plan.phase),
                plan: plan.plan.as_ref().map(|draft| {
                    if key_available {
                        scrub_serializable(plan_approval_dto(draft, ""), &keys)
                    } else {
                        PlanApprovalDto {
                            summary: None,
                            steps: Vec::new(),
                        }
                    }
                }),
                todos: if key_available {
                    plan.todos
                        .iter()
                        .map(|todo| scrub_serializable(todo_dto(todo), &keys))
                        .collect()
                } else {
                    Vec::new()
                },
            });
            for message in &session.messages {
                messages.push(MessageDto {
                    message_id: message.message_id.clone(),
                    session_id: session_id.clone(),
                    role: message.role,
                    text: if key_available {
                        redact_with_keys(&message.text, &keys)
                    } else {
                        "[REDACTED]".into()
                    },
                });
            }
            if let Some(request_id) = &session.active_request {
                if active_session_id.is_none() {
                    active_session_id = Some(session_id.clone());
                }
                if let Some(active) = inner
                    .active
                    .get(request_id)
                    .filter(|_| active_session_id.as_ref() == Some(session_id))
                {
                    active_request_id = Some(request_id.clone());
                    active_turn_id = Some(active.turn_id().clone());
                }
            }
            if let Some(approval) = &session.pending_approval {
                if key_available {
                    pending_approvals.push((
                        session_id.clone(),
                        scrub_serializable(approval.clone(), &keys),
                    ));
                }
            }
        }
        // Prefer the active session's park; otherwise the first in recency
        // order (the pre-9d behaviour).
        if let Some(active) = &active_session_id {
            pending_approval = pending_approvals
                .iter()
                .find(|(session, _)| session == active)
                .map(|(_, approval)| approval.clone());
        }
        ChatStateDto {
            sessions,
            messages,
            active_session_id: active_session_id.clone(),
            active_request_id,
            active_turn_id,
            pending_approval,
            pending_approvals: pending_approvals
                .into_iter()
                .map(|(_, approval)| approval)
                .collect(),
            todos: if !key_available {
                Vec::new()
            } else {
                active_session_id
                    .as_ref()
                    .and_then(|id| inner.sessions.get(id))
                    .map(|session| {
                        skarn_ai::plan::snapshot(&session.plan_store)
                            .todos
                            .iter()
                            .map(|todo| scrub_serializable(todo_dto(todo), &keys))
                            .collect()
                    })
                    .unwrap_or_default()
            },
        }
    }

    /// The session the engine currently considers active (new/switch/send).
    pub(crate) fn active_session_id(&self) -> Option<SessionId> {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.active_session.clone().or_else(|| {
            inner
                .sessions
                .iter()
                .max_by_key(|(_, session)| session.last_activity)
                .map(|(id, _)| id.clone())
        })
    }

    pub(crate) fn live_sessions(&self) -> Vec<AgentSessionLiveDto> {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner
            .sessions
            .iter()
            .map(|(session_id, session)| {
                let plan = skarn_ai::plan::snapshot(&session.plan_store);
                let active = session
                    .active_request
                    .as_ref()
                    .and_then(|id| inner.active.get(id));
                let approvals = session
                    .pending_approval
                    .as_ref()
                    .map(|approval| {
                        let (path_or_command, diff_or_input, workflow) = session
                            .pending_card
                            .as_ref()
                            .map(approval_body)
                            .unwrap_or_else(|| (approval.summary.clone(), None, None));
                        let (action_class, risk) = session
                            .pending_card
                            .as_ref()
                            .map(|c| pending_card_classification(c, self.trust.is_trusted()))
                            .map(|(c, r)| (Some(c), Some(r)))
                            .unwrap_or((None, None));
                        let question_timeout = self
                            .question_timeouts
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .get(&approval.approval_id)
                            .cloned();
                        ApprovalNeededDto {
                            request_id: approval.request_id.clone(),
                            session_id: session_id.clone(),
                            turn_id: approval.turn_id.clone(),
                            approval_id: approval.approval_id.clone(),
                            tool_name: session.pending_tool_name.clone().unwrap_or_default(),
                            path_or_command,
                            diff_or_input,
                            workflow,
                            plan: approval.plan.clone(),
                            prompt: approval.decision_prompt.clone(),
                            subagent_id: None,
                            risk,
                            action_class,
                            question_timeout,
                        }
                    })
                    .into_iter()
                    .collect();
                AgentSessionLiveDto {
                    session_id: session_id.clone(),
                    title: session.title.clone(),
                    created_at: (session.created_at != 0).then_some(session.created_at),
                    updated_at: (session.updated_at != 0).then_some(session.updated_at),
                    model_id: session.model.clone(),
                    mode: session.mode,
                    durable: session.durable,
                    active_request_id: session.active_request.clone(),
                    active_turn_id: active.map(|request| request.turn_id().clone()),
                    state: None,
                    messages: session
                        .messages
                        .iter()
                        .map(|message| MessageDto {
                            message_id: message.message_id.clone(),
                            session_id: session_id.clone(),
                            role: message.role,
                            text: message.text.clone(),
                        })
                        .collect(),
                    timeline: Vec::new(),
                    approvals,
                    plan_phase: plan_phase_dto(plan.phase),
                    plan: plan.plan.as_ref().map(|draft| plan_approval_dto(draft, "")),
                    todos: plan.todos.iter().map(todo_dto).collect(),
                    queue: session.queue.clone(),
                    subagents: Vec::new(),
                }
            })
            .collect()
    }

    pub(crate) fn redact_live_sessions(&self, sessions: &mut Vec<AgentSessionLiveDto>) {
        if self.config.key_source.get().is_err() {
            for session in sessions {
                session.title = "[REDACTED]".into();
                session.messages.clear();
                session.timeline.clear();
                session.approvals.clear();
                session.plan = None;
                session.todos.clear();
                session.queue.clear();
                session.subagents.clear();
            }
            return;
        }
        let keys = self
            .subagent_redaction_keys
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        *sessions = scrub_serializable(sessions.clone(), &keys);
    }

    pub(crate) fn rename_session(
        &self,
        session_id: &SessionId,
        title: String,
    ) -> Result<AgentSessionRenamedDto, ProtocolErrorDto> {
        let _operation = self.operations.lock().unwrap_or_else(|p| p.into_inner());
        let _metadata = self.metadata_io.lock().unwrap_or_else(|p| p.into_inner());
        let title = title.trim();
        if title.is_empty() || title.chars().count() > 200 || title.chars().any(char::is_control) {
            return Err(ProtocolErrorDto {
                code: "invalid_session_title".into(),
                message:
                    "A session title must contain 1 to 200 characters without control characters."
                        .into(),
                retryable: false,
            });
        }
        let store = self.store_with_keys().map_err(|_| ProtocolErrorDto {
            code: "keyring_unavailable".into(),
            message: "Credential store unavailable.".into(),
            retryable: true,
        })?;
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let now = skarn_ai::now_millis();
        let durable = inner
            .sessions
            .get(session_id)
            .is_none_or(|session| session.durable);
        if durable {
            let store = store.ok_or_else(no_workspace)?;
            let mut state = if let Some(session) = inner.sessions.get(session_id) {
                session_state(session_id, session)
            } else {
                store
                    .load_state(session_id.as_str())
                    .map_err(|e| session_persistence_error(&e))?
                    .ok_or_else(|| unknown_session(session_id))?
            };
            state.title = title.to_owned();
            state.updated_at = now;
            store
                .save_state(&state)
                .map_err(|e| session_persistence_error(&e))?;
        } else if !inner.sessions.contains_key(session_id) {
            return Err(unknown_session(session_id));
        }
        if let Some(session) = inner.sessions.get_mut(session_id) {
            session.title = title.to_owned();
            session.updated_at = now;
        }
        let keys = self
            .subagent_redaction_keys
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        Ok(AgentSessionRenamedDto {
            session_id: session_id.clone(),
            title: redact_with_keys(title, &keys),
            updated_at: now,
        })
    }

    // -----------------------------------------------------------------
    // NewSession / SwitchSession
    // -----------------------------------------------------------------

    // -----------------------------------------------------------------
    // RetryMessage
    // -----------------------------------------------------------------

    /// Remove the failed tail (the named user message and everything after
    /// it) and re-dispatch that message's exact content once. The named
    /// message must be an existing user turn with a recorded dispatch path;
    /// a live turn or an unknown/non-user message is refused without
    /// touching the transcript.
    pub(crate) fn retry_message<R: tauri::Runtime>(
        self: &Arc<Self>,
        log: Arc<EventLog>,
        app: tauri::AppHandle<R>,
        envelope_session: Option<SessionId>,
        session_id: SessionId,
        message_id: MessageId,
    ) -> Result<RequestId, ProtocolErrorDto> {
        if envelope_session
            .as_ref()
            .is_some_and(|session| session != &session_id)
        {
            return Err(stale_session());
        }
        if self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .sessions
            .get(&session_id)
            .is_none_or(|session| session.durable)
        {
            let request_id = RequestId::new(Uuid::new_v4().to_string()).expect("uuid is valid");
            self.retry_durable_message(log, app, request_id.clone(), session_id, message_id)?;
            return Ok(request_id);
        }
        let (spec, text) = {
            let inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let session = inner
                .sessions
                .get(&session_id)
                .ok_or_else(|| unknown_session(&session_id))?;
            if session.active_request.is_some() {
                return Err(session_busy());
            }
            let message = session
                .messages
                .iter()
                .find(|message| message.message_id == message_id)
                .ok_or_else(|| unknown_message(&message_id))?;
            if message.role != MessageRole::User {
                return Err(not_a_user_message(&message_id));
            }
            let spec = message.retry.clone().ok_or_else(|| ProtocolErrorDto {
                code: "retry_unavailable".into(),
                message: "That message has no recorded dispatch to retry.".into(),
                retryable: false,
            })?;
            (spec, message.text.clone())
        };
        if envelope_session.is_some_and(|session| session != session_id) {
            return Err(stale_session());
        }

        let request_id = RequestId::new(Uuid::new_v4().to_string()).expect("uuid is valid");
        let target = RetryTarget {
            session_id: session_id.clone(),
            message_id,
        };
        match spec {
            RetrySpec::Chat { model_id } => self.send_chat_inner(
                log,
                app,
                Some(session_id.clone()),
                request_id.clone(),
                ChatRequestDto {
                    session_id,
                    message: text,
                    model_id,
                },
                Some(target),
            )?,
            RetrySpec::Agent {
                model_id,
                mode,
                reasoning_effort,
                system_instructions,
                temperature,
            } => self.send_agent_inner(
                log,
                app,
                Some(session_id.clone()),
                request_id.clone(),
                skarn_protocol::AgentRequestDto {
                    session_id,
                    prompt: text,
                    model_id,
                    mode,
                    reasoning_effort,
                    system_instructions,
                    temperature,
                    editor_context: None,
                    attachments: Vec::new(),
                },
                Some(target),
            )?,
        }
        Ok(request_id)
    }

    // -----------------------------------------------------------------
    // SendChat
    // -----------------------------------------------------------------

    pub(crate) fn send_chat<R: tauri::Runtime>(
        self: &Arc<Self>,
        log: Arc<EventLog>,
        app: tauri::AppHandle<R>,
        envelope_session: Option<SessionId>,
        request_id: RequestId,
        payload: ChatRequestDto,
    ) -> Result<(), ProtocolErrorDto> {
        let _operation = self.operations.lock().unwrap_or_else(|p| p.into_inner());
        if envelope_session
            .as_ref()
            .is_some_and(|id| id != &payload.session_id)
        {
            return Err(stale_session());
        }
        let identity = serde_json::json!({ "kind": "chat", "request": &payload });
        if self.send_duplicate(&request_id, &identity)? {
            return Ok(());
        }
        self.send_chat_inner(
            log,
            app,
            envelope_session,
            request_id.clone(),
            payload,
            None,
        )?;
        self.accepted_sends
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(request_id, identity);
        Ok(())
    }

    fn send_chat_inner<R: tauri::Runtime>(
        self: &Arc<Self>,
        log: Arc<EventLog>,
        app: tauri::AppHandle<R>,
        envelope_session: Option<SessionId>,
        request_id: RequestId,
        payload: ChatRequestDto,
        retry: Option<RetryTarget>,
    ) -> Result<(), ProtocolErrorDto> {
        if envelope_session.is_some_and(|session| session != payload.session_id) {
            return Err(stale_session());
        }
        let api_key = self.fetch_api_key()?;
        let model = self.resolve_model_for_dispatch(payload.model_id.as_deref())?;
        let model_id = model.id;

        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(target) = &retry {
            truncate_for_retry(&mut inner, target)?;
        }
        let spec = RetrySpec::Chat {
            model_id: payload.model_id.clone(),
        };
        let session = begin_turn(
            &mut inner,
            &request_id,
            &payload.session_id,
            &payload.message,
            Some(spec),
        )?;
        session.model = Some(model_id.clone());
        // One turn, one streaming assistant message. The conversation is
        // the transcript's tail — `begin_turn` already landed the user's
        // message on it.
        let turn_id = fresh_turn_id();
        let message_id = fresh_message_id();
        let conversation: Vec<ChatMessage> = conversation_tail(session);
        let request = ChatRequest::new(model_id, conversation);
        let (cancel_tx, cancel_rx) = watch::channel(false);
        inner.active.insert(
            request_id.clone(),
            ActiveRequest::Chat {
                session_id: payload.session_id.clone(),
                turn_id: turn_id.clone(),
                cancel: cancel_tx,
            },
        );
        drop(inner);

        tauri::async_runtime::spawn(chat_relay(ChatRelayCtx {
            log,
            engine: Arc::clone(self),
            app,
            request_id,
            session_id: payload.session_id,
            turn_id,
            message_id,
            base_url: self.base_url(),
            api_key,
            request,
            cancel_rx,
        }));
        Ok(())
    }

    // -----------------------------------------------------------------
    // SendAgent
    // -----------------------------------------------------------------

    pub(crate) fn send_agent<R: tauri::Runtime>(
        self: &Arc<Self>,
        log: Arc<EventLog>,
        app: tauri::AppHandle<R>,
        envelope_session: Option<SessionId>,
        request_id: RequestId,
        payload: skarn_protocol::AgentRequestDto,
    ) -> Result<(), ProtocolErrorDto> {
        let _operation = self.operations.lock().unwrap_or_else(|p| p.into_inner());
        if envelope_session
            .as_ref()
            .is_some_and(|id| id != &payload.session_id)
        {
            return Err(stale_session());
        }
        {
            let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            if inner
                .sessions
                .get(&payload.session_id)
                .is_some_and(|session| !session.queue.is_empty())
            {
                return Err(session_busy());
            }
        }
        let identity = serde_json::json!({ "kind": "agent", "request": &payload });
        if self.send_duplicate(&request_id, &identity)? {
            return Ok(());
        }
        self.send_agent_inner(
            log,
            app,
            envelope_session,
            request_id.clone(),
            payload,
            None,
        )?;
        self.accepted_sends
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(request_id, identity);
        Ok(())
    }

    fn send_duplicate(
        &self,
        request_id: &RequestId,
        identity: &serde_json::Value,
    ) -> Result<bool, ProtocolErrorDto> {
        let accepted = self
            .accepted_sends
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        match accepted.get(request_id) {
            Some(previous) if previous == identity => Ok(true),
            Some(_) => Err(idempotency_conflict()),
            None => Ok(false),
        }
    }

    pub(crate) fn queue_agent<R: tauri::Runtime>(
        self: &Arc<Self>,
        log: Arc<EventLog>,
        app: tauri::AppHandle<R>,
        envelope_session: Option<SessionId>,
        request_id: RequestId,
        request: skarn_protocol::AgentRequestDto,
    ) -> Result<(), ProtocolErrorDto> {
        {
            let _operation = self.operations.lock().unwrap_or_else(|p| p.into_inner());
            if envelope_session
                .as_ref()
                .is_some_and(|id| id != &request.session_id)
            {
                return Err(stale_session());
            }
            if request.prompt.trim().is_empty() {
                return Err(invalid_queue("A queued prompt cannot be empty."));
            }
            let identity = serde_json::json!({ "kind": "agent", "request": &request });
            if self.send_duplicate(&request_id, &identity)? {
                return Ok(());
            }
            let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            let session = inner
                .sessions
                .get_mut(&request.session_id)
                .ok_or_else(|| unknown_session(&request.session_id))?;
            session.queue.push(AgentQueueItemDto {
                request_id: request_id.clone(),
                request: request.clone(),
                created_at: skarn_ai::now_millis(),
                status: AgentQueueStatusDto::Queued,
                error: None,
            });
            self.accepted_sends
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(request_id, identity);
        }
        self.publish_queue(&log, &app, &request.session_id);
        self.kick_queue(log, app, &request.session_id);
        Ok(())
    }

    pub(crate) fn mutate_queue<R: tauri::Runtime>(
        &self,
        log: &EventLog,
        app: &tauri::AppHandle<R>,
        session_id: &SessionId,
        mutation: QueueMutation,
    ) -> Result<(), ProtocolErrorDto> {
        let _operation = self.operations.lock().unwrap_or_else(|p| p.into_inner());
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let session = inner
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| unknown_session(session_id))?;
        match mutation {
            QueueMutation::Update(id, request) => {
                let request = *request;
                if &request.session_id != session_id || request.prompt.trim().is_empty() {
                    return Err(invalid_queue(
                        "The queued prompt must target its existing session and contain text.",
                    ));
                }
                let item = session
                    .queue
                    .iter_mut()
                    .find(|item| item.request_id == id)
                    .ok_or_else(|| invalid_queue("That queued request does not exist."))?;
                item.request = request;
                item.status = AgentQueueStatusDto::Queued;
                item.error = None;
                self.accepted_sends
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(
                        id,
                        serde_json::json!({ "kind": "agent", "request": &item.request }),
                    );
            }
            QueueMutation::Remove(id) => {
                if !session.queue.iter().any(|item| item.request_id == id) {
                    return Err(invalid_queue("That queued request does not exist."));
                }
                session.queue.retain(|item| item.request_id != id);
            }
            QueueMutation::Reorder(ids) => {
                let unique: std::collections::BTreeSet<_> = ids.iter().collect();
                if ids.len() != session.queue.len()
                    || unique.len() != ids.len()
                    || ids
                        .iter()
                        .any(|id| !session.queue.iter().any(|item| &item.request_id == id))
                {
                    return Err(invalid_queue(
                        "Queue order must be an exact permutation of queued request IDs.",
                    ));
                }
                session.queue.sort_by_key(|item| {
                    ids.iter()
                        .position(|id| id == &item.request_id)
                        .expect("validated queue ID")
                });
            }
            QueueMutation::Clear => session.queue.clear(),
            QueueMutation::Resume => {
                session.queue_paused = false;
                if let Some(item) = session.queue.first_mut() {
                    item.status = AgentQueueStatusDto::Queued;
                    item.error = None;
                }
            }
        }
        drop(inner);
        self.publish_queue(log, app, session_id);
        Ok(())
    }

    fn publish_queue<R: tauri::Runtime>(
        &self,
        log: &EventLog,
        app: &tauri::AppHandle<R>,
        session_id: &SessionId,
    ) {
        let queue = self
            .inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .sessions
            .get(session_id)
            .map(|session| session.queue.clone());
        if let Some(queue) = queue {
            let keys = self
                .subagent_redaction_keys
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            log.record_session(
                app,
                FrontendEvent::AgentQueueChanged(scrub_serializable(
                    AgentQueueChangedDto {
                        session_id: session_id.clone(),
                        queue,
                    },
                    &keys,
                )),
                None,
                session_id.clone(),
            );
        }
    }

    /// Claims and starts at most one head. Terminal relays are the only automatic drain trigger.
    pub(crate) fn kick_queue<R: tauri::Runtime>(
        self: &Arc<Self>,
        log: Arc<EventLog>,
        app: tauri::AppHandle<R>,
        session_id: &SessionId,
    ) {
        let _operation = self.operations.lock().unwrap_or_else(|p| p.into_inner());
        let item = {
            let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            let Some(session) = inner.sessions.get_mut(session_id) else {
                return;
            };
            if session.active_request.is_some() || session.queue_paused {
                return;
            }
            let Some(item) = session.queue.first_mut() else {
                return;
            };
            if item.status != AgentQueueStatusDto::Queued {
                return;
            }
            item.status = AgentQueueStatusDto::Starting;
            item.clone()
        };
        self.publish_queue(&log, &app, session_id);
        // Recheck trust at execution, not just enqueue time. Credentials/model are checked by send.
        let result = if !self.queue_allowed.load(Ordering::SeqCst) {
            Err(ProtocolErrorDto {
                code: "workspace_trust_restart_required".into(),
                message:
                    "Workspace trust changed. Restart Selora before resuming queued agent work."
                        .into(),
                retryable: false,
            })
        } else {
            self.send_agent_inner(
                Arc::clone(&log),
                app.clone(),
                Some(session_id.clone()),
                item.request_id.clone(),
                item.request,
                None,
            )
        };
        {
            let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(session) = inner.sessions.get_mut(session_id) {
                match result {
                    Ok(()) => session
                        .queue
                        .retain(|queued| queued.request_id != item.request_id),
                    Err(error) => {
                        session.queue_paused = true;
                        if let Some(head) = session.queue.first_mut() {
                            head.status = AgentQueueStatusDto::Failed;
                            head.error = Some(error);
                        }
                    }
                }
            }
        }
        self.publish_queue(&log, &app, session_id);
    }

    fn send_agent_inner<R: tauri::Runtime>(
        self: &Arc<Self>,
        log: Arc<EventLog>,
        app: tauri::AppHandle<R>,
        envelope_session: Option<SessionId>,
        request_id: RequestId,
        payload: skarn_protocol::AgentRequestDto,
        retry: Option<RetryTarget>,
    ) -> Result<(), ProtocolErrorDto> {
        if envelope_session.is_some_and(|session| session != payload.session_id) {
            return Err(stale_session());
        }
        let api_key = self.fetch_api_key()?;
        let mut model = self.resolve_model_for_dispatch(payload.model_id.as_deref())?;
        let root = self
            .workspace_root
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .ok_or_else(no_workspace)?;
        let base_url = self.base_url();
        let catalog_snapshot = self.catalog_models();
        let previous_cap = model.tool_capability;
        let check_result = ensure_tool_capability_ready(
            &self.tool_capability_cache,
            &catalog_snapshot,
            &base_url,
            &api_key,
            &mut model,
        );
        if model.tool_capability != previous_cap {
            self.update_catalog_tool_capability(&model.id, model.tool_capability);
        }
        check_result?;
        let turn_id = fresh_turn_id();

        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(target) = &retry {
            truncate_for_retry(&mut inner, target)?;
        }
        let spec = RetrySpec::Agent {
            model_id: payload.model_id.clone(),
            mode: payload.mode,
            reasoning_effort: payload.reasoning_effort.clone(),
            system_instructions: payload.system_instructions.clone(),
            temperature: payload.temperature,
        };
        let session = begin_turn(
            &mut inner,
            &request_id,
            &payload.session_id,
            &payload.prompt,
            Some(spec),
        )?;
        // Mode and model are durable session facts. Full access is not: the
        // flag lives only in this `ChatSession` and is reset on switch/new.
        session.model = Some(model.id.clone());
        session.mode = persist_mode(payload.mode);
        session.auto_approve = matches!(payload.mode, AgentModeDto::FullAccess);
        session.durable = true;
        session.updated_at = skarn_ai::now_millis();
        if session.restored_park {
            // A new turn supersedes a restored park: it can never execute.
            session.pending_approval = None;
            session.pending_card = None;
            session.pending_tool_name = None;
            session.restored_park = false;
        }
        let open_todos = skarn_ai::plan::snapshot(&session.plan_store).todos;
        let tail = conversation_tail(session);
        let plan_store = session.plan_store.clone();
        self.subagents.begin_turn(payload.session_id.as_str());
        drop(inner);
        let rules_dto = self.load_and_cache_rules(&root, self.trust.is_trusted());
        let rules_instructions = self
            .rules_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .1
            .clone();
        let perm_mode = permission_mode_from_dto(payload.mode);
        let effective_local_only = self.trust.effective_local_only();
        let _ = RulesBoundaryPolicy::enforce_immutable_frame(
            perm_mode,
            perm_mode,
            RiskLevel::Low,
            RiskLevel::Low,
            effective_local_only,
            effective_local_only,
        );
        let sys_prompt = system_prompt(&payload, &root, &rules_instructions);
        let mut conversation: Vec<ChatMessage> = vec![ChatMessage::system(sys_prompt)];
        conversation.extend(tail);
        self.flush_session(&payload.session_id);
        log.record_session(
            &app,
            FrontendEvent::RulesLoaded(rules_dto),
            Some(request_id.clone()),
            payload.session_id.clone(),
        );

        // The turn: 5c's loop over 5a's client and 5b's router, rooted and
        // capped exactly like the egui shell's `spawn_agent` (§17.6).
        let caps = TurnCaps {
            max_tool_iterations: self.config.agent.max_iterations,
            max_turn_secs: self.config.agent.max_turn_secs,
            max_cost_usd: self.config.agent.max_cost_usd,
            ..TurnCaps::spec_defaults(model.context_window)
        };
        let cancel_token = DecisionCancelToken::new();
        let mut input = TurnInput::new(model.clone(), conversation)
            .with_caps(caps.clone())
            .with_manifest(manifest_for(payload.mode))
            .with_permission_mode(perm_mode)
            .with_question_timeout_secs(skarn_ai::DEFAULT_QUESTION_TIMEOUT_SECS)
            .with_cancel_token(cancel_token.clone())
            .with_compaction(CompactionConfig::default())
            .with_open_todos(open_todos);
        if let Some(temp) = payload.temperature {
            input = input.with_temperature(temp);
        }
        if let Some(ref effort) = payload.reasoning_effort {
            input = input.with_reasoning_effort(effort.clone());
        }
        let mut tool_settings = agent_tool_settings(&self.config.agent, self.brave_key());
        tool_settings.workspace_trusted = self.trust.is_trusted();
        tool_settings.local_only = effective_local_only;
        // §19.3: a child never inherits the parent's command allowlist or
        // session auto-approve grant: `full` means writes and bash still card
        // in the parent session.
        let child_settings = skarn_ai::AgentToolSettings {
            auto_approve_commands: Vec::new(),
            ..tool_settings.clone()
        };
        self.register_subagent_redaction_key(&api_key);
        let backend: Arc<dyn skarn_ai::SubagentBackend> = Arc::new(DesktopSubagentBackend {
            root: root.clone(),
            settings: child_settings,
            base_url: self.base_url(),
            api_key: api_key.clone(),
        });
        let checkpoint_store = self
            .checkpoint_store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let ws_id = workspace_id_for(&root);
        let tools = RouterToolRunner::new(
            ToolContext::new(root.clone(), tool_settings)
                .with_plan(plan_store)
                .with_subagents(self.subagents.clone(), payload.session_id.as_str())
                .with_subagent_backend(backend, model.clone(), caps.clone())
                .with_permission_mode(perm_mode)
                .with_workspace_reader(self.workspace_reader_for(&root))
                .with_scoped_policy(Arc::clone(&self.scoped_policy))
                .with_checkpoint_store(
                    checkpoint_store,
                    ws_id.as_str(),
                    payload.session_id.as_str(),
                    turn_id.as_str(),
                )
                .with_cancel_token(cancel_token.clone()),
        );
        self.active_cancel_tokens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(request_id.clone(), cancel_token);
        let chat = skarn_ai::ClientChatStream::new(skarn_ai::SkarnClient::new(
            &self.base_url(),
            &api_key,
        ));
        let (events_tx, events_rx) = mpsc::unbounded_channel::<skarn_ai::AgentEvent>();
        // The loop spawns on tauri's async runtime; the decision sender
        // stays here (parked in the active map) for `AgentDecision`.
        let tokio_handle = tauri::async_runtime::handle().inner().clone();
        let (decision_tx, join) =
            skarn_ai::spawn_agent_turn(&tokio_handle, input, chat, tools, events_tx);
        {
            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            inner.active.insert(
                request_id.clone(),
                ActiveRequest::Agent {
                    session_id: payload.session_id.clone(),
                    turn_id: turn_id.clone(),
                    decisions: decision_tx,
                },
            );
        }

        self.ensure_subagent_relay(Arc::clone(&log), app.clone());
        tauri::async_runtime::spawn(agent_relay(AgentRelayCtx {
            log,
            engine: Arc::clone(self),
            app,
            request_id,
            session_id: payload.session_id,
            turn_id,
            api_key,
            events_rx,
            join,
        }));
        Ok(())
    }

    /// Take the runtime's one event receiver and start the session relay.
    /// Called by the first agent turn; later turns reuse the running relay.
    fn ensure_subagent_relay<R: tauri::Runtime>(
        self: &Arc<Self>,
        log: Arc<EventLog>,
        app: tauri::AppHandle<R>,
    ) {
        let receiver = self
            .subagent_events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        let Some(events_rx) = receiver else {
            return;
        };
        tauri::async_runtime::spawn(subagent_relay(SubagentRelayCtx {
            log,
            engine: Arc::clone(self),
            app,
            events_rx,
        }));
    }

    /// Phase 9e: start one background child and emit its single typed
    /// completion event. The runtime outlives the parent turn; the relay
    /// only maps the one accepted result onto `skarn://event`. A cancelled
    /// child emits nothing, and a late result is discarded by the runtime.
    ///
    /// The host-facing spawn tool (Phase 9c) calls this seam; keeping the
    /// relay here means every consumer gets the same event contract.
    #[allow(dead_code)]
    pub(crate) fn spawn_background_subagent<R: tauri::Runtime, F>(
        self: &Arc<Self>,
        log: Arc<EventLog>,
        app: tauri::AppHandle<R>,
        request: skarn_ai::SpawnRequest,
        work: F,
    ) -> Result<skarn_ai::SubagentHandle, skarn_ai::SpawnError>
    where
        F: std::future::Future<Output = skarn_ai::ChildOutcome> + Send + 'static,
    {
        let child_id = request.child_id.clone();
        let handle = self.subagents.spawn_background(request, work)?;
        let runtime = self.subagents.clone();
        tauri::async_runtime::spawn(async move {
            if let Some(completion) = runtime.wait_for_child(&child_id).await {
                let Ok(parent) = SessionId::new(completion.parent_session.clone()) else {
                    return;
                };
                log.record_session(
                    &app,
                    FrontendEvent::SubagentCompleted(subagent_completed_dto(completion)),
                    None,
                    parent,
                );
            }
        });
        Ok(handle)
    }

    // -----------------------------------------------------------------
    // StopChat / AgentDecision
    // -----------------------------------------------------------------

    /// One stop story for both kinds: a chat's cancel watch flips (the
    /// request future is dropped); an agent turn's decision channel gets
    /// `Stop` (the loop kills bash children via the router's abort handle).
    /// Stopping an unknown or finished request is a silent success — the
    /// terminal event already told the frontend how it ended.
    pub(crate) fn stop_chat(
        &self,
        request_id: &RequestId,
        envelope_session: Option<SessionId>,
    ) -> Result<(), ProtocolErrorDto> {
        let _operation = self.operations.lock().unwrap_or_else(|p| p.into_inner());
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let owner = inner
            .active
            .get(request_id)
            .map(|active| active.session_id().clone());
        if let Some(owner) = owner.as_ref() {
            if envelope_session
                .as_ref()
                .is_some_and(|session| session != owner)
            {
                return Err(stale_session());
            }
            if let Some(session) = inner.sessions.get_mut(owner) {
                session.queue_paused = true;
            }
        }
        if let Some(active) = inner.active.get(request_id) {
            if envelope_session.is_some_and(|session| session != *active.session_id()) {
                return Err(stale_session());
            }
            match active {
                ActiveRequest::Chat { cancel, .. } => {
                    let _ = cancel.send(true);
                }
                ActiveRequest::Agent { decisions, .. } => {
                    // Phase 9e: stopping the parent stops its children. The
                    // runtime drops any result that arrives afterwards.
                    let _ = decisions.send((Uuid::nil(), skarn_ai::AgentDecision::Stop));
                    self.subagents.cancel_parent(active.session_id().as_str());
                }
            }
        }
        Ok(())
    }

    /// The approval gate: the parked card is validated and consumed
    /// strictly here — only the matching decision gets through, exactly
    /// once. `Stop` needs no card and forwards to whatever is running.
    #[cfg(test)]
    pub(crate) fn agent_decision(
        &self,
        request_id: &RequestId,
        approval_id: &ApprovalId,
        decision: DecisionDto,
        envelope_session: Option<SessionId>,
    ) -> Result<(), ProtocolErrorDto> {
        self.agent_decision_with_resolution(
            request_id,
            approval_id,
            decision,
            envelope_session,
            None,
        )
    }

    pub(crate) fn agent_decision_with_resolution(
        &self,
        request_id: &RequestId,
        approval_id: &ApprovalId,
        decision: DecisionDto,
        envelope_session: Option<SessionId>,
        resolution: Option<skarn_protocol::AgentDecisionResolutionDto>,
    ) -> Result<(), ProtocolErrorDto> {
        if decision == DecisionDto::Stop {
            return self.stop_chat(request_id, envelope_session);
        }
        let _operation = self.operations.lock().unwrap_or_else(|p| p.into_inner());
        let identity = serde_json::json!({ "request_id": request_id, "session_id": envelope_session, "decision": decision, "resolution": &resolution });
        if let Some(previous) = self
            .accepted_decisions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(approval_id)
        {
            if previous != &identity {
                return Err(idempotency_conflict());
            }
            let still_running = self
                .inner
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .active
                .contains_key(request_id);
            if still_running {
                return Ok(());
            }
        }
        let result = self.agent_decision_once(
            request_id,
            approval_id,
            decision,
            envelope_session,
            resolution,
        );
        if result.is_ok() {
            self.accepted_decisions
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(approval_id.clone(), identity);
        }
        result
    }

    fn agent_decision_once(
        &self,
        request_id: &RequestId,
        approval_id: &ApprovalId,
        decision: DecisionDto,
        envelope_session: Option<SessionId>,
        resolution: Option<skarn_protocol::AgentDecisionResolutionDto>,
    ) -> Result<(), ProtocolErrorDto> {
        let (answer, scoped_resolution) = match (decision, resolution) {
            (
                DecisionDto::Answer,
                Some(skarn_protocol::AgentDecisionResolutionDto::Question { option_id, text }),
            ) => {
                let answer = [option_id.unwrap_or_default(), text.unwrap_or_default()].join("\n");
                if answer.trim().is_empty() || answer.len() > 8000 {
                    return Err(ProtocolErrorDto {
                        code: "invalid_answer".into(),
                        message: "An answer must contain 1 to 8000 bytes.".into(),
                        retryable: false,
                    });
                }
                (Some(answer), None)
            }
            (DecisionDto::Answer, _) => {
                return Err(ProtocolErrorDto {
                    code: "invalid_answer".into(),
                    message: "A structured question answer is required.".into(),
                    retryable: false,
                })
            }
            (
                DecisionDto::Revise,
                Some(skarn_protocol::AgentDecisionResolutionDto::Plan {
                    feedback: Some(feedback),
                    ..
                }),
            ) => {
                if feedback.len() > 8000 {
                    return Err(ProtocolErrorDto {
                        code: "invalid_answer".into(),
                        message: "Revision feedback exceeds 8000 bytes.".into(),
                        retryable: false,
                    });
                }
                (
                    Some(format!(
                        "Revision requested; remain in plan mode. User feedback: {feedback}"
                    )),
                    None,
                )
            }
            (
                _,
                Some(skarn_protocol::AgentDecisionResolutionDto::Scoped {
                    grant,
                    target_pattern,
                }),
            ) => (None, Some((grant, target_pattern))),
            _ => (None, None),
        };
        // Stop applies without a pending card (§17.4: honored regardless of
        // which card it names — the nil id carries no card).
        if decision == DecisionDto::Stop {
            return self.stop_chat(request_id, envelope_session);
        }

        // §19.3: a child card lives on its own rail. Validate the request,
        // session and card type, then let the runtime consume the single-use
        // id before forwarding to the child's turn.
        if let Some(route) = self
            .subagent_cards
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(approval_id)
            .cloned()
        {
            if &route.request_id != request_id
                || envelope_session
                    .as_ref()
                    .is_some_and(|session| session != &route.session_id)
            {
                return Err(stale_session());
            }
            let is_answer = decision == DecisionDto::Answer;
            if route.is_question != is_answer {
                return Err(ProtocolErrorDto {
                    code: "approval_card_mismatch".into(),
                    message: if route.is_question {
                        "This subagent card needs a question answer, not an approval decision."
                            .into()
                    } else {
                        "A question answer cannot approve this subagent card.".into()
                    },
                    retryable: false,
                });
            }
            self.subagent_cards
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(approval_id);
            let loop_id = Uuid::parse_str(approval_id.as_str()).map_err(|_| ProtocolErrorDto {
                code: "approval_mismatch".into(),
                message: "The approval id does not name a pending approval.".into(),
                retryable: false,
            })?;
            return self
                .subagents
                .decide(loop_id, (loop_id, map_decision(decision, answer)))
                .map_err(|error| ProtocolErrorDto {
                    code: "approval_not_pending".into(),
                    message: error.to_string(),
                    retryable: false,
                });
        }

        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // The session whose parked card belongs to this request.
        let Some((session_id, session)) = inner
            .sessions
            .iter_mut()
            .find(|(_, session)| {
                session
                    .pending_approval
                    .as_ref()
                    .is_some_and(|approval| &approval.request_id == request_id)
            })
            .map(|(id, session)| (id.clone(), session))
        else {
            return Err(ProtocolErrorDto {
                code: "approval_not_pending".into(),
                message: "No approval is pending for this request.".into(),
                retryable: false,
            });
        };
        let pending_turn = session
            .pending_approval
            .as_ref()
            .expect("found by pending approval")
            .turn_id
            .clone();
        if envelope_session
            .as_ref()
            .is_some_and(|owner| owner != &session_id)
        {
            return Err(stale_session());
        }
        // §19.4: a question card is answered, never approved; an approval
        // card is never answered. Refuse the wrong card type before the
        // single-use gate consumes the id.
        let pending_is_question = session
            .pending_approval
            .as_ref()
            .expect("found by pending approval")
            .decision_prompt
            .is_some();
        if pending_is_question != (decision == DecisionDto::Answer) {
            return Err(ProtocolErrorDto {
                code: "approval_card_mismatch".into(),
                message: if pending_is_question {
                    "This card needs a question answer, not an approval decision.".into()
                } else {
                    "A question answer cannot approve this card.".into()
                },
                retryable: false,
            });
        }
        if decision == DecisionDto::ApproveAllThisTurn
            && session
                .pending_approval
                .as_ref()
                .is_some_and(|approval| !approval.can_approve_all_this_turn)
        {
            return Err(ProtocolErrorDto {
                code: "invalid_decision".into(),
                message: "This card requires its own decision and cannot grant blanket approval."
                    .into(),
                retryable: false,
            });
        }
        if let Some((grant, _)) = scoped_resolution.as_ref() {
            let is_allow_grant = matches!(
                grant,
                GrantDecisionDto::AllowOnce
                    | GrantDecisionDto::AllowSession
                    | GrantDecisionDto::AllowProject
            );
            if is_allow_grant != (decision == DecisionDto::Approve) {
                return Err(ProtocolErrorDto {
                    code: "invalid_decision".into(),
                    message: "Scoped grant polarity must match approve or deny decision.".into(),
                    retryable: false,
                });
            }
            let persistent_allow = matches!(
                grant,
                GrantDecisionDto::AllowSession | GrantDecisionDto::AllowProject
            );
            let pending_high_risk = session
                .pending_card
                .as_ref()
                .map(|c| pending_card_classification(c, self.trust.is_trusted()).1)
                .is_some_and(|r| matches!(r, RiskLevelDto::High | RiskLevelDto::Critical));
            if persistent_allow && pending_high_risk {
                return Err(ProtocolErrorDto {
                    code: "grant_exceeds_risk_ceiling".into(),
                    message: "GrantExceedsRiskCeiling: persistent AllowSession or AllowProject is forbidden on High/Critical risk actions.".into(),
                    retryable: false,
                });
            }
        }
        // A restored park has no live turn. Consume it exactly once and tell
        // the caller it expired — never execute the restored action.
        if session.restored_park {
            let matches = session
                .pending_approval
                .as_ref()
                .is_some_and(|approval| &approval.approval_id == approval_id);
            if !matches {
                return Err(ProtocolErrorDto {
                    code: "approval_mismatch".into(),
                    message: "The decision does not match the restored approval.".into(),
                    retryable: false,
                });
            }
            session.pending_approval = None;
            session.pending_card = None;
            session.pending_tool_name = None;
            let sid = session_id.clone();
            drop(inner);
            self.flush_session(&sid);
            return Err(ProtocolErrorDto {
                code: "park_expired".into(),
                message: "This approval was restored from disk and has already expired. Ask again to start a new turn.".into(),
                retryable: false,
            });
        }
        // The single-use gate: a mismatch leaves the card parked — only the
        // matching decision consumes it.
        validate_and_consume_approval(
            &mut session.pending_approval,
            request_id,
            &session_id,
            &pending_turn,
            approval_id,
            decision,
        )
        .map_err(|_| ProtocolErrorDto {
            code: "approval_mismatch".into(),
            message: "The decision does not match the pending approval.".into(),
            retryable: false,
        })?;
        session.pending_card = None;
        session.pending_tool_name = None;
        // The loop keys decisions by its own uuid approval id; a decision
        // that survived the gate names a uuid by construction.
        let Ok(loop_id) = Uuid::parse_str(approval_id.as_str()) else {
            return Err(ProtocolErrorDto {
                code: "approval_mismatch".into(),
                message: "The approval id does not name a pending approval.".into(),
                retryable: false,
            });
        };
        let Some(ActiveRequest::Agent {
            session_id: owner,
            decisions,
            ..
        }) = inner.active.get(request_id)
        else {
            let sid = session_id.clone();
            drop(inner);
            self.flush_session(&sid);
            return Err(ProtocolErrorDto {
                code: "approval_expired".into(),
                message: "The turn this approval belongs to has ended.".into(),
                retryable: false,
            });
        };
        if envelope_session.is_some_and(|session| session != *owner) {
            return Err(stale_session());
        }
        let mapped = match scoped_resolution {
            Some((grant, target_pattern)) => skarn_ai::AgentDecision::Scoped {
                grant: grant_decision_from_dto(grant),
                target_pattern,
            },
            None => map_decision(decision, answer),
        };
        let result = decisions
            .send((loop_id, mapped))
            .map_err(|_| ProtocolErrorDto {
                code: "approval_expired".into(),
                message: "The turn this approval belongs to has ended.".into(),
                retryable: false,
            });
        self.question_timeouts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(approval_id);
        drop(inner);
        self.flush_session(&session_id);
        result
    }

    // -----------------------------------------------------------------
    // Phase 9d — RetryMessage
    // -----------------------------------------------------------------

    /// Rewind the session to just before `message_id` and re-send that user
    /// message as a new turn. The rewritten transcript drops the abandoned
    /// branch; the full history of *other* sessions is untouched.
    fn retry_durable_message<R: tauri::Runtime>(
        self: &Arc<Self>,
        log: Arc<EventLog>,
        app: tauri::AppHandle<R>,
        request_id: RequestId,
        session_id: SessionId,
        message_id: MessageId,
    ) -> Result<(), ProtocolErrorDto> {
        let store = self.store().ok_or_else(no_workspace)?;
        let (prompt, model, mode, entries) = {
            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if inner
                .sessions
                .get(&session_id)
                .is_some_and(|session| session.active_request.is_some())
            {
                return Err(ProtocolErrorDto {
                    code: "session_busy".into(),
                    message: "A request is already running for this session — stop it first."
                        .into(),
                    retryable: false,
                });
            }
            if !inner.sessions.contains_key(&session_id) {
                let restored = self.load_session_from_disk(&session_id)?;
                inner.sessions.insert(session_id.clone(), restored);
            }
            let session = inner.sessions.get_mut(&session_id).expect("inserted above");
            let index = session
                .messages
                .iter()
                .position(|message| message.message_id == message_id)
                .ok_or_else(|| ProtocolErrorDto {
                    code: "message_not_found".into(),
                    message: "That message is not in this session.".into(),
                    retryable: false,
                })?;
            if session.messages[index].role != MessageRole::User {
                return Err(ProtocolErrorDto {
                    code: "retry_requires_user_message".into(),
                    message: "Only a user message can be retried.".into(),
                    retryable: false,
                });
            }
            let prompt = session.messages[index].text.clone();
            let loaded = store
                .load(session_id.as_str())
                .map_err(|error| session_persistence_error(&error))?
                .ok_or_else(|| unknown_session(&session_id))?;
            let durable_index = loaded
                .transcript
                .entries
                .iter()
                .position(|entry| entry.id() == message_id.as_str())
                .ok_or_else(|| unknown_message(&message_id))?;
            if loaded.transcript.entries[durable_index..]
                .iter()
                .any(|entry| {
                    let calls = match entry {
                        TranscriptEntry::ToolCalls { calls, .. } => Some(calls),
                        TranscriptEntry::Assistant { tool_calls, .. } => tool_calls.as_ref(),
                        _ => None,
                    };
                    calls.is_some_and(|calls| {
                        calls
                            .iter()
                            .any(|call| skarn_ai::plan::is_side_effecting_tool(&call.name))
                    })
                })
            {
                return Err(ProtocolErrorDto {
                    code: "retry_partial_effects".into(),
                    message: "This turn attempted side effects. Start a new turn to inspect the result; retry could duplicate writes or commands.".into(),
                    retryable: false,
                });
            }
            let entries = loaded.transcript.entries[..durable_index].to_vec();
            store
                .rewrite_transcript(session_id.as_str(), &entries)
                .map_err(|error| session_persistence_error(&error))?;
            session.messages = stored_transcript(&entries);
            session.durable_entries = session.messages.len();
            session.pending_approval = None;
            session.pending_card = None;
            session.pending_tool_name = None;
            session.restored_park = false;
            session.updated_at = skarn_ai::now_millis();
            let model = session.model.clone();
            let mode = session.mode;
            inner.active_session = Some(session_id.clone());
            (prompt, model, mode, entries)
        };
        let _ = entries;
        self.flush_session(&session_id);
        self.send_agent(
            log,
            app,
            Some(session_id.clone()),
            request_id,
            skarn_protocol::AgentRequestDto {
                session_id: session_id.clone(),
                prompt,
                model_id: model,
                mode,
                reasoning_effort: None,
                system_instructions: None,
                temperature: None,
                editor_context: None,
                attachments: Vec::new(),
            },
        )
    }

    // -----------------------------------------------------------------
    // The boot connection probe (the model picker's data path)
    // -----------------------------------------------------------------

    /// Read the key, probe the gateway (`list_models` — connected means up
    /// AND the key is valid), cache the catalog, and announce it through
    /// `ModelsChanged` + `SkarnStatusChanged`. A missing key or a failed
    /// probe is a quiet `Disconnected` (never fabricates a catalog and never
    /// overwrites an explicit persisted model selection).
    pub(crate) async fn connect_probe<R: tauri::Runtime>(
        self: &Arc<Self>,
        log: Arc<EventLog>,
        app: tauri::AppHandle<R>,
    ) {
        let key = match self.fetch_api_key() {
            Ok(key) => key,
            Err(err) => {
                let is_keyring_err = err.code == "keyring_unavailable";
                let reason = if err.code == "not_connected" {
                    None
                } else {
                    Some(err.message)
                };
                {
                    let mut inner = self
                        .inner
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    inner.connection = ConnectionStatus::Disconnected;
                    inner.last_connection_error = reason.clone();
                }
                let active = self.active_model_id();
                log.record(
                    &app,
                    status_changed_with_error(ConnectionStatus::Disconnected, &active, reason),
                    None,
                );
                if is_keyring_err {
                    log.record(
                        &app,
                        FrontendEvent::Toast(ToastDto {
                            severity: ToastSeverity::Warning,
                            message: "Saved API key could not be read from Windows Credential Manager — click Connect to re-enter it.".into(),
                        }),
                        None,
                    );
                }
                return;
            }
        };
        let base_url = self.base_url();
        let client = skarn_ai::SkarnClient::new(&base_url, &key);
        let models = match client.list_models().await {
            Ok(models) if !models.is_empty() => models,
            Ok(_) => {
                let reason = discovery_error_message(
                    &base_url,
                    &key,
                    &SkarnError::Ai("provider returned an empty model catalog".into()),
                );
                {
                    let mut inner = self
                        .inner
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    inner.connection = ConnectionStatus::Disconnected;
                    inner.last_connection_error = Some(reason.clone());
                }
                let active = self.active_model_id();
                log.record(
                    &app,
                    status_changed_with_error(
                        ConnectionStatus::Disconnected,
                        &active,
                        Some(reason),
                    ),
                    None,
                );
                return;
            }
            Err(err) => {
                let reason = discovery_error_message(&base_url, &key, &err);
                {
                    let mut inner = self
                        .inner
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    inner.connection = ConnectionStatus::Disconnected;
                    inner.last_connection_error = Some(reason.clone());
                }
                let active = self.active_model_id();
                log.record(
                    &app,
                    status_changed_with_error(
                        ConnectionStatus::Disconnected,
                        &active,
                        Some(reason),
                    ),
                    None,
                );
                return;
            }
        };
        self.clear_capability_cache_for(&base_url);
        let active_model = {
            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let active_model = reconcile_catalog_active_model(inner.active_model.clone(), &models);
            inner.models = models.clone();
            inner.connection = ConnectionStatus::Connected;
            inner.last_connection_error = None;
            inner.active_model = active_model.clone();
            active_model
        };
        log.record(
            &app,
            FrontendEvent::ModelsChanged(ModelsDto {
                models: models.iter().map(model_dto).collect(),
                active_model_id: active_model.clone(),
            }),
            None,
        );
        log.record(
            &app,
            status_changed(ConnectionStatus::Connected, &active_model),
            None,
        );
    }

    pub(crate) fn set_connection(&self, status: ConnectionStatus) {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .connection = status;
    }

    // -----------------------------------------------------------------
    // Relay-side seams (called between awaits, never across one)
    // -----------------------------------------------------------------

    /// The relay's terminal for a chat stream: unregisters the request and
    /// lands the (possibly partial) assistant message in the transcript.
    fn finish_chat(
        &self,
        request_id: &RequestId,
        session_id: &SessionId,
        assistant: Option<(MessageId, String)>,
    ) {
        {
            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            inner.active.remove(request_id);
            if let Some(session) = inner.sessions.get_mut(session_id) {
                if session.active_request.as_ref() == Some(request_id) {
                    session.active_request = None;
                }
                if let Some((message_id, text)) = assistant {
                    push_message(session, message_id, MessageRole::Assistant, text, None);
                }
                session.updated_at = skarn_ai::now_millis();
            }
        }
        self.flush_session(session_id);
    }

    /// The relay's terminal for an agent turn: same teardown, plus clearing
    /// any parked approval (a decision for a dead turn can never apply).
    fn finish_agent(
        &self,
        request_id: &RequestId,
        session_id: &SessionId,
        assistant: Option<(MessageId, String)>,
    ) {
        {
            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            inner.active.remove(request_id);
            if let Some(session) = inner.sessions.get_mut(session_id) {
                if session.active_request.as_ref() == Some(request_id) {
                    session.active_request = None;
                }
                if let Some(approval) = session.pending_approval.take() {
                    self.question_timeouts
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .remove(&approval.approval_id);
                }
                session.pending_card = None;
                session.pending_tool_name = None;
                session.restored_park = false;
                if let Some((message_id, text)) = assistant {
                    push_message(session, message_id, MessageRole::Assistant, text, None);
                }
                session.updated_at = skarn_ai::now_millis();
            }
        }
        self.active_cancel_tokens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(request_id);
        self.flush_session(session_id);
    }

    /// Park the turn's approval card (the single-use slot). The relay calls
    /// this *before* emitting `ApprovalNeeded`, so a decision can never race
    /// the announcement.
    fn set_pending_approval(
        &self,
        session_id: &SessionId,
        approval: ApprovalDto,
        card: ApprovalCard,
        tool_name: String,
    ) {
        {
            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(session) = inner.sessions.get_mut(session_id) {
                session.pending_approval = Some(approval);
                session.pending_card = Some(card);
                session.pending_tool_name = Some(tool_name);
                session.restored_park = false;
                session.updated_at = skarn_ai::now_millis();
            }
        }
        self.flush_session(session_id);
    }

    // -----------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------

    pub(crate) fn fetch_api_key(&self) -> Result<String, ProtocolErrorDto> {
        let key = self.config.key_source.get().and_then(|key| {
            key.ok_or_else(|| ProtocolErrorDto {
                code: "not_connected".into(),
                message: "No API key is stored — connect first.".into(),
                retryable: true,
            })
        })?;
        self.register_subagent_redaction_key(&key);
        Ok(key)
    }

    fn register_subagent_redaction_key(&self, key: &str) {
        if key.is_empty() {
            return;
        }
        let mut keys = self
            .subagent_redaction_keys
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !keys.iter().any(|known| known == key) {
            keys.push(key.to_owned());
        }
    }

    pub(crate) fn resolve_model(&self, preferred: Option<&str>) -> Result<Model, ProtocolErrorDto> {
        let id = preferred
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                self.active_model_id()
                    .map(|value| value.trim().to_owned())
                    .filter(|value| !value.is_empty())
            })
            .ok_or_else(no_model_selected)?;
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(found) = inner.models.iter().find(|model| {
            model.id == id
                || model.id.eq_ignore_ascii_case(&id)
                || model.display_name.eq_ignore_ascii_case(&id)
        }) {
            return Ok(found.clone());
        }
        let mut bare = Model {
            display_name: id.clone(),
            id,
            context_window: 0,
            price_per_1k_input: 0.0,
            price_per_1k_output: 0.0,
            supports_streaming: true,
            tool_capability: ToolCapability::Unknown,
            reasoning_capability: skarn_ai::ReasoningCapability::Unknown,
            available: true,
            family: String::new(),
        };
        skarn_ai::apply_model_policy(&inner.base_url, &mut bare);
        Ok(bare)
    }

    /// Resolve a model for live dispatch (plain chat, agent turn, or workflow ask).
    ///
    /// - Explicit `preferred` model IDs win for that request.
    /// - Without `preferred`, uses the synchronized active fallback model.
    /// - Refuses retired/unavailable models (e.g. HTTP 410 models).
    /// - Refuses an active fallback model that disappeared from a non-empty
    ///   discovered catalog instead of silently swapping it.
    pub(crate) fn resolve_model_for_dispatch(
        &self,
        preferred: Option<&str>,
    ) -> Result<Model, ProtocolErrorDto> {
        let explicit = preferred.map(str::trim).filter(|value| !value.is_empty());
        let id = explicit
            .map(str::to_owned)
            .or_else(|| {
                self.active_model_id()
                    .map(|value| value.trim().to_owned())
                    .filter(|value| !value.is_empty())
            })
            .ok_or_else(no_model_selected)?;
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(found) = inner.models.iter().find(|model| {
            model.id == id
                || model.id.eq_ignore_ascii_case(&id)
                || model.display_name.eq_ignore_ascii_case(&id)
        }) {
            if !found.is_available() {
                return Err(model_unavailable_error(&found.id, &inner.models, true));
            }
            return Ok(found.clone());
        }
        if explicit.is_none() && !inner.models.is_empty() {
            return Err(model_unavailable_error(&id, &inner.models, false));
        }
        let mut bare = Model {
            display_name: id.clone(),
            id,
            context_window: 0,
            price_per_1k_input: 0.0,
            price_per_1k_output: 0.0,
            supports_streaming: true,
            tool_capability: ToolCapability::Unknown,
            reasoning_capability: skarn_ai::ReasoningCapability::Unknown,
            available: true,
            family: String::new(),
        };
        skarn_ai::apply_model_policy(&inner.base_url, &mut bare);
        if !bare.is_available() {
            return Err(model_unavailable_error(&bare.id, &inner.models, true));
        }
        Ok(bare)
    }

    /// Workflow asks inherit the same per-turn limits, but not session approval grants.
    pub(crate) fn workflow_turn_caps(&self, context_window: u32) -> TurnCaps {
        TurnCaps {
            max_tool_iterations: self.config.agent.max_iterations,
            max_turn_secs: self.config.agent.max_turn_secs,
            max_cost_usd: self.config.agent.max_cost_usd,
            ..TurnCaps::spec_defaults(context_window)
        }
    }

    /// The test seam over [`SessionEngine::resolve_model`] (the boot probe
    /// test proves the cached catalog feeds model resolution).
    #[cfg(test)]
    pub(crate) fn resolve_model_for_test(
        &self,
        preferred: Option<&str>,
    ) -> Result<Model, ProtocolErrorDto> {
        self.resolve_model(preferred)
    }

    /// The Brave key (§17.1 `web_search`): the settings field first, the
    /// keyring entry second (R5's seam — the key never lives in a file once
    /// migrated). Never logged, never in an event.
    fn brave_key(&self) -> Option<String> {
        let from_settings = self
            .config
            .agent
            .brave_search_api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_owned);
        from_settings.or_else(|| match self.config.key_source {
            ApiKeySource::Keyring => skarn_settings::keyring::brave_key_get().ok().flatten(),
            #[cfg(test)]
            ApiKeySource::Fixed(_) | ApiKeySource::Mutable(_) => None,
        })
    }
}

// ---------------------------------------------------------------------------
// The chat relay
// ---------------------------------------------------------------------------

struct ChatRelayCtx<R: tauri::Runtime> {
    log: Arc<EventLog>,
    engine: Arc<SessionEngine>,
    app: tauri::AppHandle<R>,
    request_id: RequestId,
    session_id: SessionId,
    turn_id: TurnId,
    message_id: MessageId,
    base_url: String,
    api_key: String,
    request: ChatRequest,
    cancel_rx: watch::Receiver<bool>,
}

/// Relays `SkarnClient::stream_chat`'s events onto the protocol channel.
/// Mirrors the egui shell's `spawn_chat` (wiring.rs): the stream runs as
/// its own subtask gated on the cancel watch; the relay stops at the first
/// terminal event and always joins the subtask — an error that never
/// reached the channel is still surfaced (`ChatFailed`), a cancel is
/// silent, and the partial bubble is preserved either way.
async fn chat_relay<R: tauri::Runtime>(ctx: ChatRelayCtx<R>) {
    let ChatRelayCtx {
        log,
        engine,
        app,
        request_id,
        session_id,
        turn_id,
        message_id,
        base_url,
        api_key,
        request,
        mut cancel_rx,
    } = ctx;
    let client = skarn_ai::SkarnClient::new(&base_url, &api_key);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<ChatEvent>();

    // The cancel watch: a user Stop flips it, the request future is dropped
    // (the SSE connection closes), and the subtask resolves `None` so the
    // relay stays silent about the cancel.
    let stream = tauri::async_runtime::spawn(async move {
        tokio::select! {
            _ = cancel_rx.changed() => None,
            result = client.stream_chat(request, events_tx) => Some(result),
        }
    });

    let mut delta_seq: u64 = 0;
    let mut text = String::new();
    let mut delta_buffer = SubagentDeltaBuffer::default();
    let mut terminal: Option<Result<ChatReceipt, SkarnError>> = None;
    let mut key_failed = false;
    while let Some(event) = events_rx.recv().await {
        if engine.fetch_api_key().is_err() {
            key_failed = true;
            delta_buffer.pending.clear();
            stream.abort();
            break;
        }
        delta_buffer.keys = engine
            .subagent_redaction_keys
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        match event {
            ChatEvent::Delta(delta) => {
                let delta = delta_buffer.push(&delta, &api_key, false);
                if delta.is_empty() {
                    continue;
                }
                text.push_str(&delta);
                delta_seq += 1;
                log.record_session(
                    &app,
                    FrontendEvent::ChatDelta(ChatDeltaDto {
                        request_id: request_id.clone(),
                        session_id: session_id.clone(),
                        turn_id: turn_id.clone(),
                        message_id: message_id.clone(),
                        sequence: delta_seq,
                        text: delta,
                    }),
                    Some(request_id.clone()),
                    session_id.clone(),
                );
            }
            // Plain chat never arms tools (`ChatRequest::tools` is `None`),
            // so a tool-call fragment cannot arrive here — defensive skip.
            ChatEvent::ToolCallDelta { .. } => continue,
            ChatEvent::Done(receipt) => {
                terminal = Some(Ok(receipt));
                break;
            }
            ChatEvent::Error(error) => {
                terminal = Some(Err(error));
                break;
            }
        }
    }

    let tail = delta_buffer.push("", &api_key, true);
    if !tail.is_empty() {
        text.push_str(&tail);
        delta_seq += 1;
        log.record_session(
            &app,
            FrontendEvent::ChatDelta(ChatDeltaDto {
                request_id: request_id.clone(),
                session_id: session_id.clone(),
                turn_id: turn_id.clone(),
                message_id: message_id.clone(),
                sequence: delta_seq,
                text: tail,
            }),
            Some(request_id.clone()),
            session_id.clone(),
        );
    }

    // Always join the stream task: when it resolved normally its terminal
    // event was already relayed; a cancel resolved `None` (silent by
    // design); a panicking subtask is surfaced as a `ChatFailed` (never a
    // silent cancel, PLAN.md §8).
    let outcome = if key_failed {
        Some(Err(SkarnError::Other(
            "Credential store unavailable; stream stopped.".into(),
        )))
    } else {
        match stream.await {
            Ok(joined) => terminal.or(joined),
            Err(join_error) => Some(Err(SkarnError::Other(format!(
                "chat worker died: {join_error}"
            )))),
        }
    };
    match outcome {
        Some(Ok(receipt)) => {
            // Finalize the bubble first, then announce: a bootstrap taken
            // after the event is consistent with the transcript.
            engine.finish_chat(
                &request_id,
                &session_id,
                Some((message_id.clone(), text.clone())),
            );
            log.record_session(
                &app,
                FrontendEvent::ChatCompleted(ChatCompletedDto {
                    request_id: request_id.clone(),
                    session_id: session_id.clone(),
                    turn_id: turn_id.clone(),
                    message_id: message_id.clone(),
                    text,
                    total_tokens: u64::from(receipt.prompt_tokens)
                        + u64::from(receipt.completion_tokens),
                    cost_usd: receipt.cost_usd,
                }),
                Some(request_id.clone()),
                session_id.clone(),
            );
        }
        Some(Err(error)) => {
            // Keep the partial bubble (the frontend's Retry drops it; the
            // transcript keeps the user turn either way).
            let assistant = if text.is_empty() {
                None
            } else {
                Some((message_id, text.clone()))
            };
            engine.finish_chat(&request_id, &session_id, assistant);
            let keys = engine
                .subagent_redaction_keys
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            let mut error = to_protocol_error(&error, "");
            error.message = redact_with_keys(&error.message, &keys);
            log.record_session(
                &app,
                FrontendEvent::ChatFailed(ChatFailedDto {
                    request_id: request_id.clone(),
                    session_id: session_id.clone(),
                    turn_id,
                    error,
                }),
                Some(request_id),
                session_id.clone(),
            );
        }
        // `None`: cancelled — the partial bubble is preserved (an empty
        // partial adds no transcript entry) and the stop is silent (Phase
        // R1: a user stop is never a ChatError).
        None => {
            let assistant = if text.is_empty() {
                None
            } else {
                Some((message_id.clone(), text.clone()))
            };
            engine.finish_chat(&request_id, &session_id, assistant);
            log.record_session(
                &app,
                FrontendEvent::ChatCompleted(ChatCompletedDto {
                    request_id: request_id.clone(),
                    session_id: session_id.clone(),
                    turn_id: turn_id.clone(),
                    message_id: message_id.clone(),
                    text,
                    total_tokens: 0,
                    cost_usd: 0.0,
                }),
                Some(request_id.clone()),
                session_id.clone(),
            );
        }
    }
    tauri::async_runtime::spawn_blocking(move || engine.kick_queue(log, app, &session_id));
}

// ---------------------------------------------------------------------------
// The agent relay
// ---------------------------------------------------------------------------

struct AgentRelayCtx<R: tauri::Runtime> {
    log: Arc<EventLog>,
    engine: Arc<SessionEngine>,
    app: tauri::AppHandle<R>,
    request_id: RequestId,
    session_id: SessionId,
    turn_id: TurnId,
    api_key: String,
    events_rx: mpsc::UnboundedReceiver<skarn_ai::AgentEvent>,
    join: tokio::task::JoinHandle<skarn_ai::AgentTurnReport>,
}

/// Relays the agent loop's `AgentEvent` stream onto the protocol channel —
/// the web-shell twin of the egui `spawn_agent` relay (wiring.rs). Every
/// round's `State(Thinking)` rotates the streaming `message_id` (one
/// message id per round, the same way the conversation grows a message per
/// round); the last `ToolCallStarted` names the card in
/// `ApprovalNeeded`; `TurnDone` finalizes the transcript, clears the
/// parked approval, and emits the summary footer.
async fn agent_relay<R: tauri::Runtime>(ctx: AgentRelayCtx<R>) {
    let AgentRelayCtx {
        log,
        engine,
        app,
        request_id,
        session_id,
        turn_id,
        api_key,
        mut events_rx,
        join,
    } = ctx;

    let mut current_message: Option<MessageId> = None;
    let mut delta_seq: u64 = 0;
    let mut last_call: Option<skarn_ai::ToolCall> = None;
    let mut done_seen = false;
    let mut final_text = String::new();
    let mut delta_buffer = SubagentDeltaBuffer::default();
    let mut failure = None;

    while let Some(event) = events_rx.recv().await {
        if engine.fetch_api_key().is_err() {
            failure = Some("Credential store unavailable; turn stopped.");
            delta_buffer.pending.clear();
            if let Some(ActiveRequest::Agent { decisions, .. }) = engine
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .active
                .get(&request_id)
            {
                let _ = decisions.send((Uuid::nil(), skarn_ai::AgentDecision::Stop));
            }
            engine.subagents.cancel_parent(session_id.as_str());
            join.abort();
            break;
        }
        let keys = engine
            .subagent_redaction_keys
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        delta_buffer.keys = keys.clone();
        if is_message_boundary(&event) && !delta_buffer.pending.is_empty() {
            // A new message or terminal event ends this message, so a partial
            // key prefix can no longer be completed by a later delta. Ordinary
            // events (tool state, plans, todos) must leave the prefix buffered:
            // the stream may resume and complete the key.
            let text = delta_buffer.push("", &api_key, true);
            final_text.push_str(&text);
            delta_seq += 1;
            let message_id = current_message.get_or_insert_with(fresh_message_id).clone();
            log.record_session(
                &app,
                FrontendEvent::ChatDelta(ChatDeltaDto {
                    request_id: request_id.clone(),
                    session_id: session_id.clone(),
                    turn_id: turn_id.clone(),
                    message_id,
                    sequence: delta_seq,
                    text,
                }),
                Some(request_id.clone()),
                session_id.clone(),
            );
        }
        match event {
            skarn_ai::AgentEvent::State(state) => {
                // A new round is a new streaming message.
                if state == AgentTurnState::Thinking {
                    current_message = Some(fresh_message_id());
                    delta_seq = 0;
                }
                let (phase, tool_name) = map_state(&state);
                log.record_session(
                    &app,
                    FrontendEvent::AgentStateChanged(AgentStateDto {
                        request_id: request_id.clone(),
                        session_id: session_id.clone(),
                        turn_id: turn_id.clone(),
                        state: phase,
                        tool_name: tool_name.map(|name| redact_with_keys(&name, &keys)),
                    }),
                    Some(request_id.clone()),
                    session_id.clone(),
                );
            }
            skarn_ai::AgentEvent::Delta(delta) => {
                let delta = delta_buffer.push(&delta, &api_key, false);
                if delta.is_empty() {
                    continue;
                }
                final_text.push_str(&delta);
                delta_seq += 1;
                let message_id = current_message.get_or_insert_with(fresh_message_id).clone();
                log.record_session(
                    &app,
                    FrontendEvent::ChatDelta(ChatDeltaDto {
                        request_id: request_id.clone(),
                        session_id: session_id.clone(),
                        turn_id: turn_id.clone(),
                        message_id,
                        sequence: delta_seq,
                        text: delta,
                    }),
                    Some(request_id.clone()),
                    session_id.clone(),
                );
            }
            skarn_ai::AgentEvent::ToolCallStarted(call) => {
                match engine.record_tool_calls(&session_id, vec![call.clone()]) {
                    // The keyring is temporarily unreadable: the record is
                    // queued and retried, so the turn continues.
                    Ok(()) | Err(TranscriptAppend::Queued) => {}
                    Err(TranscriptAppend::Failed) => {
                        failure = Some("Could not persist tool call; turn stopped.");
                        if let Some(ActiveRequest::Agent { decisions, .. }) = engine
                            .inner
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .active
                            .get(&request_id)
                        {
                            let _ = decisions.send((Uuid::nil(), skarn_ai::AgentDecision::Stop));
                        }
                        engine.subagents.cancel_parent(session_id.as_str());
                        join.abort();
                        break;
                    }
                }
                log.record_session(
                    &app,
                    FrontendEvent::ToolCallStarted(ToolCallStartedDto {
                        request_id: request_id.clone(),
                        session_id: session_id.clone(),
                        turn_id: turn_id.clone(),
                        call_id: redact_with_keys(&call.id, &keys),
                        tool_name: redact_with_keys(&call.name, &keys),
                        args_json: redact_json_text(&call.arguments, &keys),
                    }),
                    Some(request_id.clone()),
                    session_id.clone(),
                );
                last_call = Some(call);
            }
            skarn_ai::AgentEvent::ToolResult(result) => {
                if result.ok
                    && last_call.as_ref().is_some_and(|call| {
                        matches!(
                            call.name.as_str(),
                            "write_file" | "edit_file" | "delete_file"
                        )
                    })
                {
                    engine.refresh_workspace_buffers();
                }
                match engine.record_tool_result(&session_id, &result) {
                    Ok(()) | Err(TranscriptAppend::Queued) => {}
                    Err(TranscriptAppend::Failed) => {
                        failure = Some("Could not persist tool result; turn stopped.");
                        if let Some(ActiveRequest::Agent { decisions, .. }) = engine
                            .inner
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .active
                            .get(&request_id)
                        {
                            let _ = decisions.send((Uuid::nil(), skarn_ai::AgentDecision::Stop));
                        }
                        engine.subagents.cancel_parent(session_id.as_str());
                        join.abort();
                        break;
                    }
                }
                log.record_session(
                    &app,
                    FrontendEvent::ToolResult(ToolResultDto {
                        request_id: request_id.clone(),
                        session_id: session_id.clone(),
                        turn_id: turn_id.clone(),
                        call_id: redact_with_keys(&result.id, &keys),
                        result_summary: redact_with_keys(&result.content, &keys),
                        is_err: !result.ok,
                    }),
                    Some(request_id.clone()),
                    session_id.clone(),
                );
            }
            skarn_ai::AgentEvent::ApprovalNeeded { approval_id, card } => {
                let card = redact_card_with_keys(card, &keys);
                let tool_name = last_call
                    .as_ref()
                    .map(|call| redact_with_keys(&call.name, &keys))
                    .unwrap_or_else(|| fallback_tool_name(&card));
                let (path_or_command, diff_or_input, workflow) = approval_body(&card);
                let plan = approval_plan(&card);
                let prompt = approval_prompt(&card);
                let approval_id_dto = ApprovalId::new(approval_id.to_string())
                    .expect("uuid string is a valid id");
                let (action_class_dto, risk_dto) =
                    pending_card_classification(&card, engine.trust.is_trusted());
                let question_timeout = if matches!(&card, ApprovalCard::Question(_)) {
                    let timeout = QuestionTimeoutDto {
                        deadline_ms: skarn_ai::now_millis()
                            .saturating_add(skarn_ai::DEFAULT_QUESTION_TIMEOUT_SECS * 1000),
                        auto_continue_allowed: true,
                        timed_out: false,
                        paused: false,
                    };
                    engine
                        .question_timeouts
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .insert(approval_id_dto.clone(), timeout.clone());
                    Some(timeout)
                } else {
                    None
                };
                // Park before announcing: the single-use slot exists before
                // the frontend can possibly answer.
                engine.set_pending_approval(
                    &session_id,
                    ApprovalDto {
                        approval_id: approval_id_dto.clone(),
                        request_id: request_id.clone(),
                        turn_id: turn_id.clone(),
                        session_id: session_id.clone(),
                        summary: format!("{tool_name} {path_or_command}"),
                        can_approve_all_this_turn: !matches!(
                            &card,
                            ApprovalCard::Plan { .. }
                                | ApprovalCard::Workflow { .. }
                                | ApprovalCard::Question(_)
                        ),
                        plan,
                        decision_prompt: prompt.clone(),
                    },
                    card.clone(),
                    tool_name.clone(),
                );
                log.record_session(
                    &app,
                    FrontendEvent::ApprovalNeeded(ApprovalNeededDto {
                        request_id: request_id.clone(),
                        session_id: session_id.clone(),
                        turn_id: turn_id.clone(),
                        approval_id: approval_id_dto,
                        tool_name,
                        path_or_command,
                        diff_or_input,
                        workflow,
                        plan: approval_plan(&card),
                        prompt,
                        subagent_id: None,
                        risk: Some(risk_dto),
                        action_class: Some(action_class_dto),
                        question_timeout,
                    }),
                    Some(request_id.clone()),
                    session_id.clone(),
                );
            }
            skarn_ai::AgentEvent::PlanChanged { phase, plan } => {
                engine.sync_plan_store(&session_id);
                log.record_session(
                    &app,
                    FrontendEvent::PlanStateChanged(PlanStateDto {
                        session_id: session_id.clone(),
                        phase: plan_phase_dto(phase),
                        plan: plan
                            .as_ref()
                            .map(|draft| scrub_serializable(plan_approval_dto(draft, ""), &keys)),
                    }),
                    Some(request_id.clone()),
                    session_id.clone(),
                );
            }
            skarn_ai::AgentEvent::TodosChanged(todos) => {
                engine.sync_plan_store(&session_id);
                log.record_session(
                    &app,
                    FrontendEvent::TodosChanged(TodosDto {
                        session_id: session_id.clone(),
                        todos: todos
                            .iter()
                            .map(|todo| scrub_serializable(todo_dto(todo), &keys))
                            .collect(),
                    }),
                    Some(request_id.clone()),
                    session_id.clone(),
                );
            }
            skarn_ai::AgentEvent::DecisionTrace(trace) => {
                log.record_session(
                    &app,
                    FrontendEvent::DecisionTrace(decision_trace_dto(
                        &request_id,
                        &session_id,
                        &turn_id,
                        &trace,
                        &keys,
                    )),
                    Some(request_id.clone()),
                    session_id.clone(),
                );
            }
            skarn_ai::AgentEvent::QuestionCountdown {
                approval_id,
                deadline_ms,
                auto_continue_allowed,
                timed_out,
                paused,
            } => {
                let approval_id_dto = ApprovalId::new(approval_id.to_string())
                    .expect("uuid string is a valid id");
                let timeout = QuestionTimeoutDto {
                    deadline_ms,
                    auto_continue_allowed,
                    timed_out,
                    paused,
                };
                if timed_out {
                    engine
                        .question_timeouts
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .remove(&approval_id_dto);
                    let mut inner = engine
                        .inner
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    if let Some(session) = inner.sessions.get_mut(&session_id) {
                        if session
                            .pending_approval
                            .as_ref()
                            .is_some_and(|p| p.approval_id == approval_id_dto)
                        {
                            session.pending_approval = None;
                            session.pending_card = None;
                            session.pending_tool_name = None;
                        }
                    }
                    log.consume_agent_approval(&approval_id_dto);
                } else {
                    engine
                        .question_timeouts
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .insert(approval_id_dto.clone(), timeout.clone());
                }
                log.record_session(
                    &app,
                    FrontendEvent::QuestionCountdownUpdated(QuestionCountdownUpdatedDto {
                        request_id: request_id.clone(),
                        session_id: session_id.clone(),
                        turn_id: turn_id.clone(),
                        approval_id: approval_id_dto,
                        timeout,
                    }),
                    Some(request_id.clone()),
                    session_id.clone(),
                );
            }
            skarn_ai::AgentEvent::ScopedGrantsUpdated(entries) => {
                engine.sync_persisted_grants();
                let grants: Vec<ScopedGrantDto> = entries.iter().map(scoped_grant_dto).collect();
                log.record_session(
                    &app,
                    FrontendEvent::ScopedGrantsChanged(ScopedGrantsChangedDto {
                        session_id: Some(session_id.clone()),
                        grants,
                    }),
                    Some(request_id.clone()),
                    session_id.clone(),
                );
            }
            skarn_ai::AgentEvent::TurnCheckpointRecorded(manifest) => {
                let summary = engine.record_turn_checkpoint(&session_id, &manifest);
                log.record_session(
                    &app,
                    FrontendEvent::TurnCheckpointUpdated(summary),
                    Some(request_id.clone()),
                    session_id.clone(),
                );
            }
            skarn_ai::AgentEvent::Compacted(cursor) => {
                engine.record_compaction(&session_id, cursor);
            }
            skarn_ai::AgentEvent::CompactionFailed(error) => {
                let (message, partial_summary) = match &error {
                    CompactionError::SummaryFailed {
                        message,
                        partial_summary,
                    } => (message.clone(), partial_summary.clone()),
                    other => (other.to_string(), None),
                };
                log.record_session(
                    &app,
                    FrontendEvent::AgentCompactionFailed(AgentCompactionFailedDto {
                        request_id: request_id.clone(),
                        session_id: session_id.clone(),
                        turn_id: turn_id.clone(),
                        message: redact_with_keys(&message, &keys),
                        partial_summary: partial_summary
                            .map(|summary| redact_with_keys(&summary, &keys)),
                    }),
                    Some(request_id.clone()),
                    session_id.clone(),
                );
            }
            skarn_ai::AgentEvent::TurnDone(report) => {
                done_seen = true;
                // A queued tool record that still cannot be written is a
                // visible turn failure, never a silent hole in the
                // transcript.
                let persisted = engine.persist_pending_transcript().is_ok();
                // The final answer is the last round's message; an empty
                // final (a tools-only abort) adds nothing.
                let assistant = if report.final_message.trim().is_empty() {
                    None
                } else {
                    let message_id = current_message.get_or_insert_with(fresh_message_id).clone();
                    Some((message_id, redact_with_keys(&report.final_message, &keys)))
                };
                engine.finish_agent(&request_id, &session_id, assistant);
                if final_text.is_empty() && !report.final_message.trim().is_empty() {
                    let message_id = current_message.get_or_insert_with(fresh_message_id).clone();
                    log.record_session(
                        &app,
                        FrontendEvent::ChatDelta(ChatDeltaDto {
                            request_id: request_id.clone(),
                            session_id: session_id.clone(),
                            turn_id: turn_id.clone(),
                            message_id,
                            sequence: 1,
                            text: redact_with_keys(&report.final_message, &keys),
                        }),
                        Some(request_id.clone()),
                        session_id.clone(),
                    );
                }
                log.record_session(
                    &app,
                    FrontendEvent::AgentTurnDone(AgentTurnDoneDto {
                        request_id: request_id.clone(),
                        session_id: session_id.clone(),
                        turn_id: turn_id.clone(),
                        total_tokens: report.summary.tokens,
                        cost_usd: report.summary.cost_usd,
                        num_tools: report.summary.tools_used,
                        approved: report.summary.approved,
                        denied: report.summary.denied,
                        completed: report.summary.completed && persisted,
                        end_message: if persisted {
                            redact_with_keys(&report.end.toast(), &keys)
                        } else {
                            "Could not persist tool records; credential store unavailable.".into()
                        },
                    }),
                    Some(request_id.clone()),
                    session_id.clone(),
                );
                break;
            }
        }
    }

    // Always join the loop task (the worker-died pattern, PLAN.md §8): a
    // turn whose event channel closed before `TurnDone` still gets one
    // terminal event — from the report if it completed, else a synthetic
    // `Failed`.
    if !done_seen {
        if failure.is_none() && !delta_buffer.pending.is_empty() {
            let text = delta_buffer.push("", &api_key, true);
            final_text.push_str(&text);
            delta_seq += 1;
            let message_id = current_message.get_or_insert_with(fresh_message_id).clone();
            log.record_session(
                &app,
                FrontendEvent::ChatDelta(ChatDeltaDto {
                    request_id: request_id.clone(),
                    session_id: session_id.clone(),
                    turn_id: turn_id.clone(),
                    message_id,
                    sequence: delta_seq,
                    text,
                }),
                Some(request_id.clone()),
                session_id.clone(),
            );
        }
        if failure.is_none() && engine.persist_pending_transcript().is_err() {
            // The keyring stayed unavailable for the whole turn: report the
            // unwritten tool records instead of ending as if they landed.
            failure = Some("Could not persist tool records; credential store unavailable.");
        }
        let (summary, end_message) = if let Some(message) = failure {
            (
                AgentSummary {
                    iterations: 0,
                    tools_used: 0,
                    approved: 0,
                    denied: 0,
                    tokens: 0,
                    cost_usd: 0.0,
                    completed: false,
                },
                message.to_owned(),
            )
        } else {
            match join.await {
                Ok(report) => (report.summary, report.end.toast()),
                Err(join_error) => (
                    AgentSummary {
                        iterations: 0,
                        tools_used: 0,
                        approved: 0,
                        denied: 0,
                        tokens: 0,
                        cost_usd: 0.0,
                        completed: false,
                    },
                    format!("agent worker died: {join_error}"),
                ),
            }
        };
        let assistant = if final_text.is_empty() {
            None
        } else {
            Some((fresh_message_id(), final_text.clone()))
        };
        engine.finish_agent(&request_id, &session_id, assistant);
        let keys = engine
            .subagent_redaction_keys
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        log.record_session(
            &app,
            FrontendEvent::AgentTurnDone(AgentTurnDoneDto {
                request_id: request_id.clone(),
                session_id: session_id.clone(),
                turn_id,
                total_tokens: summary.tokens,
                cost_usd: summary.cost_usd,
                num_tools: summary.tools_used,
                approved: summary.approved,
                denied: summary.denied,
                completed: summary.completed,
                end_message: redact_with_keys(&end_message, &keys),
            }),
            Some(request_id),
            session_id.clone(),
        );
    }
    tauri::async_runtime::spawn_blocking(move || engine.kick_queue(log, app, &session_id));
}

// ---------------------------------------------------------------------------
// The subagent relay (completions + child cards)
// ---------------------------------------------------------------------------

struct SubagentRelayCtx<R: tauri::Runtime> {
    log: Arc<EventLog>,
    engine: Arc<SessionEngine>,
    app: tauri::AppHandle<R>,
    events_rx: mpsc::Receiver<skarn_ai::SubagentEvent>,
}

#[derive(Default)]
struct SubagentDeltaBuffer {
    pending: String,
    keys: Vec<String>,
    sequence: u64,
}

impl SubagentDeltaBuffer {
    fn push(&mut self, text: &str, key: &str, flush: bool) -> String {
        if !key.is_empty() && !self.keys.iter().any(|known| known == key) {
            self.keys.push(key.to_owned());
        }
        self.pending.push_str(text);
        let mut output = String::new();
        let mut consumed = 0;
        while consumed < self.pending.len() {
            let remaining = &self.pending[consumed..];
            if !flush
                && self
                    .keys
                    .iter()
                    .any(|key| key.len() > remaining.len() && key.starts_with(remaining))
            {
                break;
            } else if let Some(key) = self
                .keys
                .iter()
                .filter(|key| remaining.starts_with(key.as_str()))
                .max_by_key(|key| key.len())
            {
                consumed += key.len();
                output.push_str("[REDACTED]");
            } else {
                let ch = remaining.chars().next().expect("remaining is not empty");
                consumed += ch.len_utf8();
                output.push(ch);
            }
        }
        self.pending.drain(..consumed);
        output
    }
}

type SubagentDeltaBuffers = BTreeMap<(String, String, String), SubagentDeltaBuffer>;

/// True for events that end one streaming message: only there can a buffered
/// key prefix never be completed by a later delta. Every other event is
/// ordinary output and must preserve the buffered text.
fn is_message_boundary(event: &skarn_ai::AgentEvent) -> bool {
    matches!(
        event,
        skarn_ai::AgentEvent::State(AgentTurnState::Thinking) | skarn_ai::AgentEvent::TurnDone(_)
    )
}

fn redact_with_keys(text: &str, keys: &[String]) -> String {
    let mut buffer = SubagentDeltaBuffer {
        keys: keys.iter().filter(|key| !key.is_empty()).cloned().collect(),
        ..Default::default()
    };
    buffer.push(text, "", true)
}

fn emit_subagent_delta<R: tauri::Runtime>(
    log: &EventLog,
    app: &tauri::AppHandle<R>,
    identity: &(String, String, String),
    buffer: &mut SubagentDeltaBuffer,
    text: String,
) {
    if text.is_empty() {
        return;
    }
    let Ok(parent) = SessionId::new(identity.0.clone()) else {
        return;
    };
    buffer.sequence += 1;
    log.record_session(
        app,
        FrontendEvent::SubagentActivity(SubagentActivityDto::Delta {
            child_id: identity.1.clone(),
            parent_session_id: parent.clone(),
            message_id: redact_with_keys(&identity.2, &buffer.keys),
            sequence: buffer.sequence,
            text,
        }),
        RequestId::new(identity.1.clone()).ok(),
        parent,
    );
}

fn flush_subagent_deltas<R: tauri::Runtime>(
    buffers: &mut SubagentDeltaBuffers,
    log: &EventLog,
    app: &tauri::AppHandle<R>,
    child: Option<(&str, &str)>,
    key: &str,
) {
    buffers.retain(|identity, buffer| {
        if child.is_some_and(|(parent, child)| identity.0 != parent || identity.1 != child) {
            return true;
        }
        let text = buffer.push("", key, true);
        emit_subagent_delta(log, app, identity, buffer, text);
        false
    });
}

/// Map the shared runtime's events onto the normal session stream. A
/// completion lands in the roster once (the frontend dedupes by child id) and
/// a child card parks on the same approval rail as the parent's own cards.
async fn subagent_relay<R: tauri::Runtime>(ctx: SubagentRelayCtx<R>) {
    let SubagentRelayCtx {
        log,
        engine,
        app,
        mut events_rx,
    } = ctx;
    let mut delta_buffers = SubagentDeltaBuffers::new();
    let mut changes = engine.subagents.subscribe_changes();
    let mut sink_closed = false;
    loop {
        // Drain already queued activity before terminal events retained on
        // overflow. The retained terminal is bounded to one per child.
        let event = match events_rx.try_recv() {
            Ok(event) => Some(event),
            Err(mpsc::error::TryRecvError::Disconnected) => {
                sink_closed = true;
                engine.subagents.take_sink_terminal()
            }
            Err(mpsc::error::TryRecvError::Empty) => {
                if let Some(event) = engine.subagents.take_sink_terminal() {
                    Some(event)
                } else {
                    tokio::select! {
                        event = events_rx.recv() => event,
                        _ = changes.changed() => continue,
                    }
                }
            }
        };
        let Some(event) = event else { break };
        // Preserve event order while the keyring is unavailable. No event or
        // buffered text is emitted until the active key can be read again.
        let Ok(api_key) = engine.fetch_api_key() else {
            // Cancel the child before discarding a card that cannot be shown.
            let child = match &event {
                skarn_ai::SubagentEvent::Activity(activity) => Some(activity.child_id.as_str()),
                skarn_ai::SubagentEvent::ApprovalNeeded(card) => Some(card.child_id.as_str()),
                _ => None,
            };
            if let Some(child) = child {
                engine.subagents.cancel_child(child);
                engine
                    .subagent_cards
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .retain(|_, route| route.child_id != child);
            }
            if matches!(
                &event,
                skarn_ai::SubagentEvent::Cancelled { .. } | skarn_ai::SubagentEvent::Completed(_)
            ) {
                engine.subagents.defer_sink_terminal(event);
                if sink_closed {
                    // No producer remains to wake this relay again; keep the
                    // terminal reserved for the next reader instead of
                    // spinning on it while the keyring is down.
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            delta_buffers.clear();
            continue;
        };
        engine.register_subagent_redaction_key(&api_key);
        let keys = engine
            .subagent_redaction_keys
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        for buffer in delta_buffers.values_mut() {
            for key in &keys {
                if !buffer.keys.contains(key) {
                    buffer.keys.push(key.clone());
                }
            }
        }
        match event {
            skarn_ai::SubagentEvent::Completed(completion) => {
                flush_subagent_deltas(
                    &mut delta_buffers,
                    &log,
                    &app,
                    Some((&completion.parent_session, &completion.child_id)),
                    &api_key,
                );
                let Ok(parent) = SessionId::new(completion.parent_session.clone()) else {
                    continue;
                };
                let mut dto = subagent_completed_dto(completion);
                dto.error = dto.error.map(|text| redact_with_keys(&text, &keys));
                dto.final_message = redact_with_keys(&dto.final_message, &keys);
                for artifact in &mut dto.artifacts {
                    artifact.kind = redact_with_keys(&artifact.kind, &keys);
                    artifact.path = redact_with_keys(&artifact.path, &keys);
                }
                log.record_session(&app, FrontendEvent::SubagentCompleted(dto), None, parent);
            }
            skarn_ai::SubagentEvent::Activity(activity) => {
                if let skarn_ai::SubagentActivityKind::Delta {
                    message_id, text, ..
                } = &activity.kind
                {
                    if SessionId::new(activity.parent_session.clone()).is_err() {
                        continue;
                    }
                    let identity = (
                        activity.parent_session.clone(),
                        activity.child_id.clone(),
                        message_id.clone(),
                    );
                    let buffer = delta_buffers.entry(identity.clone()).or_default();
                    buffer.keys = keys.clone();
                    let text = buffer.push(text, &api_key, false);
                    emit_subagent_delta(&log, &app, &identity, buffer, text);
                    continue;
                }
                let child_id = activity.child_id.clone();
                let Ok(parent) = SessionId::new(activity.parent_session.clone()) else {
                    continue;
                };
                let Some(activity) = subagent_activity_dto(activity, &keys) else {
                    continue;
                };
                log.record_session(
                    &app,
                    FrontendEvent::SubagentActivity(activity),
                    RequestId::new(child_id).ok(),
                    parent,
                );
            }
            skarn_ai::SubagentEvent::Cancelled {
                child_id,
                parent_session,
            } => {
                engine
                    .subagent_cards
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .retain(|_, route| route.child_id != child_id);
                flush_subagent_deltas(
                    &mut delta_buffers,
                    &log,
                    &app,
                    Some((&parent_session, &child_id)),
                    &api_key,
                );
                let Ok(parent) = SessionId::new(parent_session) else {
                    continue;
                };
                log.record_session(
                    &app,
                    FrontendEvent::SubagentCancelled(SubagentCancelledDto {
                        child_id: child_id.clone(),
                        parent_session_id: parent.clone(),
                    }),
                    RequestId::new(child_id).ok(),
                    parent,
                );
            }
            skarn_ai::SubagentEvent::ApprovalNeeded(child_card) => {
                if matches!(
                    engine.subagents.child_state(&child_card.child_id),
                    Some(skarn_ai::ChildState::Cancelled)
                ) {
                    continue;
                }
                let Ok(session_id) = SessionId::new(child_card.parent_session.clone()) else {
                    continue;
                };
                let approval_id = ApprovalId::new(child_card.approval_id.to_string())
                    .expect("uuid string is a valid id");
                let child_id = child_card.child_id.clone();
                let card = redact_card_with_keys(child_card.card, &keys);
                let tool_name = fallback_tool_name(&card);
                let (path_or_command, diff_or_input, workflow) = approval_body(&card);
                let plan = approval_plan(&card);
                let prompt = approval_prompt(&card);
                let is_question = matches!(&card, ApprovalCard::Question(_));
                let (request_id, turn_id) = engine
                    .active_request_for_session(&session_id)
                    .unwrap_or_else(|| {
                        (
                            RequestId::new(format!("subagent-{child_id}"))
                                .expect("minted request id"),
                            TurnId::new(format!("turn-{child_id}")).expect("minted turn id"),
                        )
                    });
                engine
                    .subagent_cards
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .insert(
                        approval_id.clone(),
                        SubagentCardRoute {
                            request_id: request_id.clone(),
                            session_id: session_id.clone(),
                            is_question,
                            child_id: child_id.clone(),
                        },
                    );
                let (action_class_dto, risk_dto) =
                    pending_card_classification(&card, engine.trust.is_trusted());
                log.record_session(
                    &app,
                    FrontendEvent::ApprovalNeeded(ApprovalNeededDto {
                        request_id: request_id.clone(),
                        session_id: session_id.clone(),
                        turn_id,
                        approval_id,
                        tool_name,
                        path_or_command,
                        diff_or_input,
                        workflow,
                        plan,
                        prompt,
                        subagent_id: Some(child_id),
                        risk: Some(risk_dto),
                        action_class: Some(action_class_dto),
                        question_timeout: None,
                    }),
                    Some(request_id),
                    session_id,
                );
            }
        }
    }
    if let Ok(api_key) = engine.fetch_api_key() {
        if !api_key.is_empty() {
            engine.register_subagent_redaction_key(&api_key);
            let keys = engine
                .subagent_redaction_keys
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            for buffer in delta_buffers.values_mut() {
                buffer.keys = keys.clone();
            }
            flush_subagent_deltas(&mut delta_buffers, &log, &app, None, &api_key);
        }
    }
}

/// The desktop subagent backend: one fresh chat stream and one policy-scoped
/// router per child turn. It deliberately carries no session approval grant.
struct DesktopSubagentBackend {
    root: PathBuf,
    settings: skarn_ai::AgentToolSettings,
    base_url: String,
    api_key: String,
}

impl skarn_ai::SubagentBackend for DesktopSubagentBackend {
    fn chat(&self) -> Result<Box<dyn skarn_ai::ChatStream>, String> {
        Ok(Box::new(skarn_ai::ClientChatStream::new(
            skarn_ai::SkarnClient::new(&self.base_url, &self.api_key),
        )))
    }

    fn tools(&self, policy: skarn_ai::SubagentPolicy) -> Result<Box<dyn ToolRunner>, String> {
        let context =
            ToolContext::new(self.root.clone(), self.settings.clone()).with_policy(policy);
        Ok(Box::new(RouterToolRunner::new(context)))
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// The agent turn's system prompt: the mode-correct approval note, the
/// reasoning-depth line, the user's extra instructions, and the editor
/// reference (path/language/selection/dirty — never the file text; the
/// tools read the bytes).
fn system_prompt(
    payload: &skarn_protocol::AgentRequestDto,
    root: &Path,
    rules_instructions: &str,
) -> String {
    let depth_instructions = match payload.reasoning_effort.as_deref() {
        Some("none") => "\nReasoning depth: Off. Provide direct answers without extended thinking preamble.",
        Some("max") => "\nReasoning depth: Maximum. Provide deep, rigorous chain of thought. Analyze all edge cases, potential failure modes, and architectural implications before executing actions.",
        Some("quick") => "\nReasoning depth: Quick. Be concise and direct. Focus on immediate solution without excessive explanation.",
        _ => "\nReasoning depth: Standard. Balanced reasoning with clear explanations and thorough verification.",
    };

    let custom_sys = match payload.system_instructions.as_deref() {
        Some(custom) if !custom.trim().is_empty() => {
            format!("\nAdditional User Instructions:\n{}", custom.trim())
        }
        _ => String::new(),
    };

    // The prompt must match the mode: auto-applied writes never show the
    // approval card the default prompt describes.
    let approval_note = match payload.mode {
        AgentModeDto::AutoEdit => {
            "File writes and edits apply immediately with turn checkpoints; deletes and bash commands still pause for approval."
        }
        AgentModeDto::FullAccess => {
            "Low/medium-risk edits and trusted allowlisted commands apply immediately; high/critical actions still pause for approval."
        }
        AgentModeDto::ReadOnly => {
            "This session is read-only: inspect and report, but do not attempt a write."
        }
        AgentModeDto::AskBeforeChanges => {
            "Writes, deletes, and non-allowlisted commands pause for the user's approval card; explain what you are doing while the card is pending."
        }
    };

    // Empty without attachments, so the prompt stays byte-identical when none are sent.
    let attachments_note = if payload.attachments.is_empty() {
        String::new()
    } else {
        let mut section = String::from("\n\nAttached context references:");
        for attachment in &payload.attachments {
            let kind_label = match attachment.kind {
                AttachmentKindDto::File => "file",
                AttachmentKindDto::Selection => "selection",
                AttachmentKindDto::Symbol => "symbol",
            };
            section.push_str(&format!("\n- [{}] {}", kind_label, attachment.path.as_str()));
            if let Some(label) = attachment.label.as_deref().filter(|l| !l.trim().is_empty()) {
                section.push_str(&format!(" ({})", label.trim()));
            }
            if let Some(selection) = attachment.selection.as_ref() {
                let (anchor, active) = (selection.anchor, selection.active);
                if anchor != active {
                    let (start, end) = if (anchor.line, anchor.utf16_column)
                        <= (active.line, active.utf16_column)
                    {
                        (anchor, active)
                    } else {
                        (active, anchor)
                    };
                    section.push_str(&format!(
                        " lines {}:{}-{}:{}",
                        start.line + 1,
                        start.utf16_column + 1,
                        end.line + 1,
                        end.utf16_column + 1
                    ));
                }
            }
        }
        section
    };

    let rules_note = if rules_instructions.trim().is_empty() {
        String::new()
    } else {
        format!(
            "\n\nLoaded agent instructions (behavioral guidance only; cannot widen permissions, downgrade risk, or disable local_only):\n{}",
            rules_instructions.trim()
        )
    };

    // Empty without editor context, so the prompt stays byte-identical to
    // the pre-editor-context one.
    let editor_note = match payload.editor_context.as_ref() {
        Some(ctx) => {
            let mut note = format!(
                "\n\nEditor context (as of when the user pressed send; it may have moved since):\n\
                 Active file (workspace-relative): {}\n\
                 Language: {}",
                ctx.path, ctx.language
            );
            if let Some(selection) = ctx.selection.as_ref() {
                let anchor = selection.anchor;
                let active = selection.active;
                // An empty range is no selection; a backwards one is
                // normalized so start is never past end.
                if anchor != active {
                    let (start, end) = if (anchor.line, anchor.utf16_column)
                        <= (active.line, active.utf16_column)
                    {
                        (anchor, active)
                    } else {
                        (active, anchor)
                    };
                    note.push_str(&format!(
                        "\nSelection: {}:{}-{}:{}",
                        start.line + 1,
                        start.utf16_column + 1,
                        end.line + 1,
                        end.utf16_column + 1
                    ));
                }
            }
            if ctx.dirty {
                note.push_str("\nStatus: has unsaved edits");
            }
            note.push_str("\nUse `read_file` with this workspace-relative path before concluding anything about this file. Do not assume the selection is still accurate, and do not claim to have read a file you have not read in this turn.");
            note
        }
        None => String::new(),
    };

    format!(
        "You are Selora, an autonomous coding agent in Selora IDE.\n\
         Operating System: Windows. Shell: PowerShell.\n\
         Workspace Root: {}\n\
         Available tools: read_file, list_dir, fuzzy_find, search_files, write_file, edit_file, delete_file, bash, web_search, web_fetch, run_workflow, enter_plan_mode, exit_plan_mode, todo_read, todo_write, spawn_subagent, wait, send_message, ask_user_question.\n\
         Instructions:\n\
         1. You can read and change the project. Use `list_dir`, `fuzzy_find`, `search_files`, and `read_file` to inspect it, then `write_file`, `edit_file`, or `delete_file` to make the change.\n\
         2. Do not stop at describing a change: when the user asks for a code change, perform it with the tools. {}\n\
         3. Always inspect relevant files before modifying them, and re-read after a failed edit rather than guessing.\n\
         4. Break complex work into clear steps and keep the live todo list current with `todo_write` (read it with `todo_read`).\n\
         5. For a non-trivial change, call `enter_plan_mode`, explore read-only, then propose the plan with `exit_plan_mode`; while planning, every write, delete, bash command, and workflow is blocked until the user approves the plan. Approving the plan still leaves each write behind its own diff card.\n\
         6. Make clean, accurate edits and verify your work (build, test, or read the result back).\n\
         7. Use `spawn_subagent` for a bounded independent task. Give each child a short `name`; it labels the child session and its title-bar pet. For fan-out, parallelize, or independent work, call `spawn_subagent` several times with `background: true` in one round, then collect with `wait`. Default policy is read-only; use full only for delegated edits. Children cannot spawn children. Full child writes/bash still require individual approval.\n\
         {}{}{}{}{}",
        root.display(),
        approval_note,
        depth_instructions,
        custom_sys,
        editor_note,
        attachments_note,
        rules_note
    )
}

/// The `AgentToolSettings` mapping (R5's seam): the shell's settings into
/// the router's slice, the Brave key through the `skip_serializing` field
/// (never serialized back out, never logged, §17.2).
fn agent_tool_settings(
    settings: &skarn_settings::AgentSettings,
    brave_api_key: Option<String>,
) -> skarn_ai::AgentToolSettings {
    skarn_ai::AgentToolSettings {
        shell: settings.shell.clone(),
        auto_approve_commands: settings.auto_approve_commands.clone(),
        brave_api_key,
        workspace_trusted: settings.workspace_trusted,
        local_only: settings.local_only,
    }
}

/// Look up (or create) the session, guard the busy rules, land the user's
/// message, and mark the request active — a refused send never mutates
/// the transcript, an accepted one is registered before the relay spawns.
fn begin_turn<'a>(
    inner: &'a mut EngineInner,
    request_id: &RequestId,
    session_id: &SessionId,
    user_text: &str,
    retry: Option<RetrySpec>,
) -> Result<&'a mut ChatSession, ProtocolErrorDto> {
    if inner.active.contains_key(request_id) {
        return Err(ProtocolErrorDto {
            code: "request_in_flight".into(),
            message: "This request id is already running.".into(),
            retryable: false,
        });
    }
    if let Some(session) = inner.sessions.get(session_id) {
        if session.active_request.is_some() {
            return Err(session_busy());
        }
    }
    inner.activity_counter += 1;
    let activity = inner.activity_counter;
    let now = skarn_ai::now_millis();
    let session = inner
        .sessions
        .entry(session_id.clone())
        .or_insert_with(|| ChatSession::new(title_from(user_text), now));
    session.last_activity = activity;
    session.active_request = Some(request_id.clone());
    session.updated_at = now;
    if inner.active_session.is_none() {
        inner.active_session = Some(session_id.clone());
    }
    let user_message_id = fresh_message_id();
    push_message(
        session,
        user_message_id,
        MessageRole::User,
        user_text.to_owned(),
        retry,
    );
    Ok(session)
}

/// Validate and apply a `RetryMessage` truncation under the engine lock:
/// remove the named user message and everything after it. Called by the send
/// paths after their key/model/workspace preflight, so a refused preflight
/// never mutates the transcript.
fn truncate_for_retry(
    inner: &mut EngineInner,
    target: &RetryTarget,
) -> Result<(), ProtocolErrorDto> {
    let session = inner
        .sessions
        .get_mut(&target.session_id)
        .ok_or_else(|| unknown_session(&target.session_id))?;
    if session.active_request.is_some() {
        return Err(session_busy());
    }
    let index = session
        .messages
        .iter()
        .position(|message| message.message_id == target.message_id)
        .ok_or_else(|| unknown_message(&target.message_id))?;
    if session.messages[index].role != MessageRole::User {
        return Err(not_a_user_message(&target.message_id));
    }
    session.messages.truncate(index);
    Ok(())
}

fn push_message(
    session: &mut ChatSession,
    message_id: MessageId,
    role: MessageRole,
    text: String,
    retry: Option<RetrySpec>,
) {
    session.messages.push(StoredMessage {
        message_id,
        role,
        text,
        retry,
        tool_calls: None,
        tool_call_id: None,
        ok: true,
        timestamp: skarn_ai::now_millis(),
    });
    // The transcript keeps the full history; the in-memory tail is what
    // the request needs. Never re-append a message that was already
    // written. Request construction bounds context independently of presentation.
}

/// The trailing transcript as request messages. Tool-call/tool-result
/// pairing is preserved for restored sessions (Z3's live-only behaviour
/// would lose the model's own tool context on resume).
fn conversation_tail(session: &ChatSession) -> Vec<ChatMessage> {
    session
        .messages
        .iter()
        .rev()
        .take(REQUEST_HISTORY_MESSAGES)
        .rev()
        .map(|message| {
            let mut chat = match message.role {
                MessageRole::User => ChatMessage::user(&message.text),
                MessageRole::Assistant => ChatMessage::assistant(&message.text),
                MessageRole::Tool => ChatMessage::tool_result(
                    message.tool_call_id.clone().unwrap_or_default(),
                    &message.text,
                ),
                MessageRole::System => ChatMessage::system(&message.text),
            };
            if message.role == MessageRole::Assistant {
                chat.tool_calls = message.tool_calls.clone();
            }
            chat
        })
        .collect()
}

fn title_from(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(32)
        .collect()
}

/// Persisted mode: `FullAccess` is session trust, not a durable mode.
fn persist_mode(mode: AgentModeDto) -> AgentModeDto {
    match mode {
        AgentModeDto::FullAccess => AgentModeDto::AskBeforeChanges,
        other => other,
    }
}

fn durable_mode(mode: AgentModeDto) -> AgentSessionMode {
    match persist_mode(mode) {
        AgentModeDto::ReadOnly => AgentSessionMode::Explore,
        AgentModeDto::AutoEdit => AgentSessionMode::AutoEdit,
        AgentModeDto::AskBeforeChanges | AgentModeDto::FullAccess => AgentSessionMode::Execute,
    }
}

fn mode_from_disk(mode: AgentSessionMode) -> AgentModeDto {
    match mode {
        AgentSessionMode::Explore => AgentModeDto::ReadOnly,
        AgentSessionMode::AutoEdit => AgentModeDto::AutoEdit,
        AgentSessionMode::Execute | AgentSessionMode::Plan => AgentModeDto::AskBeforeChanges,
    }
}

/// One in-memory message as a durable transcript record.
fn message_entry(message: &StoredMessage) -> TranscriptEntry {
    let id = message.message_id.as_str().to_owned();
    match message.role {
        MessageRole::User => TranscriptEntry::User {
            id,
            timestamp: message.timestamp,
            content: message.text.clone(),
        },
        MessageRole::Assistant | MessageRole::System => TranscriptEntry::Assistant {
            id,
            timestamp: message.timestamp,
            content: message.text.clone(),
            tool_calls: message.tool_calls.clone(),
        },
        MessageRole::Tool => TranscriptEntry::Tool {
            id,
            timestamp: message.timestamp,
            call_id: message.tool_call_id.clone().unwrap_or_default(),
            ok: message.ok,
            content: message.text.clone(),
        },
    }
}

/// A restored transcript message in the in-memory shape.
fn stored_message(message: &ChatMessage, timestamp: u64) -> StoredMessage {
    StoredMessage {
        retry: None,
        message_id: fresh_message_id(),
        role: match message.role {
            skarn_ai::Role::User => MessageRole::User,
            skarn_ai::Role::Assistant => MessageRole::Assistant,
            skarn_ai::Role::Tool => MessageRole::Tool,
            skarn_ai::Role::System => MessageRole::System,
        },
        text: message.content.clone(),
        tool_calls: message.tool_calls.clone(),
        tool_call_id: message.tool_call_id.clone(),
        ok: true,
        timestamp,
    }
}

/// Restore every durable record, including call batches and result pairing.
fn stored_transcript(entries: &[TranscriptEntry]) -> Vec<StoredMessage> {
    entries
        .iter()
        .map(|entry| {
            let (message, ok) = match entry {
                TranscriptEntry::User { content, .. } => (ChatMessage::user(content), true),
                TranscriptEntry::Assistant {
                    content,
                    tool_calls,
                    ..
                } => {
                    let mut message = ChatMessage::assistant(content);
                    message.tool_calls = tool_calls.clone();
                    (message, true)
                }
                TranscriptEntry::ToolCalls { calls, .. } => {
                    let mut message = ChatMessage::assistant("");
                    message.tool_calls = Some(calls.clone());
                    (message, true)
                }
                TranscriptEntry::Tool {
                    call_id,
                    content,
                    ok,
                    ..
                } => (ChatMessage::tool_result(call_id, content), *ok),
            };
            let mut stored = stored_message(&message, entry.timestamp());
            stored.message_id = MessageId::new(entry.id()).unwrap_or_else(|_| fresh_message_id());
            stored.ok = ok;
            stored
        })
        .collect()
}

/// The metadata half of a session write. The auto-approve flag is absent by
/// construction.
fn session_state(session_id: &SessionId, session: &ChatSession) -> AgentSessionState {
    let parks = match (&session.pending_approval, &session.pending_card) {
        (Some(approval), Some(card)) => vec![ParkedCard::Approval {
            approval_id: approval.approval_id.as_str().to_owned(),
            tool_name: session
                .pending_tool_name
                .clone()
                .unwrap_or_else(|| fallback_tool_name(card)),
            card: card.clone(),
            request_id: Some(approval.request_id.as_str().to_owned()),
            turn_id: Some(approval.turn_id.as_str().to_owned()),
            created_at: skarn_ai::now_millis(),
        }],
        _ => Vec::new(),
    };
    AgentSessionState {
        schema: skarn_ai::SESSION_SCHEMA,
        session_id: session_id.as_str().to_owned(),
        title: session.title.clone(),
        created_at: session.created_at,
        updated_at: session.updated_at,
        model: session.model.clone(),
        mode: durable_mode(session.mode),
        plan: plan_state_from_live(&skarn_ai::plan::snapshot(&session.plan_store)),
        parks,
        compaction: session.compaction,
    }
}

/// Parse a durable plan body back into a draft. Phase 9b writes the draft
/// as JSON; a record written by an earlier build holds free-form text and
/// loads as a single step rather than being dropped.
fn plan_draft_from_disk(raw: &str) -> Option<skarn_ai::PlanDraft> {
    if raw.trim().is_empty() {
        return None;
    }
    match serde_json::from_str::<skarn_ai::PlanDraft>(raw) {
        Ok(draft) if !draft.steps.is_empty() => Some(draft),
        _ => Some(skarn_ai::PlanDraft {
            summary: None,
            steps: vec![raw.to_owned()],
        }),
    }
}

/// The durable half of the live plan store: status, draft (as JSON, so the
/// steps survive a round trip) and timestamp.
fn plan_state_from_live(snapshot: &skarn_ai::PlanSnapshot) -> skarn_ai::PlanState {
    let plan = snapshot
        .plan
        .as_ref()
        .map(|draft| serde_json::to_string(draft).unwrap_or_else(|_| draft.steps.join("\n")));
    let status = match (&snapshot.plan, snapshot.phase) {
        (_, skarn_ai::PlanPhase::Planning) => skarn_ai::PlanStatus::Proposed,
        (None, skarn_ai::PlanPhase::Executing) => skarn_ai::PlanStatus::None,
        (Some(_), skarn_ai::PlanPhase::Executing) => skarn_ai::PlanStatus::Approved,
    };
    skarn_ai::PlanState {
        status,
        plan,
        updated_at: skarn_ai::now_millis(),
    }
}

fn session_persistence_error(error: &SkarnError) -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "session_persistence_failed".into(),
        message: format!("Could not read or write the agent session: {error}"),
        retryable: true,
    }
}

fn fresh_turn_id() -> TurnId {
    TurnId::new(Uuid::new_v4().to_string()).expect("uuid is valid")
}

fn fresh_message_id() -> MessageId {
    MessageId::new(Uuid::new_v4().to_string()).expect("uuid is valid")
}

pub(crate) fn model_dto(model: &Model) -> ModelDto {
    ModelDto {
        id: model.id.clone(),
        display_name: if model.display_name.is_empty() {
            model.id.clone()
        } else {
            model.display_name.clone()
        },
        available: model.is_available(),
        tool_capability: match model.tool_capability {
            ToolCapability::Supported => ToolCapabilityDto::Supported,
            ToolCapability::Unsupported => ToolCapabilityDto::Unsupported,
            ToolCapability::Unknown => ToolCapabilityDto::Unknown,
        },
        reasoning_capability: match model.reasoning_capability {
            skarn_ai::ReasoningCapability::Native => skarn_protocol::ReasoningCapabilityDto::Native,
            skarn_ai::ReasoningCapability::Unsupported => {
                skarn_protocol::ReasoningCapabilityDto::Unsupported
            }
            skarn_ai::ReasoningCapability::Unknown => {
                skarn_protocol::ReasoningCapabilityDto::Unknown
            }
        },
        reasoning_modes: model
            .reasoning_modes()
            .into_iter()
            .map(str::to_owned)
            .collect(),
    }
}

/// Preserve an existing explicit model selection across catalog refreshes and
/// reconnects (canonicalizing casing if matched). Only pick an initial default
/// when there is genuinely no prior selection, skipping unavailable/retired models.
pub(crate) fn reconcile_catalog_active_model(
    current: Option<String>,
    models: &[Model],
) -> Option<String> {
    if let Some(existing) = current
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        if let Some(matched) = models
            .iter()
            .find(|model| model.id == existing || model.id.eq_ignore_ascii_case(existing))
        {
            return Some(matched.id.clone());
        }
        return Some(existing.to_owned());
    }
    models
        .iter()
        .find(|model| model.is_available() && model.tool_capability == ToolCapability::Supported)
        .or_else(|| {
            models
                .iter()
                .find(|model| model.is_available() && model.tool_capability != ToolCapability::Unsupported)
        })
        .or_else(|| models.iter().find(|model| model.is_available()))
        .map(|model| model.id.clone())
}

fn compatible_model_hint(models: &[Model]) -> String {
    let mut suggested: Vec<&str> = models
        .iter()
        .filter(|model| model.is_available() && model.tool_capability == ToolCapability::Supported)
        .map(|model| model.id.as_str())
        .take(3)
        .collect();
    if suggested.is_empty() {
        "Refresh the model catalog and select a verified tool-capable model.".to_owned()
    } else {
        suggested.dedup();
        format!("Try a tool-capable model such as {}.", suggested.join(", "))
    }
}

fn model_unavailable_error(model_id: &str, catalog: &[Model], retired: bool) -> ProtocolErrorDto {
    let available_hint: Vec<&str> = catalog
        .iter()
        .filter(|model| model.is_available())
        .map(|model| model.id.as_str())
        .take(3)
        .collect();
    let suggestion = if available_hint.is_empty() {
        "Refresh the model catalog or choose an active model.".to_owned()
    } else {
        format!(
            "Refresh the model catalog or select an available model such as {}.",
            available_hint.join(", ")
        )
    };
    let detail = if retired {
        format!(
            "Model '{model_id}' is retired/end-of-life or unavailable and cannot be used. {suggestion}"
        )
    } else {
        format!(
            "Selected model '{model_id}' is no longer present in the provider's model catalog. {suggestion}"
        )
    };
    ProtocolErrorDto {
        code: "model_unavailable".into(),
        message: detail,
        retryable: false,
    }
}

fn run_blocking_probe<F, T>(future: F) -> Result<T, String>
where
    F: std::future::Future<Output = T> + Send,
    T: Send,
{
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("selora-capability-probe".into())
            .spawn_scoped(scope, || tauri::async_runtime::block_on(future))
            .map_err(|err| format!("Could not start capability validation worker: {err}"))?
            .join()
            .map_err(|_| "Capability validation worker panicked".to_owned())
    })
}

/// Gate an agent turn or workflow ask before sending a tool manifest:
/// - `Supported`: allowed.
/// - `Unsupported`: rejected locally (`model_capability_unsupported`) with no manifest sent.
/// - `Unknown`: runs a cached non-streaming `complete_chat` probe; fails closed
///   with `model_capability_unknown` / `model_capability_unsupported` if validation fails.
pub(crate) fn ensure_tool_capability_ready(
    cache: &ToolCapabilityCache,
    catalog_models: &[Model],
    base_url: &str,
    api_key: &str,
    model: &mut Model,
) -> Result<(), ProtocolErrorDto> {
    if !model.is_available() {
        return Err(model_unavailable_error(&model.id, catalog_models, true));
    }
    match model.tool_capability {
        ToolCapability::Supported => Ok(()),
        ToolCapability::Unsupported => {
            let hint = compatible_model_hint(catalog_models);
            Err(ProtocolErrorDto {
                code: "model_capability_unsupported".into(),
                message: format!(
                    "Model '{}' does not support tool calling and cannot run agent turns. {hint}",
                    model.id
                ),
                retryable: false,
            })
        }
        ToolCapability::Unknown => {
            if let Some(cached) = cache.get(base_url, &model.id) {
                match cached {
                    CachedCapabilityEntry::Supported => {
                        model.tool_capability = ToolCapability::Supported;
                        return Ok(());
                    }
                    CachedCapabilityEntry::Unsupported(reason) => {
                        return Err(ProtocolErrorDto {
                            code: "model_capability_unsupported".into(),
                            message: reason,
                            retryable: false,
                        });
                    }
                }
            }

            let client = skarn_ai::SkarnClient::new(base_url, api_key);
            let probe = run_blocking_probe(client.probe_tool_capability(&model.id)).map_err(|err| {
                ProtocolErrorDto {
                    code: "model_capability_unknown".into(),
                    message: format!(
                        "Could not verify tool-calling capability for model '{}': {err}",
                        model.id
                    ),
                    retryable: true,
                }
            })?;

            match probe {
                Ok(ToolProbeOutcome::Supported) => {
                    cache.insert(base_url, &model.id, CachedCapabilityEntry::Supported);
                    model.tool_capability = ToolCapability::Supported;
                    Ok(())
                }
                Ok(ToolProbeOutcome::Unsupported(reason)) => {
                    let message = format!(
                        "Model '{}' does not support tool calling: {reason} {}",
                        model.id,
                        compatible_model_hint(catalog_models)
                    );
                    cache.insert(
                        base_url,
                        &model.id,
                        CachedCapabilityEntry::Unsupported(message.clone()),
                    );
                    model.tool_capability = ToolCapability::Unsupported;
                    Err(ProtocolErrorDto {
                        code: "model_capability_unsupported".into(),
                        message,
                        retryable: false,
                    })
                }
                Ok(ToolProbeOutcome::Inconclusive(reason)) => {
                    let message = format!(
                        "Tool-calling capability for model '{}' could not be confirmed: {reason} {}",
                        model.id,
                        compatible_model_hint(catalog_models)
                    );
                    Err(ProtocolErrorDto {
                        code: "model_capability_unknown".into(),
                        message,
                        retryable: true,
                    })
                }
                Err(err) => {
                    if skarn_ai::classify_failure(&err) == skarn_ai::GatewayFailure::Auth {
                        return Err(to_protocol_error(&err, api_key));
                    }
                    let retryable = matches!(
                        skarn_ai::classify_failure(&err),
                        skarn_ai::GatewayFailure::Timeout
                            | skarn_ai::GatewayFailure::Network
                            | skarn_ai::GatewayFailure::Unavailable
                    );
                    Err(ProtocolErrorDto {
                        code: "model_capability_unknown".into(),
                        message: redact(
                            &format!(
                                "Could not verify tool-calling capability for model '{}': {}. {}",
                                model.id,
                                skarn_ai::user_message(&err),
                                compatible_model_hint(catalog_models)
                            ),
                            api_key,
                        ),
                        retryable,
                    })
                }
            }
        }
    }
}

pub(crate) fn discovery_error_message(base_url: &str, api_key: &str, error: &SkarnError) -> String {
    let provider = skarn_ai::safe_provider_label(base_url);
    let safe_err = redact(&skarn_ai::user_message(error), api_key);
    let detail = match skarn_ai::classify_failure(error) {
        skarn_ai::GatewayFailure::Auth => {
            let raw = redact(&error.to_string(), api_key);
            format!(
                "Model discovery failed at {provider}: credentials were rejected ({raw}). Check your API key and account access."
            )
        }
        skarn_ai::GatewayFailure::Timeout => format!(
            "Model discovery failed at {provider}: request timed out ({safe_err}). Credentials could not be verified."
        ),
        skarn_ai::GatewayFailure::Network => format!(
            "Model discovery failed at {provider}: could not reach the provider ({safe_err}). Credentials could not be verified."
        ),
        skarn_ai::GatewayFailure::Unavailable => format!(
            "Model discovery failed at {provider}: provider unavailable ({safe_err}). Credentials could not be verified."
        ),
        skarn_ai::GatewayFailure::ModelUnavailable
        | skarn_ai::GatewayFailure::ToolUnsupported
        | skarn_ai::GatewayFailure::Rejected
        | skarn_ai::GatewayFailure::InvalidResponse => format!(
            "Model discovery failed at {provider}: {safe_err}. Credentials could not be verified against the model catalog."
        ),
    };
    redact(&detail, api_key)
}

pub(crate) fn discovery_protocol_error(
    base_url: &str,
    api_key: &str,
    error: &SkarnError,
) -> ProtocolErrorDto {
    let retryable = matches!(
        skarn_ai::classify_failure(error),
        skarn_ai::GatewayFailure::Timeout
            | skarn_ai::GatewayFailure::Network
            | skarn_ai::GatewayFailure::Unavailable
    );
    ProtocolErrorDto {
        code: "model_discovery_failed".into(),
        message: discovery_error_message(base_url, api_key, error),
        retryable,
    }
}

pub(crate) fn empty_catalog_protocol_error(base_url: &str) -> ProtocolErrorDto {
    let provider = skarn_ai::safe_provider_label(base_url);
    ProtocolErrorDto {
        code: "model_discovery_failed".into(),
        message: format!(
            "Model discovery failed at {provider}: the provider returned an empty model catalog."
        ),
        retryable: false,
    }
}

fn manifest_for(mode: AgentModeDto) -> Vec<ToolSpec> {
    match mode {
        AgentModeDto::AskBeforeChanges | AgentModeDto::AutoEdit | AgentModeDto::FullAccess => {
            skarn_ai::tool_manifest()
        }
        AgentModeDto::ReadOnly => skarn_ai::tool_manifest()
            .into_iter()
            .filter(|spec| EXPLORE_TOOLS.contains(&spec.name.as_str()))
            .collect(),
    }
}

fn map_state(state: &AgentTurnState) -> (AgentPhase, Option<String>) {
    match state {
        AgentTurnState::Thinking => (AgentPhase::Thinking, None),
        AgentTurnState::RunningTool(name) => (AgentPhase::RunningTool, Some(name.clone())),
        AgentTurnState::AwaitingApproval(_) => (AgentPhase::AwaitingApproval, None),
        AgentTurnState::Done => (AgentPhase::Done, None),
    }
}

/// The `ApprovalNeeded` card body for one router card: the target line the
/// generic card's header shows, an optional diff, and — for `run_workflow` —
/// the §18.1 confirmation payload the WorkflowCard renders (script, confirmed
/// command set, cost cap). A workflow card's `path_or_command` is the workflow
/// *name*; the script rides `workflow.script`, never `diff_or_input` (it is not
/// a diff and must not be counted as one).
fn approval_body(card: &ApprovalCard) -> (String, Option<String>, Option<WorkflowApprovalDto>) {
    match card {
        ApprovalCard::Diff { path, diff } => (path.clone(), Some(diff.clone()), None),
        ApprovalCard::Delete { path } => (path.clone(), None, None),
        ApprovalCard::Command { command } => (command.clone(), None, None),
        ApprovalCard::Workflow {
            name,
            script,
            commands,
            max_cost_usd,
        } => (
            name.clone(),
            None,
            Some(WorkflowApprovalDto {
                name: name.clone(),
                script: script.clone(),
                commands: commands.clone(),
                max_cost_usd: *max_cost_usd,
            }),
        ),
        ApprovalCard::Plan { summary, steps } => (
            summary
                .clone()
                .unwrap_or_else(|| format!("plan with {} step(s)", steps.len())),
            None,
            None,
        ),
        ApprovalCard::Question(question_card) => (
            question_card
                .questions
                .first()
                .map(|question| question.question.clone())
                .unwrap_or_else(|| "question".to_owned()),
            None,
            None,
        ),
    }
}

/// The plan payload for the wire card (and the parked approval slot), when
/// the card is a plan.
fn approval_plan(card: &ApprovalCard) -> Option<PlanApprovalDto> {
    match card {
        ApprovalCard::Plan { summary, steps } => Some(PlanApprovalDto {
            summary: summary.clone(),
            steps: steps.clone(),
        }),
        _ => None,
    }
}

/// §19.4: one question card maps onto the single decision prompt the UI
/// renders (`prompt` on the event, `decision_prompt` on the parked approval).
/// The option id is the label: that is the value the answer echoes back.
fn approval_prompt(card: &ApprovalCard) -> Option<DecisionPromptDto> {
    let ApprovalCard::Question(question_card) = card else {
        return None;
    };
    let question = question_card.questions.first()?;
    Some(DecisionPromptDto::Question {
        header: None,
        question: question.question.clone(),
        options: question
            .options
            .iter()
            .map(|option| QuestionPromptOptionDto {
                id: option.label.clone(),
                label: option.label.clone(),
                description: (!option.description.is_empty()).then(|| option.description.clone()),
            })
            .collect(),
        allow_free_text: question.allow_free_text,
        free_text_placeholder: None,
    })
}

fn plan_phase_dto(phase: skarn_ai::PlanPhase) -> PlanPhaseDto {
    match phase {
        skarn_ai::PlanPhase::Planning => PlanPhaseDto::Planning,
        skarn_ai::PlanPhase::Executing => PlanPhaseDto::Executing,
    }
}

fn plan_approval_dto(plan: &skarn_ai::PlanDraft, api_key: &str) -> PlanApprovalDto {
    PlanApprovalDto {
        summary: plan
            .summary
            .as_deref()
            .map(|summary| redact(summary, api_key)),
        steps: plan
            .steps
            .iter()
            .map(|step| redact(step, api_key))
            .collect(),
    }
}

fn todo_dto(todo: &skarn_ai::TodoItem) -> TodoDto {
    TodoDto {
        id: todo.id.clone(),
        title: todo.title.clone(),
        status: match todo.status {
            skarn_ai::TodoStatus::Pending => TodoStatusDto::Pending,
            skarn_ai::TodoStatus::InProgress => TodoStatusDto::InProgress,
            skarn_ai::TodoStatus::Completed => TodoStatusDto::Completed,
        },
    }
}

fn fallback_tool_name(card: &ApprovalCard) -> String {
    match card {
        ApprovalCard::Diff { .. } => "write_file".into(),
        ApprovalCard::Delete { .. } => "delete_file".into(),
        ApprovalCard::Command { .. } => "bash".into(),
        ApprovalCard::Workflow { .. } => "run_workflow".into(),
        ApprovalCard::Plan { .. } => "exit_plan_mode".into(),
        ApprovalCard::Question(_) => "ask_user_question".into(),
    }
}

/// Map the runtime's completion onto the wire DTO (one shape for every
/// consumer; the frontend dedupes by `child_id`).
fn subagent_completed_dto(completion: skarn_ai::SubagentCompletion) -> SubagentCompletedDto {
    SubagentCompletedDto {
        child_id: completion.child_id,
        parent_session_id: SessionId::new(completion.parent_session)
            .expect("runtime parent sessions originate from a SessionId"),
        policy: subagent_policy_dto(completion.policy),
        ok: completion.ok,
        error: completion.error,
        final_message: completion.final_message,
        artifacts: completion
            .artifacts
            .into_iter()
            .map(|artifact| SubagentArtifactDto {
                kind: artifact.kind,
                path: artifact.path,
            })
            .collect(),
    }
}

fn subagent_policy_dto(policy: skarn_ai::SubagentPolicy) -> SubagentPolicyDto {
    match policy {
        skarn_ai::SubagentPolicy::ReadOnly => SubagentPolicyDto::ReadOnly,
        skarn_ai::SubagentPolicy::Full => SubagentPolicyDto::Full,
    }
}

/// Map one live runtime activity step onto the protocol. The API key is
/// scrubbed at this seam, like every other model-visible string.
fn subagent_activity_dto(
    activity: skarn_ai::SubagentActivity,
    keys: &[String],
) -> Option<SubagentActivityDto> {
    let parent = SessionId::new(activity.parent_session).ok()?;
    let child_id = activity.child_id;
    Some(match activity.kind {
        skarn_ai::SubagentActivityKind::Started {
            name,
            policy,
            mission,
        } => SubagentActivityDto::Started {
            child_id,
            parent_session_id: parent,
            name: Some(redact_with_keys(&name, keys)),
            policy: subagent_policy_dto(policy),
            mission: redact_with_keys(&mission, keys),
        },
        skarn_ai::SubagentActivityKind::Delta {
            message_id,
            sequence,
            text,
        } => SubagentActivityDto::Delta {
            child_id,
            parent_session_id: parent,
            message_id: redact_with_keys(&message_id, keys),
            sequence,
            text: redact_with_keys(&text, keys),
        },
        skarn_ai::SubagentActivityKind::ToolCallStarted {
            call_id,
            tool_name,
            args_json,
        } => SubagentActivityDto::ToolCallStarted {
            child_id,
            parent_session_id: parent,
            call_id: redact_with_keys(&call_id, keys),
            tool_name: redact_with_keys(&tool_name, keys),
            args_json: redact_json_text(&args_json, keys),
        },
        skarn_ai::SubagentActivityKind::ToolResult {
            call_id,
            result_summary,
            is_err,
        } => SubagentActivityDto::ToolResult {
            child_id,
            parent_session_id: parent,
            call_id: redact_with_keys(&call_id, keys),
            result_summary: redact_with_keys(&result_summary, keys),
            is_err,
        },
        skarn_ai::SubagentActivityKind::StateChanged { state, tool_name } => {
            SubagentActivityDto::StateChanged {
                child_id,
                parent_session_id: parent,
                state: match state {
                    skarn_ai::SubagentRunState::Thinking => SubagentRunState::Thinking,
                    skarn_ai::SubagentRunState::RunningTool => SubagentRunState::RunningTool,
                    skarn_ai::SubagentRunState::AwaitingApproval => {
                        SubagentRunState::AwaitingApproval
                    }
                    skarn_ai::SubagentRunState::Done => SubagentRunState::Done,
                },
                tool_name: tool_name.map(|name| redact_with_keys(&name, keys)),
            }
        }
    })
}

fn map_decision(decision: DecisionDto, answer: Option<String>) -> skarn_ai::AgentDecision {
    match decision {
        DecisionDto::Answer => skarn_ai::AgentDecision::Answer(answer.unwrap_or_default()),
        DecisionDto::Approve => skarn_ai::AgentDecision::Approve,
        DecisionDto::Deny => skarn_ai::AgentDecision::Deny,
        DecisionDto::ApproveAllThisTurn => skarn_ai::AgentDecision::ApproveAllThisTurn,
        DecisionDto::Revise => answer
            .map(skarn_ai::AgentDecision::Answer)
            .unwrap_or(skarn_ai::AgentDecision::Revise),
        DecisionDto::Stop => skarn_ai::AgentDecision::Stop,
    }
}

fn permission_mode_from_dto(mode: AgentModeDto) -> PermissionMode {
    match mode {
        AgentModeDto::ReadOnly => PermissionMode::ReadOnly,
        AgentModeDto::AskBeforeChanges => PermissionMode::AskBeforeChanges,
        AgentModeDto::AutoEdit => PermissionMode::AutoEdit,
        AgentModeDto::FullAccess => PermissionMode::FullAccess,
    }
}

fn pending_card_classification(card: &ApprovalCard, trusted: bool) -> (ActionClassDto, RiskLevelDto) {
    match card {
        ApprovalCard::Diff { .. } => (ActionClassDto::FileEdit, RiskLevelDto::Medium),
        ApprovalCard::Delete { .. } => (ActionClassDto::FileDelete, RiskLevelDto::High),
        ApprovalCard::Command { .. } => (
            ActionClassDto::CommandGated,
            if trusted {
                RiskLevelDto::High
            } else {
                RiskLevelDto::Critical
            },
        ),
        ApprovalCard::Workflow { .. } => (ActionClassDto::Workflow, RiskLevelDto::High),
        ApprovalCard::Plan { .. } => (ActionClassDto::PlanControl, RiskLevelDto::High),
        ApprovalCard::Question(_) => (ActionClassDto::Question, RiskLevelDto::Low),
    }
}

fn risk_dto(risk: RiskLevel) -> RiskLevelDto {
    match risk {
        RiskLevel::Low => RiskLevelDto::Low,
        RiskLevel::Medium => RiskLevelDto::Medium,
        RiskLevel::High => RiskLevelDto::High,
        RiskLevel::Critical => RiskLevelDto::Critical,
    }
}

fn action_class_dto(class: ActionClass) -> ActionClassDto {
    match class {
        ActionClass::ReadInspect => ActionClassDto::ReadInspect,
        ActionClass::FileEdit => ActionClassDto::FileEdit,
        ActionClass::FileDelete => ActionClassDto::FileDelete,
        ActionClass::CommandAllowlisted => ActionClassDto::CommandAllowlisted,
        ActionClass::CommandGated => ActionClassDto::CommandGated,
        ActionClass::Workflow => ActionClassDto::Workflow,
        ActionClass::PlanControl => ActionClassDto::PlanControl,
        ActionClass::Question => ActionClassDto::Question,
    }
}

fn action_class_from_dto(class: ActionClassDto) -> ActionClass {
    match class {
        ActionClassDto::ReadInspect => ActionClass::ReadInspect,
        ActionClassDto::FileEdit => ActionClass::FileEdit,
        ActionClassDto::FileDelete => ActionClass::FileDelete,
        ActionClassDto::CommandAllowlisted => ActionClass::CommandAllowlisted,
        ActionClassDto::CommandGated => ActionClass::CommandGated,
        ActionClassDto::Workflow => ActionClass::Workflow,
        ActionClassDto::PlanControl => ActionClass::PlanControl,
        ActionClassDto::Question => ActionClass::Question,
    }
}

fn grant_decision_dto(decision: GrantDecision) -> GrantDecisionDto {
    match decision {
        GrantDecision::AllowOnce => GrantDecisionDto::AllowOnce,
        GrantDecision::AllowSession => GrantDecisionDto::AllowSession,
        GrantDecision::AllowProject => GrantDecisionDto::AllowProject,
        GrantDecision::DenyOnce => GrantDecisionDto::DenyOnce,
        GrantDecision::DenySession => GrantDecisionDto::DenySession,
        GrantDecision::DenyProject => GrantDecisionDto::DenyProject,
    }
}

fn grant_decision_from_dto(decision: GrantDecisionDto) -> GrantDecision {
    match decision {
        GrantDecisionDto::AllowOnce => GrantDecision::AllowOnce,
        GrantDecisionDto::AllowSession => GrantDecision::AllowSession,
        GrantDecisionDto::AllowProject => GrantDecision::AllowProject,
        GrantDecisionDto::DenyOnce => GrantDecision::DenyOnce,
        GrantDecisionDto::DenySession => GrantDecision::DenySession,
        GrantDecisionDto::DenyProject => GrantDecision::DenyProject,
    }
}

fn scoped_grant_dto(entry: &ScopedGrantEntry) -> ScopedGrantDto {
    ScopedGrantDto {
        action_class: action_class_dto(entry.action_class),
        target_pattern: entry.target_pattern.clone(),
        decision: grant_decision_dto(entry.decision),
        max_risk: risk_dto(entry.max_risk),
    }
}

fn execution_tier_dto(tier: ExecutionTier) -> ExecutionTierDto {
    match tier {
        ExecutionTier::Tier1Deterministic => ExecutionTierDto::Tier1Deterministic,
        ExecutionTier::Tier2DecisionModel => ExecutionTierDto::Tier2DecisionModel,
        ExecutionTier::Tier3GeneralLlm => ExecutionTierDto::Tier3GeneralLlm,
    }
}

fn decision_phase_dto(phase: DecisionPhase) -> DecisionPhaseDto {
    match phase {
        DecisionPhase::Snapshotted => DecisionPhaseDto::Snapshotted,
        DecisionPhase::Routed => DecisionPhaseDto::Routed,
        DecisionPhase::Inferred => DecisionPhaseDto::Inferred,
        DecisionPhase::SchemaValidated => DecisionPhaseDto::SchemaValidated,
        DecisionPhase::PolicyEvaluated => DecisionPhaseDto::PolicyEvaluated,
        DecisionPhase::ApprovalPending => DecisionPhaseDto::ApprovalPending,
        DecisionPhase::SnapshotRechecked => DecisionPhaseDto::SnapshotRechecked,
        DecisionPhase::Executed => DecisionPhaseDto::Executed,
        DecisionPhase::Verified => DecisionPhaseDto::Verified,
        DecisionPhase::Stale => DecisionPhaseDto::Stale,
        DecisionPhase::Escalated => DecisionPhaseDto::Escalated,
        DecisionPhase::Rejected => DecisionPhaseDto::Rejected,
        DecisionPhase::Cancelled => DecisionPhaseDto::Cancelled,
        DecisionPhase::Failed => DecisionPhaseDto::Failed,
    }
}

fn decision_trace_dto(
    request_id: &RequestId,
    session_id: &SessionId,
    turn_id: &TurnId,
    trace: &skarn_ai::ToolDecisionTrace,
    keys: &[String],
) -> DecisionTraceDto {
    let expl = &trace.explanation;
    DecisionTraceDto {
        request_id: request_id.clone(),
        session_id: session_id.clone(),
        turn_id: turn_id.clone(),
        call_id: redact_with_keys(&trace.call_id, keys),
        trace_id: redact_with_keys(&expl.trace_id, keys),
        action_id: redact_with_keys(&trace.action_id, keys),
        risk: risk_dto(trace.risk),
        phase: decision_phase_dto(trace.phase),
        selected_tier: execution_tier_dto(expl.selected_tier),
        candidate_tiers: expl
            .candidate_tiers
            .iter()
            .copied()
            .map(execution_tier_dto)
            .collect(),
        provider_id: expl
            .provider_id
            .as_deref()
            .map(|p| redact_with_keys(p, keys)),
        model_id: expl
            .model_id
            .as_deref()
            .map(|m| redact_with_keys(m, keys)),
        hop_count: expl.hop_count,
        privacy_blocked_cloud: expl.privacy_blocked_cloud,
        reason_code: redact_with_keys(&expl.reason_code, keys),
        calibrated_threshold: expl.calibrated_threshold,
        raw_confidence: expl.raw_confidence,
        fsm_path: expl
            .fsm_path
            .iter()
            .copied()
            .map(decision_phase_dto)
            .collect(),
    }
}

fn rule_source_dto(source: &skarn_decision::RuleSource) -> RuleSourceDto {
    RuleSourceDto {
        path: source.path.clone(),
        scope: match source.scope {
            RuleScope::Global => RuleScopeDto::Global,
            RuleScope::Workspace => RuleScopeDto::Workspace,
        },
        active: source.active,
        inactive_reason: source.reason.clone(),
        digest: source.digest.clone(),
    }
}

fn checkpoint_summary_dto(
    session_id: &SessionId,
    manifest: &TurnCheckpointManifest,
    undone: bool,
    conflicts: Vec<UndoConflictDto>,
) -> TurnCheckpointSummaryDto {
    TurnCheckpointSummaryDto {
        session_id: session_id.clone(),
        turn_id: TurnId::new(manifest.turn_id.clone()).unwrap_or_else(|_| {
            TurnId::new("turn-unknown").expect("fallback turn id is valid")
        }),
        created_at_ms: manifest.created_at_ms,
        files: manifest
            .files
            .iter()
            .map(|f| TurnCheckpointFileDto {
                path: f.rel_path.clone(),
                existed_before: f.existed_before,
                existed_after: f.existed_after,
            })
            .collect(),
        undone,
        conflicts,
    }
}

fn status_changed(status: ConnectionStatus, active_model: &Option<String>) -> FrontendEvent {
    status_changed_with_error(status, active_model, None)
}

pub(crate) fn status_changed_with_error(
    status: ConnectionStatus,
    active_model: &Option<String>,
    error: Option<String>,
) -> FrontendEvent {
    FrontendEvent::SkarnStatusChanged(SkarnStatusDto {
        status,
        active_model_id: active_model.clone(),
        error,
    })
}

/// A Selora error into the protocol's typed error, with the key redacted
/// from the message (the client scrubs its own error bodies; this is the
/// second seam, for anything that slipped through).
fn to_protocol_error(error: &SkarnError, api_key: &str) -> ProtocolErrorDto {
    // The failure class comes from the client's typed view (timeout, auth,
    // network, gateway down, rejected, unparseable). Codes stay stable for
    // the frontend; the message is actionable copy, never a raw transport
    // dump and never the key.
    let (code, retryable) = match skarn_ai::classify_failure(error) {
        skarn_ai::GatewayFailure::Timeout => ("timeout", true),
        skarn_ai::GatewayFailure::Auth => ("auth_failed", false),
        skarn_ai::GatewayFailure::Network => ("network_error", true),
        skarn_ai::GatewayFailure::Unavailable => ("gateway_unavailable", true),
        skarn_ai::GatewayFailure::ModelUnavailable => ("model_unavailable", false),
        skarn_ai::GatewayFailure::ToolUnsupported => ("model_capability_unsupported", false),
        skarn_ai::GatewayFailure::Rejected => ("gateway_error", false),
        skarn_ai::GatewayFailure::InvalidResponse => match error {
            SkarnError::Auth(_) => ("auth_failed", false),
            SkarnError::Net(_) => ("network_error", true),
            SkarnError::Ai(_) => ("gateway_error", true),
            _ => ("chat_failed", false),
        },
    };
    ProtocolErrorDto {
        code: code.into(),
        message: redact(&skarn_ai::user_message(error), api_key),
        retryable,
    }
}

fn redact(text: &str, api_key: &str) -> String {
    if api_key.is_empty() {
        text.to_owned()
    } else {
        text.replace(api_key, "[REDACTED]")
    }
}

fn redact_json_text(text: &str, keys: &[String]) -> String {
    fn walk(value: serde_json::Value, keys: &[String]) -> serde_json::Value {
        match value {
            serde_json::Value::String(text) => {
                serde_json::Value::String(redact_with_keys(&text, keys))
            }
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.into_iter().map(|item| walk(item, keys)).collect())
            }
            serde_json::Value::Object(items) => serde_json::Value::Object(
                items
                    .into_iter()
                    .map(|(name, item)| (redact_with_keys(&name, keys), walk(item, keys)))
                    .collect(),
            ),
            other => other,
        }
    }
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(value) => serde_json::to_string(&walk(value, keys)).expect("JSON serializes"),
        Err(_) => redact_with_keys(text, keys),
    }
}

fn scrub_serializable<T: serde::Serialize + serde::de::DeserializeOwned>(
    value: T,
    keys: &[String],
) -> T {
    fn walk(value: &mut serde_json::Value, keys: &[String]) {
        match value {
            serde_json::Value::String(text) => *text = redact_with_keys(text, keys),
            serde_json::Value::Array(items) => items.iter_mut().for_each(|item| walk(item, keys)),
            serde_json::Value::Object(items) => {
                for (name, item) in items {
                    if !name.ends_with("_id")
                        && !matches!(name.as_str(), "status" | "kind" | "type" | "role" | "phase")
                    {
                        walk(item, keys);
                    }
                }
            }
            _ => {}
        }
    }
    let mut value = serde_json::to_value(value).expect("DTO serializes");
    walk(&mut value, keys);
    serde_json::from_value(value).expect("redaction preserves DTO schema")
}

/// Apply the same approval redaction to plan summaries and every plan step.
#[cfg(test)]
fn redact_card(card: ApprovalCard, api_key: &str) -> ApprovalCard {
    redact_card_with_keys(card, &[api_key.to_owned()])
}

fn redact_card_with_keys(card: ApprovalCard, keys: &[String]) -> ApprovalCard {
    fn scrub(value: &mut serde_json::Value, keys: &[String]) {
        match value {
            serde_json::Value::String(text) => *text = redact_with_keys(text, keys),
            serde_json::Value::Array(values) => {
                values.iter_mut().for_each(|value| scrub(value, keys))
            }
            serde_json::Value::Object(values) => {
                values.values_mut().for_each(|value| scrub(value, keys))
            }
            _ => {}
        }
    }
    let mut value = serde_json::to_value(card).expect("approval card serializes");
    scrub(&mut value, keys);
    serde_json::from_value(value).expect("redaction preserves approval schema")
}

fn stale_session() -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "stale_workspace_or_session".into(),
        message: "Request a fresh bootstrap snapshot.".into(),
        retryable: true,
    }
}

fn session_busy() -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "session_busy".into(),
        message: "A request is already running for this session — stop it first.".into(),
        retryable: false,
    }
}

fn unknown_session(session_id: &SessionId) -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "unknown_session".into(),
        message: format!(
            "No session `{}` exists; request a fresh bootstrap.",
            session_id.as_str()
        ),
        retryable: true,
    }
}

fn unknown_message(message_id: &MessageId) -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "unknown_message".into(),
        message: format!(
            "No message `{}` exists in this session.",
            message_id.as_str()
        ),
        retryable: false,
    }
}

fn not_a_user_message(message_id: &MessageId) -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "not_a_user_message".into(),
        message: format!(
            "Message `{}` is not a user turn; retry needs the user message.",
            message_id.as_str()
        ),
        retryable: false,
    }
}

fn no_model_selected() -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "no_model_selected".into(),
        message: "No model selected — pass model_id or set an active model.".into(),
        retryable: false,
    }
}

fn no_workspace() -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "no_workspace".into(),
        message: "Agent mode needs a workspace root — open a folder so the agent can \
                  read and write files."
            .into(),
        retryable: false,
    }
}

// ---------------------------------------------------------------------------
// Tests — the Z3 vertical slice, hermetic: a loopback wiremock gateway (the
// selora-ai test pattern), a fixed engine key (the real Credential Manager
// is never touched), and tempdir workspaces. The Tauri-side seams run
// through `tauri::test::mock_app` (the Z2 pattern), the relays through
// `tauri::async_runtime`'s global runtime.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::DesktopState;
    use skarn_protocol::{
        AgentRequestDto, ChatRequestDto, DecisionDto, EditorContextDto, EventEnvelope,
        FrontendCommand, RequestEnvelope, Selection, TextPosition, WorkspaceId, PROTOCOL_VERSION,
    };
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const TEST_TIMEOUT: Duration = Duration::from_secs(10);

    // ------------------------------------------------------------------
    // Fixtures and helpers
    // ------------------------------------------------------------------

    fn mock_app_handle() -> tauri::AppHandle<tauri::test::MockRuntime> {
        tauri::test::mock_app().handle().clone()
    }

    fn gateway_settings(base_url: String) -> skarn_settings::Settings {
        let mut settings = skarn_settings::Settings::default();
        settings.skarn.base_url = base_url;
        settings
    }

    fn seed_supported_test_model(state: &DesktopState) {
        state.sessions().set_models_and_connection(
            vec![Model {
                id: "test-model".into(),
                display_name: "Test Model".into(),
                context_window: 128_000,
                price_per_1k_input: 0.0,
                price_per_1k_output: 0.0,
                supports_streaming: true,
                tool_capability: ToolCapability::Supported,
                reasoning_capability: skarn_ai::ReasoningCapability::Unknown,
                available: true,
                family: "test".into(),
            }],
            ConnectionStatus::Connected,
            Some("test-model".into()),
        );
    }

    fn envelope(
        state: &DesktopState,
        command: FrontendCommand,
    ) -> RequestEnvelope<FrontendCommand> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);
        let request_id = format!("transport-{}", NEXT_REQUEST.fetch_add(1, Ordering::Relaxed));
        RequestEnvelope {
            protocol_version: PROTOCOL_VERSION,
            workspace_id: Some(WorkspaceId::new("no-workspace").expect("valid id")),
            core_instance_id: Some(state.core_instance_id()),
            request_id: RequestId::new(request_id).expect("valid id"),
            session_id: None,
            payload: command,
        }
    }

    fn send_chat(request_id: &str, session: &str, message: &str) -> FrontendCommand {
        FrontendCommand::SendChat {
            request_id: RequestId::new(request_id).expect("valid id"),
            request: ChatRequestDto {
                session_id: SessionId::new(session).expect("valid id"),
                message: message.into(),
                model_id: Some("test-model".into()),
            },
        }
    }

    fn send_agent(request_id: &str, session: &str, prompt: &str) -> FrontendCommand {
        FrontendCommand::SendAgent {
            request_id: RequestId::new(request_id).expect("valid id"),
            request: AgentRequestDto {
                session_id: SessionId::new(session).expect("valid id"),
                prompt: prompt.into(),
                model_id: Some("test-model".into()),
                mode: AgentModeDto::AskBeforeChanges,
                reasoning_effort: None,
                system_instructions: None,
                temperature: None,
                editor_context: None,
                attachments: Vec::new(),
            },
        }
    }

    fn agent_decision(
        request_id: &str,
        approval_id: &str,
        decision: DecisionDto,
    ) -> FrontendCommand {
        FrontendCommand::AgentDecision {
            resolution: None,
            request_id: RequestId::new(request_id).expect("valid id"),
            approval_id: ApprovalId::new(approval_id).expect("valid id"),
            decision,
        }
    }

    fn stop_chat(request_id: &str) -> FrontendCommand {
        FrontendCommand::StopChat {
            request_id: RequestId::new(request_id).expect("valid id"),
        }
    }

    /// A tempdir workspace with a seeded NOTES.md the tools are rooted at.
    fn temp_workspace(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("selora-z3-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("workspace dir");
        std::fs::write(root.join("NOTES.md"), "notes-body-xyz\n").expect("seed file");
        root
    }

    /// The SSE chunk builders (the client's own test fixtures' shapes).
    fn content_chunk(text: &str) -> serde_json::Value {
        serde_json::json!({ "choices": [ { "delta": { "content": text } } ] })
    }
    fn fragment_chunk(index: usize, id: &str, name: &str, arguments: &str) -> serde_json::Value {
        serde_json::json!({
            "choices": [ { "delta": { "tool_calls": [ {
                "index": index, "id": id, "type": "function",
                "function": { "name": name, "arguments": arguments }
            } ] } } ]
        })
    }
    fn usage_chunk(reason: &str, prompt: u32, completion: u32, cost: f64) -> serde_json::Value {
        serde_json::json!({
            "choices": [ { "delta": {}, "finish_reason": reason } ],
            "usage": { "prompt_tokens": prompt, "completion_tokens": completion, "cost_usd": cost }
        })
    }
    fn sse(chunks: &[serde_json::Value]) -> String {
        let mut body = String::new();
        for chunk in chunks {
            body.push_str("data: ");
            body.push_str(&chunk.to_string());
            body.push_str("\n\n");
        }
        body.push_str("data: [DONE]\n\n");
        body
    }

    /// One streaming round that calls tools: a content marker (the mock
    /// gateway's body matcher), one fragment per call, then `tool_calls`.
    fn tool_round_sse(calls: &[(&str, &str, &str)], marker: &str, cost: f64) -> String {
        let mut chunks = vec![content_chunk(marker)];
        for (index, (id, name, arguments)) in calls.iter().enumerate() {
            chunks.push(fragment_chunk(index, id, name, arguments));
        }
        chunks.push(usage_chunk("tool_calls", 100, 50, cost));
        sse(&chunks)
    }

    /// One streaming round that is the final answer.
    fn answer_round_sse(text: &str, cost: f64) -> String {
        sse(&[content_chunk(text), usage_chunk("stop", 100, 50, cost)])
    }

    /// Wait until a parked approval differs from `previous` (a decision
    /// unparks the turn, which then parks again on the next card).
    fn wait_for_new_approval(
        state: &DesktopState,
        previous: Option<&str>,
    ) -> skarn_protocol::ApprovalDto {
        let deadline = Instant::now() + TEST_TIMEOUT;
        loop {
            if let Some(pending) = state.bootstrap().chat.pending_approval {
                if previous != Some(pending.approval_id.as_str()) {
                    return pending;
                }
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for a new parked approval"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// A mock gateway: one SSE round per `(body marker, response)`, mounted
    /// in the given order (wiremock matches in mount order — mount the most
    /// specific marker first). `expect_hits` arms a hit-count verification
    /// per round (wiremock verifies on drop) — false for turns that are
    /// stopped before their script plays out.
    fn mock_gateway(rounds: Vec<(&str, String)>, expect_hits: bool) -> MockServer {
        tauri::async_runtime::block_on(async {
            let server = MockServer::start().await;
            for (marker, body) in rounds {
                let mut mock = Mock::given(method("POST"))
                    .and(path("/v1/chat/completions"))
                    .and(body_string_contains(marker.to_owned()))
                    .respond_with(ResponseTemplate::new(200).set_body_string(body));
                if expect_hits {
                    mock = mock.expect(1);
                }
                mock.mount(&server).await;
            }
            server
        })
    }

    /// Wait until the mock gateway has received at least one request — the
    /// stop tests must not race their cancellation against the relay's
    /// first request (the egui tests' synchronization rule).
    fn sync_with_server(server: &MockServer) {
        let deadline = Instant::now() + TEST_TIMEOUT;
        while tauri::async_runtime::block_on(async { server.received_requests().await })
            .is_none_or(|requests| requests.is_empty())
        {
            assert!(
                Instant::now() < deadline,
                "the request never reached the mock gateway"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Poll the recorded event log until an event matching the predicate
    /// lands (the relays run on tauri::async_runtime's workers; the poll
    /// never blocks the runtime).
    fn wait_for(
        state: &DesktopState,
        predicate: impl Fn(&EventEnvelope<FrontendEvent>) -> bool,
    ) -> Vec<EventEnvelope<FrontendEvent>> {
        let deadline = Instant::now() + TEST_TIMEOUT;
        loop {
            let events = state.events_after(0);
            if events.iter().any(&predicate) {
                return events;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for the event; recorded so far:\n{events:#?}"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// A compact event tag for order assertions.
    fn tag(event: &EventEnvelope<FrontendEvent>) -> String {
        match &event.payload {
            FrontendEvent::AgentStateChanged(dto) => match dto.state {
                AgentPhase::Thinking => "state:thinking".into(),
                AgentPhase::RunningTool => {
                    format!("state:running:{}", dto.tool_name.as_deref().unwrap_or("?"))
                }
                AgentPhase::AwaitingApproval => "state:awaiting".into(),
                AgentPhase::Done => "state:done".into(),
            },
            FrontendEvent::ChatDelta(dto) => format!("delta:{}", dto.text),
            FrontendEvent::ToolCallStarted(dto) => format!("tool_start:{}", dto.tool_name),
            FrontendEvent::ToolResult(dto) => format!("tool_result:{}", dto.call_id),
            FrontendEvent::ApprovalNeeded(dto) => format!("approval:{}", dto.tool_name),
            FrontendEvent::AgentTurnDone(_) => "turn_done".into(),
            FrontendEvent::ChatCompleted(_) => "completed".into(),
            FrontendEvent::ChatFailed(dto) => format!("failed:{}", dto.error.code),
            other => format!("other:{other:?}"),
        }
    }

    fn find_payload<T>(
        events: &[EventEnvelope<FrontendEvent>],
        predicate: impl Fn(&FrontendEvent) -> Option<T> + Copy,
    ) -> T {
        events
            .iter()
            .filter_map(|event| predicate(&event.payload))
            .next_back()
            .expect("the event is in the log")
    }

    /// A raw TCP "gateway" that streams one delta, then stalls — wiremock
    /// cannot hold a body open mid-stream. The thread parks on the socket
    /// until the client cancels and drops it.
    fn stalling_gateway(first_delta: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the stalling gateway");
        let uri = format!("http://{}", listener.local_addr().expect("addr"));
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\n\r\ndata: {}\n\n",
            content_chunk(first_delta)
        );
        std::thread::spawn(move || {
            if let Ok((mut socket, _)) = listener.accept() {
                // Drain the request first: closing a Windows socket with
                // unread data sends RST mid-request and the client reports
                // a send failure instead of a stream.
                let mut sink = [0u8; 8192];
                let _ = socket.read(&mut sink);
                let _ = socket.write_all(response.as_bytes());
                let _ = socket.flush();
                // The stall: hold the connection open (this read parks)
                // until the client cancels and drops it.
                let _ = socket.read(&mut sink);
            }
        });
        uri
    }

    // ------------------------------------------------------------------
    // Typed refusals (no key / no workspace / stale envelopes)
    // ------------------------------------------------------------------

    #[test]
    fn send_chat_without_a_key_fails_not_connected() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings_no_key(skarn_settings::Settings::default());
        let error = state
            .dispatch(
                &app,
                envelope(&state, send_chat("chat-1", "session-1", "hi")),
            )
            .expect_err("no key is a typed refusal");
        assert_eq!(error.code, "not_connected");
        assert!(error.retryable);
        assert!(state.events_after(0).is_empty(), "refusals record nothing");
    }

    #[test]
    fn send_agent_without_a_workspace_fails_no_workspace() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(skarn_settings::Settings::default());
        let error = state
            .dispatch(
                &app,
                envelope(&state, send_agent("agent-1", "session-1", "fix it")),
            )
            .expect_err("no root is a typed refusal");
        assert_eq!(error.code, "no_workspace");
        assert!(!error.retryable);
        assert!(state.events_after(0).is_empty());
    }

    #[test]
    fn session_commands_reject_stale_envelopes() {
        let app = mock_app_handle();
        for command in [
            send_chat("chat-1", "session-1", "hi"),
            stop_chat("chat-1"),
            send_agent("agent-1", "session-1", "fix it"),
            agent_decision("agent-1", "a-valid-id", DecisionDto::Approve),
        ] {
            let state = DesktopState::with_settings(skarn_settings::Settings::default());
            let base = envelope(&state, command.clone());

            // (1) protocol version mismatch.
            let mut bad = base.clone();
            bad.protocol_version = PROTOCOL_VERSION + 1;
            assert_eq!(
                state
                    .dispatch(&app, bad)
                    .expect_err("version mismatch")
                    .code,
                "protocol_version_mismatch"
            );

            // (2) stale core instance.
            let mut bad = base.clone();
            bad.core_instance_id =
                Some(skarn_protocol::CoreInstanceId::new("other-core").expect("valid id"));
            assert_eq!(
                state.dispatch(&app, bad).expect_err("stale core").code,
                "stale_core_instance"
            );

            // (3) a foreign workspace id.
            let mut bad = base.clone();
            bad.workspace_id = Some(WorkspaceId::new("a-real-workspace").expect("valid id"));
            assert_eq!(
                state.dispatch(&app, bad).expect_err("stale workspace").code,
                "stale_workspace_or_session"
            );

            // (4) an envelope session that disagrees with the payload's.
            // (Stop/AgentDecision have no payload session: they are refused
            // by the engine instead — the request never existed.)
            let mut bad = base;
            bad.session_id = Some(SessionId::new("another-session").expect("valid id"));
            if matches!(
                command,
                FrontendCommand::SendChat { .. } | FrontendCommand::SendAgent { .. }
            ) {
                assert_eq!(
                    state
                        .dispatch(&app, bad)
                        .expect_err("session mismatch")
                        .code,
                    "stale_workspace_or_session"
                );
            }
            assert!(state.events_after(0).is_empty());
        }
    }

    #[test]
    fn session_commands_accept_their_envelope_session() {
        // A matching envelope session id passes the guards (the engine's
        // own payload comparison is the next line of defense) — proven by
        // reaching the not_connected refusal instead of a stale envelope.
        let app = mock_app_handle();
        let state = DesktopState::with_settings_no_key(skarn_settings::Settings::default());
        let mut accepted = envelope(&state, send_chat("chat-1", "session-1", "hi"));
        accepted.session_id = Some(SessionId::new("session-1").expect("valid id"));
        assert_eq!(
            state
                .dispatch(&app, accepted)
                .expect_err("past the envelope, refused at the key")
                .code,
            "not_connected"
        );
    }

    // ------------------------------------------------------------------
    // SendChat: stream, complete, fail, stop, busy
    // ------------------------------------------------------------------

    #[test]
    fn agent_relay_preserves_prefix_across_intervening_events() {
        for (first, second, expected) in [("sec", "ond", "second"), ("sec", "ret", "[REDACTED]")] {
            let state = DesktopState::with_settings(skarn_settings::Settings::default());
            let log = Arc::new(EventLog::new(state.core_instance_id()));
            let engine = Arc::new(SessionEngine::new(EngineConfig {
                base_url: "http://127.0.0.1:1".into(),
                active_model: None,
                agent: skarn_settings::AgentSettings::default(),
                key_source: ApiKeySource::Fixed(Some("secret".into())),
            }));
            let report = skarn_ai::AgentTurnReport {
                summary: AgentSummary {
                    iterations: 0,
                    tools_used: 0,
                    approved: 0,
                    denied: 0,
                    tokens: 0,
                    cost_usd: 0.0,
                    completed: true,
                },
                end: skarn_ai::TurnEnd::Completed,
                final_message: format!("{first}{second}"),
                conversation: Vec::new(),
            };
            let (tx, rx) = mpsc::unbounded_channel();
            tx.send(skarn_ai::AgentEvent::Delta(first.into())).unwrap();
            tx.send(skarn_ai::AgentEvent::State(AgentTurnState::RunningTool(
                "read_file".into(),
            )))
            .unwrap();
            tx.send(skarn_ai::AgentEvent::Delta(second.into()))
                .unwrap();
            tx.send(skarn_ai::AgentEvent::TurnDone(report.clone()))
                .unwrap();
            drop(tx);
            tauri::async_runtime::block_on(async {
                let join = tokio::spawn(async move { report });
                agent_relay(AgentRelayCtx {
                    log: Arc::clone(&log),
                    engine,
                    app: mock_app_handle(),
                    request_id: RequestId::new("request-1").unwrap(),
                    session_id: SessionId::new("session-1").unwrap(),
                    turn_id: TurnId::new("turn-1").unwrap(),
                    api_key: "secret".into(),
                    events_rx: rx,
                    join,
                })
                .await;
            });
            let text: String = log
                .events_after(0)
                .into_iter()
                .filter_map(|event| match event.payload {
                    FrontendEvent::ChatDelta(delta) => Some(delta.text),
                    _ => None,
                })
                .collect();
            assert_eq!(text, expected);
        }
    }

    #[test]
    fn agent_relay_keeps_a_key_prefix_across_ordinary_events() {
        // A tool-neutral ordinary event (todos) must not seal a buffered key
        // prefix: the stream resumes and completes the key in one redaction.
        let state = DesktopState::with_settings(skarn_settings::Settings::default());
        let log = Arc::new(EventLog::new(state.core_instance_id()));
        let engine = Arc::new(SessionEngine::new(EngineConfig {
            base_url: "http://127.0.0.1:1".into(),
            active_model: None,
            agent: skarn_settings::AgentSettings::default(),
            key_source: ApiKeySource::Fixed(Some("secret".into())),
        }));
        let report = skarn_ai::AgentTurnReport {
            summary: AgentSummary {
                iterations: 0,
                tools_used: 0,
                approved: 0,
                denied: 0,
                tokens: 0,
                cost_usd: 0.0,
                completed: true,
            },
            end: skarn_ai::TurnEnd::Completed,
            final_message: "secret".into(),
            conversation: Vec::new(),
        };
        let (tx, rx) = mpsc::unbounded_channel();
        tx.send(skarn_ai::AgentEvent::Delta("sec".into())).unwrap();
        tx.send(skarn_ai::AgentEvent::TodosChanged(Vec::new()))
            .unwrap();
        tx.send(skarn_ai::AgentEvent::Delta("ret".into())).unwrap();
        tx.send(skarn_ai::AgentEvent::TurnDone(report.clone()))
            .unwrap();
        drop(tx);
        tauri::async_runtime::block_on(async {
            let join = tokio::spawn(async move { report });
            agent_relay(AgentRelayCtx {
                log: Arc::clone(&log),
                engine,
                app: mock_app_handle(),
                request_id: RequestId::new("request-1").unwrap(),
                session_id: SessionId::new("session-1").unwrap(),
                turn_id: TurnId::new("turn-1").unwrap(),
                api_key: "secret".into(),
                events_rx: rx,
                join,
            })
            .await;
        });
        let events = log.events_after(0);
        let todos = events
            .iter()
            .position(|event| matches!(event.payload, FrontendEvent::TodosChanged(_)))
            .expect("ordinary event relayed");
        assert!(
            !events[..todos]
                .iter()
                .any(|event| matches!(event.payload, FrontendEvent::ChatDelta(_))),
            "a prefix must not be sealed before the stream resolves"
        );
        let text: String = events
            .into_iter()
            .filter_map(|event| match event.payload {
                FrontendEvent::ChatDelta(delta) => Some(delta.text),
                _ => None,
            })
            .collect();
        assert_eq!(text, "[REDACTED]");
        assert!(!serde_json::to_string(&log.events_after(0))
            .unwrap()
            .contains("secret"));
    }

    #[test]
    fn agent_relay_fails_when_tool_record_cannot_be_persisted() {
        for (tool_event, message) in [
            (
                skarn_ai::AgentEvent::ToolCallStarted(skarn_ai::ToolCall {
                    id: "call-1".into(),
                    name: "read_file".into(),
                    arguments: "{}".into(),
                }),
                "Could not persist tool call; turn stopped.",
            ),
            (
                skarn_ai::AgentEvent::ToolResult(skarn_ai::ToolResult::success(
                    "call-1", "result",
                )),
                "Could not persist tool result; turn stopped.",
            ),
        ] {
            let state = DesktopState::with_settings(skarn_settings::Settings::default());
            let log = Arc::new(EventLog::new(state.core_instance_id()));
            let engine = Arc::new(SessionEngine::new(EngineConfig {
                base_url: "http://127.0.0.1:1".into(),
                active_model: None,
                agent: skarn_settings::AgentSettings::default(),
                key_source: ApiKeySource::Fixed(Some("secret".into())),
            }));
            let (tx, rx) = mpsc::unbounded_channel();
            tx.send(tool_event).unwrap();
            drop(tx);
            tauri::async_runtime::block_on(async {
                let join = tokio::spawn(std::future::pending::<skarn_ai::AgentTurnReport>());
                agent_relay(AgentRelayCtx {
                    log: Arc::clone(&log),
                    engine,
                    app: mock_app_handle(),
                    request_id: RequestId::new("request-1").unwrap(),
                    session_id: SessionId::new("session-1").unwrap(),
                    turn_id: TurnId::new("turn-1").unwrap(),
                    api_key: "secret".into(),
                    events_rx: rx,
                    join,
                })
                .await;
            });
            let events = log.events_after(0);
            assert!(!events.iter().any(|event| matches!(
                event.payload,
                FrontendEvent::ToolCallStarted(_) | FrontendEvent::ToolResult(_)
            )));
            assert!(events.iter().any(|event| matches!(&event.payload, FrontendEvent::AgentTurnDone(done) if !done.completed && done.end_message == message)));
        }
    }

    #[test]
    fn chat_stream_redacts_a_key_split_across_deltas() {
        let app = mock_app_handle();
        let server = mock_gateway(
            vec![(
                "echo",
                sse(&[
                    content_chunk("sk-gw-desktop-"),
                    content_chunk("test-key"),
                    usage_chunk("stop", 1, 1, 0.0),
                ]),
            )],
            true,
        );
        let state = DesktopState::with_settings(gateway_settings(server.uri()));
        state
            .dispatch(
                &app,
                envelope(&state, send_chat("chat-1", "session-1", "echo")),
            )
            .unwrap();
        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatCompleted(_))
        });
        let wire = serde_json::to_string(&events).unwrap();
        assert!(!wire.contains("sk-gw-desktop-test-key"));
        assert!(wire.contains("[REDACTED]"));
        let snapshot = serde_json::to_string(&state.bootstrap().chat).unwrap();
        assert!(!snapshot.contains("sk-gw-desktop-test-key"));
    }

    #[test]
    fn send_chat_streams_deltas_and_lands_the_transcript() {
        let app = mock_app_handle();
        let server = mock_gateway(
            vec![(
                "list 3 primes",
                sse(&[
                    content_chunk("Hello"),
                    content_chunk(" world"),
                    usage_chunk("stop", 9, 2, 0.0013),
                ]),
            )],
            true,
        );
        let state = DesktopState::with_settings(gateway_settings(server.uri()));

        state
            .dispatch(
                &app,
                envelope(&state, send_chat("chat-1", "session-1", "list 3 primes")),
            )
            .expect("send_chat accepted");

        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatCompleted(_))
        });

        // The wire order: two deltas then the completion, monotonic, all
        // bound to the request and the session.
        assert_eq!(
            events
                .iter()
                .map(tag)
                .filter(|t| !t.starts_with("other:"))
                .collect::<Vec<_>>(),
            vec!["delta:Hello", "delta: world", "completed"]
        );
        assert_eq!(
            events
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        for event in &events {
            assert_eq!(
                event.request_id,
                Some(RequestId::new("chat-1").expect("valid id"))
            );
            assert_eq!(
                event.session_id,
                Some(SessionId::new("session-1").expect("valid id"))
            );
        }

        // The deltas share one message id and count 1, 2 within it.
        let deltas: Vec<&ChatDeltaDto> = events
            .iter()
            .filter_map(|event| match &event.payload {
                FrontendEvent::ChatDelta(dto) => Some(dto),
                _ => None,
            })
            .collect();
        assert_eq!(deltas.len(), 2);
        assert_eq!(deltas[0].message_id, deltas[1].message_id);
        assert_eq!(deltas[0].sequence, 1);
        assert_eq!(deltas[1].sequence, 2);

        // The completion carries the whole text and the receipt's metering.
        let completed = find_payload(&events, |payload| match payload {
            FrontendEvent::ChatCompleted(dto) => Some(dto.clone()),
            _ => None,
        });
        assert_eq!(completed.text, "Hello world");
        assert_eq!(completed.total_tokens, 11);
        assert!((completed.cost_usd - 0.0013).abs() < 1e-12);

        // The transcript: one session, auto-titled, user + assistant.
        let chat = state.bootstrap().chat;
        assert_eq!(chat.sessions.len(), 1);
        assert_eq!(chat.sessions[0].title, "list 3 primes");
        assert_eq!(chat.messages.len(), 2);
        assert_eq!(chat.messages[0].role, MessageRole::User);
        assert_eq!(chat.messages[0].text, "list 3 primes");
        assert_eq!(chat.messages[1].role, MessageRole::Assistant);
        assert_eq!(chat.messages[1].text, "Hello world");
        assert!(chat.active_request_id.is_none(), "the stream ended");
        assert!(chat.pending_approval.is_none());

        // Stopping a finished request is a silent idempotent success.
        state
            .dispatch(&app, envelope(&state, stop_chat("chat-1")))
            .expect("stopping a finished request is fine");
        assert_eq!(state.events_after(0).len(), 3, "no new events");
    }

    #[test]
    fn chat_failure_maps_to_typed_error_and_keeps_the_user_turn() {
        let app = mock_app_handle();
        let server = tauri::async_runtime::block_on(async {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
                .expect(1) // 401 is never retried
                .mount(&server)
                .await;
            server
        });
        let state = DesktopState::with_settings(gateway_settings(server.uri()));

        state
            .dispatch(
                &app,
                envelope(&state, send_chat("chat-1", "session-1", "hello?")),
            )
            .expect("send accepted");

        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatFailed(_))
        });
        let failed = find_payload(&events, |payload| match payload {
            FrontendEvent::ChatFailed(dto) => Some(dto.clone()),
            _ => None,
        });
        assert_eq!(failed.error.code, "auth_failed");
        assert!(!failed.error.retryable);
        // The user turn survives for the frontend's Retry.
        let chat = state.bootstrap().chat;
        assert_eq!(chat.messages.len(), 1);
        assert_eq!(chat.messages[0].text, "hello?");
        // The key never rode an event.
        let json = serde_json::to_string(&events).expect("events serialize");
        assert!(!json.contains("sk-gw-desktop-test-key"));
    }

    #[test]
    fn stop_chat_preserves_the_partial_bubble_silently() {
        let app = mock_app_handle();
        let uri = stalling_gateway("Hello ");
        let state = DesktopState::with_settings(gateway_settings(uri));

        state
            .dispatch(
                &app,
                envelope(&state, send_chat("chat-1", "session-1", "write a poem")),
            )
            .expect("send accepted");
        // The first delta lands while the gateway stalls.
        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatDelta(_))
        });

        state
            .dispatch(&app, envelope(&state, stop_chat("chat-1")))
            .expect("stop accepted");

        // A user stop is never a ChatError: the partial bubble completes.
        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatCompleted(_))
        });
        assert!(
            events
                .iter()
                .all(|event| !matches!(event.payload, FrontendEvent::ChatFailed(_))),
            "no failure event for a user stop"
        );
        let completed = find_payload(&events, |payload| match payload {
            FrontendEvent::ChatCompleted(dto) => Some(dto.clone()),
            _ => None,
        });
        assert_eq!(completed.text, "Hello ");
        // The partial answer is a transcript entry the restart keeps.
        let chat = state.bootstrap().chat;
        assert_eq!(chat.messages.len(), 2);
        assert_eq!(chat.messages[1].text, "Hello ");
    }

    #[test]
    fn stream_failures_map_to_typed_user_readable_codes() {
        let key = "sk-secret-key";
        let timeout = SkarnError::Net(format!(
            "{}chat stream timed out after 90s",
            skarn_ai::TIMEOUT_MARKER
        ));
        let timeout = to_protocol_error(&timeout, key);
        assert_eq!(timeout.code, "timeout");
        assert!(timeout.retryable);
        assert!(timeout.message.contains("did not answer in time"));
        assert!(
            !timeout.message.contains(key),
            "key leaked: {}",
            timeout.message
        );

        let auth = to_protocol_error(
            &SkarnError::Auth("key invalid or not authorized (HTTP 401)".into()),
            key,
        );
        assert_eq!(auth.code, "auth_failed");
        assert!(!auth.retryable);
        assert!(auth.message.contains("Check the key in Settings"));
        assert!(!auth.message.contains(key));

        let offline = to_protocol_error(
            &SkarnError::Net("cannot reach the AI gateway (https://x): refused".into()),
            key,
        );
        assert_eq!(offline.code, "network_error");
        assert!(offline.retryable);
        assert!(offline.message.contains("internet connection"));

        let down = to_protocol_error(
            &SkarnError::Ai("gateway unavailable (HTTP 503): busy".into()),
            key,
        );
        assert_eq!(down.code, "gateway_unavailable");
        assert!(down.retryable);
        assert!(down.message.contains("temporarily unavailable"));

        let rejected = to_protocol_error(
            &SkarnError::Ai("gateway rejected request (HTTP 400): bad model".into()),
            key,
        );
        assert_eq!(rejected.code, "gateway_error");
        assert!(!rejected.retryable);
        assert!(rejected.message.contains("bad model"));
    }

    #[test]
    fn session_busy_refuses_a_second_stream_without_mutating() {
        let app = mock_app_handle();
        // The response stalls server-side: the first stream stays in flight.
        let server = tauri::async_runtime::block_on(async {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .and(body_string_contains("first question".to_owned()))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_string(sse(&[content_chunk("…"), usage_chunk("stop", 1, 1, 0.0)]))
                        .set_delay(Duration::from_secs(10)),
                )
                .expect(1)
                .mount(&server)
                .await;
            server
        });
        let state = DesktopState::with_settings(gateway_settings(server.uri()));

        state
            .dispatch(
                &app,
                envelope(&state, send_chat("chat-1", "session-1", "first question")),
            )
            .expect("first send accepted");
        let error = state
            .dispatch(
                &app,
                envelope(&state, send_chat("chat-2", "session-1", "second question")),
            )
            .expect_err("the session is busy");
        assert_eq!(error.code, "session_busy");
        assert!(!error.retryable);

        // The refusal never touched the transcript: one user message.
        let chat = state.bootstrap().chat;
        assert_eq!(chat.messages.len(), 1);
        assert_eq!(chat.messages[0].text, "first question");
        assert_eq!(
            chat.active_request_id.expect("still streaming").as_str(),
            "chat-1"
        );

        // Synchronize before stopping: the first request must have reached
        // the (delayed) mock, or the stop would race it (the egui tests'
        // rule — the relay's request is async).
        sync_with_server(&server);

        // Teardown: stop the stalled stream so the test ends promptly.
        state
            .dispatch(&app, envelope(&state, stop_chat("chat-1")))
            .expect("stop accepted");
        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatCompleted(_))
        });
    }

    // ------------------------------------------------------------------
    // SendAgent: the full vertical — tools, approval, decision, turn done
    // ------------------------------------------------------------------

    /// The 3-round script: read NOTES.md (a read tool, auto-runs), write a
    /// line (a diff card — parked until approved), then the final answer.
    fn agent_rounds(new_content: &str) -> Vec<(&'static str, String)> {
        let write_arguments =
            serde_json::json!({ "path": "NOTES.md", "content": new_content }).to_string();
        vec![
            // Round 3 (mount first — its marker only exists once the write
            // result joined the conversation).
            (
                "bytes to",
                sse(&[
                    content_chunk("Done: the line is appended."),
                    usage_chunk("stop", 100, 50, 0.05),
                ]),
            ),
            // Round 2 (the write card round; the request now carries the
            // read result).
            (
                "notes-body-xyz",
                sse(&[
                    fragment_chunk(0, "call_write", "write_file", &write_arguments),
                    usage_chunk("tool_calls", 100, 50, 0.20),
                ]),
            ),
            // Round 1 (only the prompt distinguishes it).
            (
                "append a line",
                sse(&[
                    content_chunk("Reading the notes first."),
                    fragment_chunk(0, "call_read", "read_file", r#"{"path":"NOTES.md"}"#),
                    usage_chunk("tool_calls", 100, 50, 0.10),
                ]),
            ),
        ]
    }

    #[test]
    fn agent_turn_streams_tools_approvals_and_completes() {
        let app = mock_app_handle();
        let root = temp_workspace("full-turn");
        let new_content = "notes-body-xyz\nappended line\n";
        let server = mock_gateway(agent_rounds(new_content), true);
        let state = DesktopState::with_settings(gateway_settings(server.uri()));
        seed_supported_test_model(&state);
        state.sessions().set_workspace_root(Some(root.clone()));

        state
            .dispatch(
                &app,
                envelope(&state, send_agent("agent-1", "session-1", "append a line")),
            )
            .expect("send_agent accepted");

        // The card parks; its announcement carries the write's diff.
        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ApprovalNeeded(_))
        });
        let approval = find_payload(&events, |payload| match payload {
            FrontendEvent::ApprovalNeeded(dto) => Some(dto.clone()),
            _ => None,
        });
        assert_eq!(approval.tool_name, "write_file");
        assert_eq!(approval.path_or_command, "NOTES.md");
        let diff = approval.diff_or_input.expect("a diff for a write card");
        assert!(diff.contains("appended line"));
        // The file is untouched — the card IS the save moment.
        assert_eq!(
            std::fs::read_to_string(root.join("NOTES.md")).expect("seeded file"),
            "notes-body-xyz\n"
        );

        // The user approves through the command path.
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    agent_decision(
                        "agent-1",
                        approval.approval_id.as_str(),
                        DecisionDto::Approve,
                    ),
                ),
            )
            .expect("the approval resolves");

        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::AgentTurnDone(_))
        });

        // The exact wire order (the contract the dock renders against).
        assert_eq!(
            events
                .iter()
                .map(tag)
                .filter(|t| !t.starts_with("other:"))
                .collect::<Vec<_>>(),
            vec![
                "state:thinking",
                "delta:Reading the notes first.",
                "state:running:read_file",
                "tool_start:read_file",
                "tool_result:call_read",
                "state:thinking",
                "state:running:write_file",
                "tool_start:write_file",
                "state:awaiting",
                "approval:write_file",
                "state:running:write_file",
                "tool_result:call_write",
                "state:thinking",
                "delta:Done: the line is appended.",
                "state:done",
                "turn_done",
            ]
        );
        // Every event rides the request and the session, one monotonic
        // sequence with no gaps.
        for (index, event) in events.iter().enumerate() {
            assert_eq!(event.sequence, index as u64 + 1);
            assert_eq!(
                event.request_id,
                Some(RequestId::new("agent-1").expect("valid id"))
            );
            assert_eq!(
                event.session_id,
                Some(SessionId::new("session-1").expect("valid id"))
            );
        }

        // The footer: 3 rounds × 150 tokens, the summed cost, 2 tools,
        // 1 approved, the normal exit.
        let done = find_payload(&events, |payload| match payload {
            FrontendEvent::AgentTurnDone(dto) => Some(dto.clone()),
            _ => None,
        });
        assert_eq!(done.total_tokens, 450);
        assert!((done.cost_usd - 0.35).abs() < 1e-9);
        assert_eq!(done.num_tools, 2);
        assert_eq!(done.approved, 1);
        assert_eq!(done.denied, 0);
        assert!(done.completed);
        assert_eq!(done.end_message, "agent turn complete");

        // The approval executed for real: the file on disk changed.
        assert_eq!(
            std::fs::read_to_string(root.join("NOTES.md")).expect("written file"),
            new_content
        );

        // Single-use: the consumed id can never resolve again.
        let error = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    agent_decision(
                        "agent-1",
                        approval.approval_id.as_str(),
                        DecisionDto::Approve,
                    ),
                ),
            )
            .expect_err("the approval was consumed");
        assert_eq!(error.code, "approval_not_pending");

        // The transcript: the user turn + the final answer; no parked card.
        let chat = state.bootstrap().chat;
        assert_eq!(chat.messages.len(), 2);
        assert_eq!(chat.messages[1].text, "Done: the line is appended.");
        assert!(chat.pending_approval.is_none());
        assert!(chat.active_request_id.is_none());

        // The API key never crossed into an event.
        let json = serde_json::to_string(&events).expect("events serialize");
        assert!(!json.contains("sk-gw-desktop-test-key"));

        // Every scripted round was actually hit.
        tauri::async_runtime::block_on(async {
            server.verify().await;
        });

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn plan_mode_and_todos_ride_the_agent_stream_and_the_chat_snapshot() {
        let app = mock_app_handle();
        let root = temp_workspace("plan-hydrate");
        let server = mock_gateway(
            vec![
                // Mounted most specific first: the next request is
                // identified by the previous round's tool result / prompt.
                (
                    "Stored 2 todo(s)",
                    answer_round_sse("Planning is ready.", 0.01),
                ),
                (
                    "PLAN_HYDRATE_PROMPT",
                    tool_round_sse(
                        &[
                            ("call_enter", "enter_plan_mode", "{}"),
                            (
                                "call_todo",
                                "todo_write",
                                r#"{"todos":[{"title":"Audit the router","status":"completed"},{"title":"Write the plan"}]}"#,
                            ),
                        ],
                        "Reading the workspace first.",
                        0.01,
                    ),
                ),
            ],
            true,
        );
        let state = DesktopState::with_settings(gateway_settings(server.uri()));
        seed_supported_test_model(&state);
        state.sessions().set_workspace_root(Some(root.clone()));

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    send_agent("plan-h1", "session-plan", "PLAN_HYDRATE_PROMPT"),
                ),
            )
            .expect("send_agent accepted");

        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::AgentTurnDone(_))
        });
        // The live events are additive and carry the stored state.
        assert!(events.iter().any(|event| matches!(
            &event.payload,
            FrontendEvent::PlanStateChanged(dto) if dto.phase == PlanPhaseDto::Planning
        )));
        let todos = find_payload(&events, |payload| match payload {
            FrontendEvent::TodosChanged(dto) => Some(dto.todos.clone()),
            _ => None,
        });
        assert_eq!(todos.len(), 2);
        assert_eq!(todos[0].title, "Audit the router");
        assert_eq!(todos[0].status, TodoStatusDto::Completed);

        // Hydration: the bootstrap snapshot is the source of truth after a
        // reload — the session carries the phase and the same stored list.
        let chat = state.bootstrap().chat;
        let session = chat.sessions.first().expect("the session exists");
        assert_eq!(session.plan_phase, PlanPhaseDto::Planning);
        assert_eq!(session.todos.len(), 2);
        assert_eq!(session.todos[1].title, "Write the plan");
        assert_eq!(session.todos[1].status, TodoStatusDto::Pending);
        assert!(session.plan.is_none(), "no plan was approved yet");
        // The tool results are real: entering plan mode and storing todos
        // succeeded, and the write tools were never called.
        let final_state = state.bootstrap().chat;
        assert!(final_state.pending_approval.is_none());

        tauri::async_runtime::block_on(async {
            server.verify().await;
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn plan_card_revise_keeps_planning_then_approval_flips_and_writes_still_card() {
        let app = mock_app_handle();
        let root = temp_workspace("plan-revise-approve");
        let plan_args = r#"{"summary":"Make the file","plan":["Inspect","Write","Verify"]}"#;
        let server = mock_gateway(
            vec![
                ("wrote 8 bytes", answer_round_sse("Done.", 0.01)),
                (
                    "plan approved",
                    tool_round_sse(
                        &[(
                            "call_write",
                            "write_file",
                            r#"{"path":"made.txt","content":"approved"}"#,
                        )],
                        "Executing the approved plan.",
                        0.01,
                    ),
                ),
                (
                    "asked to revise the plan",
                    tool_round_sse(
                        &[("call_exit_2", "exit_plan_mode", plan_args)],
                        "Revising the plan.",
                        0.01,
                    ),
                ),
                (
                    "PLAN_FLOW_PROMPT",
                    tool_round_sse(
                        &[
                            ("call_enter", "enter_plan_mode", "{}"),
                            ("call_exit", "exit_plan_mode", plan_args),
                        ],
                        "Planning first.",
                        0.01,
                    ),
                ),
            ],
            true,
        );
        let state = DesktopState::with_settings(gateway_settings(server.uri()));
        seed_supported_test_model(&state);
        state.sessions().set_workspace_root(Some(root.clone()));

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    send_agent("plan-a", "session-approve", "PLAN_FLOW_PROMPT"),
                ),
            )
            .expect("send_agent accepted");

        // First card: the plan, with its steps on the parked approval slot
        // (so a reload can rebuild the same card).
        let first = wait_for_new_approval(&state, None);
        assert_eq!(first.summary, "exit_plan_mode Make the file");
        let plan = first.plan.expect("the parked card carries the plan");
        assert_eq!(plan.steps, ["Inspect", "Write", "Verify"]);
        assert!(!first.can_approve_all_this_turn);

        // Revise: the turn continues in plan mode and proposes again.
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    agent_decision("plan-a", first.approval_id.as_str(), DecisionDto::Revise),
                ),
            )
            .expect("revise resolves the card");
        let second = wait_for_new_approval(&state, Some(first.approval_id.as_str()));
        assert_eq!(
            plan_phase(&state, "session-approve"),
            PlanPhaseDto::Planning
        );

        // Approve: execution, with the approved plan stored and its steps
        // seeded as todos.
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    agent_decision("plan-a", second.approval_id.as_str(), DecisionDto::Approve),
                ),
            )
            .expect("approve resolves the plan");
        let third = wait_for_new_approval(&state, Some(second.approval_id.as_str()));
        assert_eq!(third.summary, "write_file made.txt");
        let session = state
            .bootstrap()
            .chat
            .sessions
            .into_iter()
            .find(|session| session.session_id.as_str() == "session-approve")
            .expect("session");
        assert_eq!(session.plan_phase, PlanPhaseDto::Executing);
        assert_eq!(session.plan.expect("approved plan").steps.len(), 3);
        assert_eq!(session.todos.len(), 3);
        // The plan approval never wrote the file; its own card is the save.
        assert!(!root.join("made.txt").exists());

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    agent_decision("plan-a", third.approval_id.as_str(), DecisionDto::Approve),
                ),
            )
            .expect("approve the write");

        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::AgentTurnDone(_))
        });
        assert_eq!(
            std::fs::read_to_string(root.join("made.txt")).expect("written file"),
            "approved"
        );

        tauri::async_runtime::block_on(async {
            server.verify().await;
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    fn plan_phase(state: &DesktopState, session_id: &str) -> PlanPhaseDto {
        state
            .bootstrap()
            .chat
            .sessions
            .into_iter()
            .find(|session| session.session_id.as_str() == session_id)
            .expect("session")
            .plan_phase
    }

    #[test]
    fn auto_mode_applies_file_writes_without_an_approval_card() {
        let app = mock_app_handle();
        let root = temp_workspace("auto-mode");
        let new_content = "notes-body-xyz\nauto-applied line\n";
        let server = mock_gateway(agent_rounds(new_content), true);
        let state = DesktopState::with_settings(gateway_settings(server.uri()));
        seed_supported_test_model(&state);
        state.sessions().set_workspace_root(Some(root.clone()));

        let mut command = send_agent("agent-auto", "session-auto", "append a line");
        if let FrontendCommand::SendAgent { request, .. } = &mut command {
            request.mode = AgentModeDto::AutoEdit;
        }
        state
            .dispatch(&app, envelope(&state, command))
            .expect("send_agent accepted");

        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::AgentTurnDone(_))
        });

        // No card was ever parked for the write.
        assert!(
            !events
                .iter()
                .any(|event| matches!(event.payload, FrontendEvent::ApprovalNeeded(_))),
            "auto mode must not emit an approval card"
        );
        let done = find_payload(&events, |payload| match payload {
            FrontendEvent::AgentTurnDone(dto) => Some(dto.clone()),
            _ => None,
        });
        assert!(done.completed);
        assert_eq!(done.approved, 1, "the auto-applied write is counted");
        assert_eq!(done.denied, 0);

        // The write executed for real.
        assert_eq!(
            std::fs::read_to_string(root.join("NOTES.md")).expect("written file"),
            new_content
        );

        tauri::async_runtime::block_on(async {
            server.verify().await;
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn approval_mismatch_keeps_the_card_parked() {
        let app = mock_app_handle();
        let root = temp_workspace("mismatch");
        let new_content = "notes-body-xyz\nappended line\n";
        let server = mock_gateway(agent_rounds(new_content), true);
        let state = DesktopState::with_settings(gateway_settings(server.uri()));
        seed_supported_test_model(&state);
        state.sessions().set_workspace_root(Some(root.clone()));

        state
            .dispatch(
                &app,
                envelope(&state, send_agent("agent-1", "session-1", "append a line")),
            )
            .expect("send_agent accepted");
        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ApprovalNeeded(_))
        });
        let approval = find_payload(&events, |payload| match payload {
            FrontendEvent::ApprovalNeeded(dto) => Some(dto.clone()),
            _ => None,
        });

        // A decision naming a different id is refused — and the real card
        // stays parked (not consumed).
        let error = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    agent_decision("agent-1", "not-the-approval", DecisionDto::Deny),
                ),
            )
            .expect_err("the ids do not line up");
        assert_eq!(error.code, "approval_mismatch");
        assert_eq!(
            std::fs::read_to_string(root.join("NOTES.md")).expect("seeded file"),
            "notes-body-xyz\n",
            "a mismatched decision executes nothing"
        );

        // The matching decision still resolves afterwards.
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    agent_decision(
                        "agent-1",
                        approval.approval_id.as_str(),
                        DecisionDto::Approve,
                    ),
                ),
            )
            .expect("the real approval resolves");
        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::AgentTurnDone(_))
        });
        assert_eq!(
            std::fs::read_to_string(root.join("NOTES.md")).expect("written file"),
            new_content
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn stop_chat_mid_agent_turn_ends_it_stopped_by_user() {
        let app = mock_app_handle();
        let root = temp_workspace("stop-mid-turn");
        let new_content = "notes-body-xyz\nappended line\n";
        let server = mock_gateway(agent_rounds(new_content), false);
        let state = DesktopState::with_settings(gateway_settings(server.uri()));
        seed_supported_test_model(&state);
        state.sessions().set_workspace_root(Some(root.clone()));

        state
            .dispatch(
                &app,
                envelope(&state, send_agent("agent-1", "session-1", "append a line")),
            )
            .expect("send_agent accepted");
        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ApprovalNeeded(_))
        });

        // StopChat routes to the agent turn too (one stop story): the loop
        // ends without the write ever executing.
        state
            .dispatch(&app, envelope(&state, stop_chat("agent-1")))
            .expect("stop accepted");
        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::AgentTurnDone(_))
        });
        let done = find_payload(&events, |payload| match payload {
            FrontendEvent::AgentTurnDone(dto) => Some(dto.clone()),
            _ => None,
        });
        assert!(!done.completed);
        assert_eq!(done.end_message, "agent turn stopped by user");
        assert_eq!(
            std::fs::read_to_string(root.join("NOTES.md")).expect("seeded file"),
            "notes-body-xyz\n",
            "a stopped turn never writes"
        );
        // No card stays parked for a dead turn.
        assert!(state.bootstrap().chat.pending_approval.is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn explore_mode_arms_the_read_only_manifest() {
        let app = mock_app_handle();
        let root = temp_workspace("explore");
        // One tool round (a read), then the answer.
        let server = mock_gateway(
            vec![
                (
                    "NOTES.md (lines",
                    sse(&[
                        content_chunk("The notes are read."),
                        usage_chunk("stop", 100, 50, 0.05),
                    ]),
                ),
                (
                    "explore the notes",
                    sse(&[
                        fragment_chunk(0, "call_read", "read_file", r#"{"path":"NOTES.md"}"#),
                        usage_chunk("tool_calls", 100, 50, 0.10),
                    ]),
                ),
            ],
            true,
        );
        let state = DesktopState::with_settings(gateway_settings(server.uri()));
        seed_supported_test_model(&state);
        state.sessions().set_workspace_root(Some(root.clone()));

        let mut command = send_agent("agent-1", "session-1", "explore the notes");
        if let FrontendCommand::SendAgent { request, .. } = &mut command {
            request.mode = AgentModeDto::ReadOnly;
        }
        state
            .dispatch(&app, envelope(&state, command))
            .expect("send_agent accepted");
        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::AgentTurnDone(_))
        });

        // The manifest the loop armed carries only read-only tools: the
        // model cannot even request a side effect.
        let requests = tauri::async_runtime::block_on(async { server.received_requests().await })
            .expect("the two rounds were recorded");
        assert_eq!(requests.len(), 2, "two rounds");
        for request in &requests {
            let body: serde_json::Value =
                serde_json::from_slice(&request.body).expect("request body is JSON");
            let tools: Vec<&str> = body["tools"]
                .as_array()
                .expect("the manifest rides every request")
                .iter()
                .map(|tool| tool["function"]["name"].as_str().expect("named tool"))
                .collect();
            assert!(tools.contains(&"read_file"), "read tools armed: {tools:?}");
            for side_effect in ["write_file", "edit_file", "delete_file", "bash"] {
                assert!(
                    !tools.contains(&side_effect),
                    "explore mode never arms {side_effect}: {tools:?}"
                );
            }
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    // ------------------------------------------------------------------
    // The boot probe (the model picker's data path)
    // ------------------------------------------------------------------

    #[test]
    fn boot_probe_announces_models_and_connection() {
        let server = tauri::async_runtime::block_on(async {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/v1/models"))
                .and(header("authorization", "Bearer sk-gw-desktop-test-key"))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({ "object": "list", "data": [
                        {
                            "id": "glm-5.3", "display_name": "GLM 5.3",
                            "context_window": 200000,
                            "price_per_1k_input": 0.01, "price_per_1k_output": 0.03,
                            "supports_streaming": true, "supports_tools": true,
                            "family": "glm"
                        }
                    ]}),
                ))
                .expect(1)
                .mount(&server)
                .await;
            server
        });
        let state = DesktopState::with_settings(gateway_settings(server.uri()));

        tauri::async_runtime::block_on(async {
            state
                .sessions()
                .connect_probe(state.event_log(), mock_app_handle())
                .await;
        });

        let events = state.events_after(0);
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0].payload, FrontendEvent::ModelsChanged(_)));
        assert!(matches!(
            events[1].payload,
            FrontendEvent::SkarnStatusChanged(_)
        ));
        let boot = state.bootstrap();
        assert_eq!(boot.connection, ConnectionStatus::Connected);
        assert_eq!(boot.models.len(), 1);
        assert_eq!(boot.models[0].display_name, "GLM 5.3");
        // The catalog feeds model resolution: a catalog hit carries the
        // context window (the caps' source).
        assert_eq!(
            state
                .sessions()
                .resolve_model_for_test(Some("glm-5.3"))
                .expect("catalog model resolves")
                .context_window,
            200_000
        );
    }

    // ------------------------------------------------------------------
    // Phase 9 session slice — todos, new/switch isolation, retry
    // ------------------------------------------------------------------

    fn session_command(command: FrontendCommand) -> FrontendCommand {
        command
    }

    fn new_session_command() -> FrontendCommand {
        session_command(FrontendCommand::NewSession)
    }

    fn switch_session_command(session: &str) -> FrontendCommand {
        FrontendCommand::SwitchSession {
            session_id: SessionId::new(session).expect("valid id"),
        }
    }

    fn delete_session_command(session: &str) -> FrontendCommand {
        FrontendCommand::DeleteSession {
            session_id: SessionId::new(session).expect("valid id"),
        }
    }

    fn retry_command(session: &str, message: MessageId) -> FrontendCommand {
        FrontendCommand::RetryMessage {
            session_id: SessionId::new(session).expect("valid id"),
            message_id: message,
        }
    }

    fn messages_for(
        snapshot: &skarn_protocol::BootstrapState,
        session: &str,
    ) -> Vec<skarn_protocol::MessageDto> {
        snapshot
            .chat
            .messages
            .iter()
            .filter(|message| message.session_id.as_str() == session)
            .cloned()
            .collect()
    }

    #[test]
    fn manage_todo_stores_session_scoped_todos_and_hydrates_the_chat_state() {
        let app = mock_app_handle();
        let root = temp_workspace("todos");
        // Round 1: the model writes a plan. Round 2: the tool result is in the
        // request body, so that marker only matches after the store.
        let server = mock_gateway(
            vec![
                (
                    "Stored 2 todo(s)",
                    sse(&[
                        content_chunk("The plan is stored."),
                        usage_chunk("stop", 10, 5, 0.01),
                    ]),
                ),
                (
                    "plan the work",
                    sse(&[
                        fragment_chunk(
                            0,
                            "call_todo",
                            "manage_todo",
                            r#"{"todos":[
                                {"id":"t1","text":"inspect the router","status":"completed"},
                                {"id":"t2","text":"store the todos","status":"in_progress"}
                            ]}"#,
                        ),
                        usage_chunk("tool_calls", 10, 5, 0.01),
                    ]),
                ),
            ],
            true,
        );
        let state = DesktopState::with_settings(gateway_settings(server.uri()));
        seed_supported_test_model(&state);
        state.sessions().set_workspace_root(Some(root.clone()));

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    send_agent("agent-todo", "session-todo", "plan the work"),
                ),
            )
            .expect("send_agent accepted");
        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::AgentTurnDone(_))
        });

        // The live announcement carries the stored list (not the raw args).
        let changed = find_payload(&events, |payload| match payload {
            FrontendEvent::TodosChanged(dto) => Some(dto.clone()),
            _ => None,
        });
        assert_eq!(changed.session_id.as_str(), "session-todo");
        assert_eq!(changed.todos.len(), 2);
        assert_eq!(changed.todos[0].id, "t1");
        assert_eq!(
            changed.todos[0].status,
            skarn_protocol::TodoStatusDto::Completed
        );
        assert_eq!(changed.todos[1].title, "store the todos");

        // The bootstrap (the hydrate path) carries the same list, both on
        // the active session and per session.
        let chat = state.bootstrap().chat;
        assert_eq!(
            chat.active_session_id.expect("active").as_str(),
            "session-todo"
        );
        assert_eq!(chat.todos.len(), 2);
        assert_eq!(
            chat.todos[1].status,
            skarn_protocol::TodoStatusDto::InProgress
        );
        let session = chat
            .sessions
            .iter()
            .find(|session| session.session_id.as_str() == "session-todo")
            .expect("the session is listed");
        assert_eq!(session.todos.len(), 2);

        // New sessions start empty; switching back restores the same list.
        state
            .dispatch(&app, envelope(&state, new_session_command()))
            .expect("new session");
        let fresh = state.bootstrap().chat;
        assert_eq!(fresh.todos.len(), 0, "todos are session-scoped");
        let fresh_id = fresh.active_session_id.expect("new session active");
        assert_ne!(fresh_id.as_str(), "session-todo");

        state
            .dispatch(
                &app,
                envelope(&state, switch_session_command("session-todo")),
            )
            .expect("switch back");
        let back = state.bootstrap().chat;
        assert_eq!(
            back.active_session_id.expect("active").as_str(),
            "session-todo"
        );
        assert_eq!(back.todos.len(), 2);
        assert_eq!(back.todos[0].title, "inspect the router");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn new_and_switch_sessions_keep_independent_transcripts() {
        let app = mock_app_handle();
        let server = mock_gateway(
            vec![(
                "session A",
                sse(&[content_chunk("answer A"), usage_chunk("stop", 1, 1, 0.0)]),
            )],
            true,
        );
        let state = DesktopState::with_settings(gateway_settings(server.uri()));
        let root = tempfile::tempdir().unwrap();
        state
            .sessions()
            .set_workspace_root(Some(root.path().to_path_buf()));

        state
            .dispatch(
                &app,
                envelope(&state, send_chat("chat-a", "session-a", "session A")),
            )
            .expect("send accepted");
        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatCompleted(_))
        });
        assert_eq!(messages_for(&state.bootstrap(), "session-a").len(), 2);

        // NewSession is a real, empty session; A's transcript stays with A.
        state
            .dispatch(&app, envelope(&state, new_session_command()))
            .expect("new session");
        let created = state.bootstrap();
        let new_id = created.chat.active_session_id.clone().expect("active");
        assert_ne!(new_id.as_str(), "session-a");
        assert!(messages_for(&created, new_id.as_str()).is_empty());
        assert_eq!(messages_for(&created, "session-a").len(), 2);
        // The transition emitted the hydrate snapshot.
        assert!(state.events_after(0).iter().any(|event| matches!(
            &event.payload,
            FrontendEvent::BootstrapState(snapshot)
                if snapshot.chat.active_session_id.as_ref() == Some(&new_id)
        )));

        // Switching back returns A's own transcript and marks A active.
        state
            .dispatch(&app, envelope(&state, switch_session_command("session-a")))
            .expect("switch back");
        let back = state.bootstrap();
        assert_eq!(
            back.chat
                .active_session_id
                .as_ref()
                .expect("active")
                .as_str(),
            "session-a"
        );
        assert_eq!(messages_for(&back, "session-a").len(), 2);
        assert!(messages_for(&back, new_id.as_str()).is_empty());

        // An unknown id is refused, never created implicitly.
        let error = state
            .dispatch(
                &app,
                envelope(&state, switch_session_command("no-such-session")),
            )
            .expect_err("unknown session");
        assert_eq!(error.code, "session_not_found");
        assert_eq!(
            state
                .bootstrap()
                .chat
                .active_session_id
                .expect("active")
                .as_str(),
            "session-a",
            "a refused switch changes nothing"
        );
    }

    #[test]
    fn delete_session_removes_open_and_disk_only_sessions_and_refreshes_bootstrap() {
        let app = mock_app_handle();
        let root = temp_workspace("delete-session");
        seed_session_store(&root, "sess-open");
        seed_session_store(&root, "sess-disk");
        let state = DesktopState::with_settings(skarn_settings::Settings::default());
        let engine = state.sessions();
        engine.set_workspace_root(Some(root.clone()));
        let open = SessionId::new("sess-open").expect("id");
        engine.switch_session(&open).expect("open the session");

        let before = state.bootstrap();
        assert!(before
            .agent_sessions
            .iter()
            .any(|session| session.session_id.as_str() == "sess-disk"));

        // A disk-only durable session is not in the dock, but delete still
        // removes its store and the next bootstrap no longer offers it.
        state
            .dispatch(&app, envelope(&state, delete_session_command("sess-disk")))
            .expect("delete disk-only");
        assert!(!root.join(".selora/agent-sessions/sess-disk").exists());
        let after_disk = state.bootstrap();
        assert!(!after_disk
            .agent_sessions
            .iter()
            .any(|session| session.session_id.as_str() == "sess-disk"));
        assert!(after_disk
            .chat
            .sessions
            .iter()
            .any(|session| session.session_id.as_str() == "sess-open"));

        // Deleting the open durable session removes both the tab and its
        // transcript, and leaves no stale active id behind.
        state
            .dispatch(&app, envelope(&state, delete_session_command("sess-open")))
            .expect("delete open");
        assert!(!root.join(".selora/agent-sessions/sess-open").exists());
        let after_open = state.bootstrap();
        assert!(after_open.chat.sessions.is_empty());
        assert!(after_open.agent_sessions.is_empty());
        assert!(after_open.chat.active_session_id.is_none());
        assert!(state.events_after(0).iter().any(|event| matches!(
            &event.payload,
            FrontendEvent::BootstrapState(snapshot)
                if snapshot.chat.sessions.is_empty() && snapshot.agent_sessions.is_empty()
        )));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn delete_session_keeps_a_non_active_tab_and_selects_a_real_replacement() {
        let root = temp_workspace("delete-active");
        for id in ["sess-a", "sess-b", "sess-c", "sess-disk"] {
            seed_session_store(&root, id);
        }
        let state = DesktopState::with_settings(skarn_settings::Settings::default());
        let engine = state.sessions();
        engine.set_workspace_root(Some(root.clone()));
        let a = SessionId::new("sess-a").expect("id");
        let b = SessionId::new("sess-b").expect("id");
        let c = SessionId::new("sess-c").expect("id");
        let disk = SessionId::new("sess-disk").expect("id");
        engine.switch_session(&a).expect("open a");
        engine.switch_session(&b).expect("open b");
        engine.switch_session(&c).expect("open c");

        // Deleting another tab must not yank the user's active selection.
        engine.delete_session(&a).expect("delete a");
        engine.delete_session(&disk).expect("delete disk-only");
        let active = state.bootstrap().chat;
        assert_eq!(
            active
                .active_session_id
                .as_ref()
                .expect("still active")
                .as_str(),
            "sess-c"
        );
        assert_eq!(
            active
                .sessions
                .iter()
                .map(|session| session.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["sess-b", "sess-c"]
        );

        // Deleting the active tab selects a real remaining session, not a
        // synthetic row and not the stale id.
        engine.delete_session(&c).expect("delete c");
        let remaining = state.bootstrap().chat;
        assert_eq!(
            remaining
                .active_session_id
                .as_ref()
                .expect("b replaces c")
                .as_str(),
            "sess-b"
        );
        assert_eq!(remaining.sessions.len(), 1);
        assert_eq!(remaining.sessions[0].session_id.as_str(), "sess-b");

        engine.delete_session(&b).expect("delete b");
        assert!(state.bootstrap().chat.active_session_id.is_none());
        assert!(state.bootstrap().chat.sessions.is_empty());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn delete_session_refuses_a_live_turn_without_removing_anything() {
        let app = mock_app_handle();
        let uri = stalling_gateway("delete-live ");
        let state = DesktopState::with_settings(gateway_settings(uri));
        let root = tempfile::tempdir().unwrap();
        state
            .sessions()
            .set_workspace_root(Some(root.path().to_path_buf()));

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    send_chat("chat-delete", "session-delete", "stay live"),
                ),
            )
            .expect("send accepted");
        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatDelta(_))
        });

        let error = state
            .dispatch(
                &app,
                envelope(&state, delete_session_command("session-delete")),
            )
            .expect_err("a live turn refuses delete");
        assert_eq!(error.code, "session_busy");
        let chat = state.bootstrap().chat;
        assert!(chat
            .sessions
            .iter()
            .any(|session| session.session_id.as_str() == "session-delete"));
        assert_eq!(
            chat.active_session_id
                .as_ref()
                .expect("still active")
                .as_str(),
            "session-delete"
        );

        state
            .dispatch(&app, envelope(&state, stop_chat("chat-delete")))
            .expect("stop accepted");
        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatCompleted(_))
        });
    }

    #[test]
    fn switch_and_new_preserve_a_live_turn() {
        let app = mock_app_handle();
        let uri = stalling_gateway("first ");
        let state = DesktopState::with_settings(gateway_settings(uri));
        let root = tempfile::tempdir().unwrap();
        state
            .sessions()
            .set_workspace_root(Some(root.path().to_path_buf()));

        state
            .dispatch(
                &app,
                envelope(&state, send_chat("chat-live", "session-live", "stay live")),
            )
            .expect("send accepted");
        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatDelta(_))
        });

        state
            .dispatch(&app, envelope(&state, new_session_command()))
            .expect("new session does not cancel another session's turn");
        assert_ne!(
            state.bootstrap().chat.active_session_id.unwrap().as_str(),
            "session-live"
        );
        state
            .dispatch(
                &app,
                envelope(&state, switch_session_command("session-live")),
            )
            .expect("switch returns to the live turn");
        assert_eq!(
            state.bootstrap().chat.active_request_id.unwrap().as_str(),
            "chat-live"
        );

        state
            .dispatch(&app, envelope(&state, stop_chat("chat-live")))
            .expect("stop accepted");
        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatCompleted(_))
        });
    }

    #[test]
    fn retry_truncates_the_failed_tail_and_dispatches_once() {
        let app = mock_app_handle();
        // The first attempt fails at the gateway (401 is never retried).
        let failing = tauri::async_runtime::block_on(async {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .and(body_string_contains("retry me"))
                .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
                .expect(1)
                .mount(&server)
                .await;
            server
        });
        let state = DesktopState::with_settings(gateway_settings(failing.uri()));
        state
            .dispatch(
                &app,
                envelope(&state, send_chat("chat-first", "session-retry", "retry me")),
            )
            .expect("send accepted");
        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatFailed(_))
        });
        assert_eq!(
            find_payload(&events, |payload| match payload {
                FrontendEvent::ChatFailed(dto) => Some(dto.error.code.clone()),
                _ => None,
            }),
            "auth_failed"
        );
        let failed = state.bootstrap();
        let user_message = messages_for(&failed, "session-retry")
            .into_iter()
            .find(|message| message.role == MessageRole::User)
            .expect("the user turn survives a failure");
        assert_eq!(messages_for(&failed, "session-retry").len(), 1);

        // The retry re-dispatches the same content exactly once against the
        // healthy gateway.
        let success = mock_gateway(
            vec![(
                "retry me",
                sse(&[
                    content_chunk("answer on retry"),
                    usage_chunk("stop", 2, 3, 0.0),
                ]),
            )],
            true,
        );
        state.sessions().set_base_url(success.uri());
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    retry_command("session-retry", user_message.message_id.clone()),
                ),
            )
            .expect("retry accepted");
        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatCompleted(_))
        });
        assert_eq!(
            find_payload(&events, |payload| match payload {
                FrontendEvent::ChatCompleted(dto) => Some(dto.text.clone()),
                _ => None,
            }),
            "answer on retry"
        );
        tauri::async_runtime::block_on(async { success.verify().await });

        // Truncation: one user turn, then the retried answer — no duplicate
        // user bubble, no leftover failed bubble.
        let after = state.bootstrap();
        let transcript = messages_for(&after, "session-retry");
        assert_eq!(transcript.len(), 2, "{transcript:#?}");
        assert_eq!(transcript[0].role, MessageRole::User);
        assert_eq!(transcript[0].text, "retry me");
        assert_eq!(transcript[1].role, MessageRole::Assistant);
        assert_eq!(transcript[1].text, "answer on retry");
        assert!(after.chat.active_request_id.is_none());

        // The old user id is gone (it was re-created), and an assistant id
        // is not a retry target.
        let error = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    retry_command("session-retry", user_message.message_id),
                ),
            )
            .expect_err("the old id was truncated");
        assert_eq!(error.code, "unknown_message");
        let error = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    retry_command("session-retry", transcript[1].message_id.clone()),
                ),
            )
            .expect_err("only user messages retry");
        assert_eq!(error.code, "not_a_user_message");
        let error = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    retry_command(
                        "session-retry",
                        MessageId::new("no-such-message").expect("valid id"),
                    ),
                ),
            )
            .expect_err("unknown id");
        assert_eq!(error.code, "unknown_message");
    }

    #[test]
    fn retry_is_refused_during_a_live_turn() {
        let app = mock_app_handle();
        let uri = stalling_gateway("partial ");
        let state = DesktopState::with_settings(gateway_settings(uri));
        state
            .dispatch(
                &app,
                envelope(&state, send_chat("chat-live", "session-live", "stay live")),
            )
            .expect("send accepted");
        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatDelta(_))
        });
        let live_user = events
            .iter()
            .find(|event| matches!(event.payload, FrontendEvent::ChatDelta(_)))
            .and_then(|_| {
                state
                    .bootstrap()
                    .chat
                    .messages
                    .into_iter()
                    .find(|message| message.role == MessageRole::User)
            })
            .expect("the live user turn");
        let error = state
            .dispatch(
                &app,
                envelope(&state, retry_command("session-live", live_user.message_id)),
            )
            .expect_err("a live turn cannot retry");
        assert_eq!(error.code, "session_busy");
        state
            .dispatch(&app, envelope(&state, stop_chat("chat-live")))
            .expect("stop accepted");
        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatCompleted(_))
        });
    }

    // ------------------------------------------------------------------
    // The approval card body mapping (the WorkflowCard's data path)
    // ------------------------------------------------------------------

    #[test]
    fn approval_body_maps_the_workflow_card_to_the_confirmation_payload() {
        let card = ApprovalCard::Workflow {
            name: "repo-audit".into(),
            script: "phase('scan');\nawait world.run('cargo', ['test']);\n".into(),
            commands: vec!["cargo test".into(), "git status".into()],
            max_cost_usd: 2.5,
        };

        let (path_or_command, diff_or_input, workflow) = approval_body(&card);
        // The header target stays the workflow name; the script is NOT a diff.
        assert_eq!(path_or_command, "repo-audit");
        assert!(diff_or_input.is_none());
        // `tool_name` stays `run_workflow` so the frontend routes the card.
        assert_eq!(fallback_tool_name(&card), "run_workflow");

        let workflow = workflow.expect("a workflow card carries the payload");
        assert_eq!(workflow.name, "repo-audit");
        assert_eq!(
            workflow.script,
            "phase('scan');\nawait world.run('cargo', ['test']);\n"
        );
        assert_eq!(workflow.commands, vec!["cargo test", "git status"]);
        assert_eq!(workflow.max_cost_usd, 2.5);
    }

    #[test]
    fn approval_body_leaves_non_workflow_cards_unchanged() {
        let (path, diff, workflow) = approval_body(&ApprovalCard::Diff {
            path: "NOTES.md".into(),
            diff: "+line\n".into(),
        });
        assert_eq!(path, "NOTES.md");
        assert_eq!(diff.as_deref(), Some("+line\n"));
        assert!(workflow.is_none());

        let (path, diff, workflow) = approval_body(&ApprovalCard::Delete {
            path: "NOTES.md".into(),
        });
        assert_eq!(path, "NOTES.md");
        assert!(diff.is_none());
        assert!(workflow.is_none());

        let (command, diff, workflow) = approval_body(&ApprovalCard::Command {
            command: "cargo test".into(),
        });
        assert_eq!(command, "cargo test");
        assert!(diff.is_none());
        assert!(workflow.is_none());
        assert_eq!(
            fallback_tool_name(&ApprovalCard::Command {
                command: "cargo test".into()
            }),
            "bash"
        );
    }

    // ------------------------------------------------------------------
    // The system prompt's editor context (path/language/selection/dirty —
    // never the file text)
    // ------------------------------------------------------------------

    fn agent_payload(editor_context: Option<EditorContextDto>) -> AgentRequestDto {
        AgentRequestDto {
            session_id: SessionId::new("session-1").expect("valid id"),
            prompt: "do the thing".into(),
            model_id: Some("test-model".into()),
            mode: AgentModeDto::AskBeforeChanges,
            reasoning_effort: None,
            system_instructions: None,
            temperature: None,
            editor_context,
            attachments: Vec::new(),
        }
    }

    #[test]
    fn system_prompt_includes_editor_context() {
        let payload = agent_payload(Some(EditorContextDto {
            path: "crates/selora-ai/src/agent/turn.rs".into(),
            language: "rust".into(),
            selection: Some(Selection {
                anchor: TextPosition {
                    line: 6,
                    utf16_column: 0,
                },
                active: TextPosition {
                    line: 6,
                    utf16_column: 13,
                },
                version: 3,
            }),
            dirty: true,
        }));

        let prompt = system_prompt(&payload, Path::new("C:/repo"), "");
        assert!(
            prompt.contains("Active file (workspace-relative): crates/selora-ai/src/agent/turn.rs")
        );
        assert!(prompt.contains("Language: rust"));
        // 0-based protocol coordinates print 1-based, like `read_file` does.
        assert!(prompt.contains("Selection: 7:1-7:14"));
        assert!(prompt.contains("Status: has unsaved edits"));
        assert!(prompt.contains("do not claim to have read a file you have not read in this turn"));
    }

    #[test]
    fn system_prompt_without_editor_context_is_unchanged() {
        let payload = agent_payload(None);

        let prompt = system_prompt(&payload, Path::new("C:/repo"), "");
        assert!(!prompt.contains("Editor context"));

        // Today's prompt, hard-coded: root `C:/repo`, execute mode, standard
        // depth, no custom instructions, and the empty editor slot.
        let expected = format!(
            "You are Selora, an autonomous coding agent in Selora IDE.\n\
             Operating System: Windows. Shell: PowerShell.\n\
             Workspace Root: {}\n\
             Available tools: read_file, list_dir, fuzzy_find, search_files, write_file, edit_file, delete_file, bash, web_search, web_fetch, run_workflow, enter_plan_mode, exit_plan_mode, todo_read, todo_write, spawn_subagent, wait, send_message, ask_user_question.\n\
             Instructions:\n\
             1. You can read and change the project. Use `list_dir`, `fuzzy_find`, `search_files`, and `read_file` to inspect it, then `write_file`, `edit_file`, or `delete_file` to make the change.\n\
             2. Do not stop at describing a change: when the user asks for a code change, perform it with the tools. {}\n\
             3. Always inspect relevant files before modifying them, and re-read after a failed edit rather than guessing.\n\
             4. Break complex work into clear steps and keep the live todo list current with `todo_write` (read it with `todo_read`).\n\
             5. For a non-trivial change, call `enter_plan_mode`, explore read-only, then propose the plan with `exit_plan_mode`; while planning, every write, delete, bash command, and workflow is blocked until the user approves the plan. Approving the plan still leaves each write behind its own diff card.\n\
             6. Make clean, accurate edits and verify your work (build, test, or read the result back).\n\
             7. Use `spawn_subagent` for a bounded independent task. Give each child a short `name`; it labels the child session and its title-bar pet. For fan-out, parallelize, or independent work, call `spawn_subagent` several times with `background: true` in one round, then collect with `wait`. Default policy is read-only; use full only for delegated edits. Children cannot spawn children. Full child writes/bash still require individual approval.\n\
             {}{}{}",
            "C:/repo",
            "Writes, deletes, and non-allowlisted commands pause for the user's approval card; explain what you are doing while the card is pending.",
            "\nReasoning depth: Standard. Balanced reasoning with clear explanations and thorough verification.",
            "",
            ""
        );
        assert_eq!(prompt, expected);
    }

    #[test]
    fn system_prompt_omits_empty_selection() {
        let payload = agent_payload(Some(EditorContextDto {
            path: "apps/selora-desktop/src/main.rs".into(),
            language: "rust".into(),
            selection: Some(Selection {
                anchor: TextPosition {
                    line: 3,
                    utf16_column: 2,
                },
                active: TextPosition {
                    line: 3,
                    utf16_column: 2,
                },
                version: 1,
            }),
            dirty: false,
        }));

        let prompt = system_prompt(&payload, Path::new("C:/repo"), "");
        // A collapsed caret is not a selection, and no edits means no
        // status line — never `0:0-0:0` or an empty range.
        assert!(!prompt.contains("Selection:"));
        assert!(!prompt.contains("Status: has unsaved edits"));
        assert!(
            prompt.contains("Active file (workspace-relative): apps/selora-desktop/src/main.rs")
        );
        assert!(prompt.contains("do not claim to have read a file you have not read in this turn"));
    }

    // ------------------------------------------------------------------
    // Phase 9d — durable sessions, resume, retry, session trust
    // ------------------------------------------------------------------

    fn seed_session_store(root: &Path, session: &str) {
        let store = skarn_ai::AgentSessionStore::new(root);
        let mut state = store
            .create(
                session,
                "Persisted session",
                Some("test-model"),
                skarn_ai::AgentSessionMode::Execute,
            )
            .expect("create session");
        store
            .append(
                session,
                &skarn_ai::TranscriptEntry::User {
                    id: "m1".into(),
                    timestamp: 1,
                    content: "first prompt".into(),
                },
            )
            .expect("append user");
        store
            .append(
                session,
                &skarn_ai::TranscriptEntry::Assistant {
                    id: "m2".into(),
                    timestamp: 2,
                    content: "first answer".into(),
                    tool_calls: None,
                },
            )
            .expect("append assistant");
        store
            .append(
                session,
                &skarn_ai::TranscriptEntry::User {
                    id: "m3".into(),
                    timestamp: 3,
                    content: "second prompt".into(),
                },
            )
            .expect("append user");
        store
            .append(
                session,
                &skarn_ai::TranscriptEntry::Assistant {
                    id: "m4".into(),
                    timestamp: 4,
                    content: "second answer old".into(),
                    tool_calls: None,
                },
            )
            .expect("append old second answer");
        state.plan = skarn_ai::PlanState {
            status: skarn_ai::PlanStatus::Proposed,
            plan: Some("keep the plan".into()),
            updated_at: 4,
        };
        state.parks = vec![skarn_ai::ParkedCard::Approval {
            approval_id: Uuid::nil().to_string(),
            tool_name: "bash".into(),
            card: ApprovalCard::Command {
                command: "cargo test".into(),
            },
            request_id: Some("req-park".into()),
            turn_id: Some("turn-park".into()),
            created_at: 5,
        }];
        store.save_state(&state).expect("save state");
        store
            .save_todos(
                session,
                &[skarn_ai::TodoItem {
                    id: "t1".into(),
                    title: "restore me".into(),
                    status: skarn_ai::TodoStatus::InProgress,
                }],
            )
            .expect("save todos");
    }

    #[test]
    fn switch_restores_a_session_and_its_park_expires_once() {
        let root = temp_workspace("resume");
        seed_session_store(&root, "sess-restore");
        let state = DesktopState::with_settings(skarn_settings::Settings::default());
        state.sessions().set_workspace_root(Some(root.clone()));

        let listed = state.sessions().agent_sessions();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "Persisted session");
        assert_eq!(listed[0].message_count, 4);
        assert_eq!(listed[0].mode, "execute");
        assert_eq!(listed[0].pending.as_ref().expect("park").kind, "approval");

        // A fresh process has not restored anything yet.
        let before = state.bootstrap().chat;
        assert!(before.sessions.is_empty());
        assert!(before.messages.is_empty());

        state
            .sessions()
            .switch_session(&SessionId::new("sess-restore").expect("id"))
            .expect("switch restores");
        let chat = state.bootstrap().chat;
        assert_eq!(chat.sessions.len(), 1);
        assert_eq!(chat.messages.len(), 4);
        assert_eq!(chat.messages[0].text, "first prompt");
        assert_eq!(chat.messages[1].text, "first answer");
        assert_eq!(chat.messages[2].text, "second prompt");
        assert_eq!(chat.messages[3].text, "second answer old");
        let park = chat.pending_approval.expect("the parked card is restored");
        assert_eq!(park.request_id.as_str(), "req-park");

        // A restored park is actionable exactly once and never executes.
        let error = state
            .sessions()
            .agent_decision(
                &RequestId::new("req-park").expect("id"),
                &ApprovalId::new(Uuid::nil().to_string()).expect("id"),
                DecisionDto::Approve,
                None,
            )
            .expect_err("a restored park expires");
        assert_eq!(error.code, "park_expired");
        assert!(state.bootstrap().chat.pending_approval.is_none());
        let error = state
            .sessions()
            .agent_decision(
                &RequestId::new("req-park").expect("id"),
                &ApprovalId::new(Uuid::nil().to_string()).expect("id"),
                DecisionDto::Approve,
                None,
            )
            .expect_err("the second decision has no card");
        assert_eq!(error.code, "approval_not_pending");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn switch_restores_the_plan_phase_draft_and_todos() {
        let root = temp_workspace("resume-plan");
        seed_session_store(&root, "sess-plan");
        let store = skarn_ai::AgentSessionStore::new(&root);
        let mut record = store
            .load("sess-plan")
            .expect("load")
            .expect("the seeded session exists")
            .state;
        record.plan = skarn_ai::PlanState {
            status: skarn_ai::PlanStatus::Proposed,
            plan: Some(
                serde_json::to_string(&skarn_ai::PlanDraft {
                    summary: Some("ship it".into()),
                    steps: vec!["Read the router".into(), "Write the plan".into()],
                })
                .expect("draft json"),
            ),
            updated_at: 7,
        };
        store.save_state(&record).expect("save state");
        store
            .save_todos(
                "sess-plan",
                &[skarn_ai::TodoItem {
                    id: "t1".into(),
                    title: "Read the router".into(),
                    status: skarn_ai::TodoStatus::Completed,
                }],
            )
            .expect("save todos");

        let state = DesktopState::with_settings(skarn_settings::Settings::default());
        let engine = state.sessions();
        engine.set_workspace_root(Some(root.clone()));
        let session_id = SessionId::new("sess-plan").expect("id");
        engine.switch_session(&session_id).expect("switch restores");

        let chat = state.bootstrap().chat;
        let session = chat.sessions.first().expect("the session exists");
        assert_eq!(session.plan_phase, PlanPhaseDto::Planning);
        assert_eq!(session.todos.len(), 1);
        assert_eq!(session.todos[0].title, "Read the router");
        assert_eq!(session.todos[0].status, TodoStatusDto::Completed);
        let plan = session.plan.as_ref().expect("the draft hydrates");
        assert_eq!(plan.steps, vec!["Read the router", "Write the plan"]);
        assert_eq!(plan.summary.as_deref(), Some("ship it"));

        // The router's structural gate sees the restored phase.
        let planning = {
            let inner = engine.inner.lock().expect("engine lock");
            skarn_ai::plan::is_planning(
                &inner.sessions.get(&session_id).expect("session").plan_store,
            )
        };
        assert!(planning, "a proposed plan reopens as planning");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn session_auto_approve_is_off_by_default_and_resets_on_switch_and_new() {
        let root = temp_workspace("auto-scope");
        seed_session_store(&root, "sess-a");
        seed_session_store(&root, "sess-b");
        let state = DesktopState::with_settings(skarn_settings::Settings::default());
        let engine = state.sessions();
        engine.set_workspace_root(Some(root.clone()));

        let a = SessionId::new("sess-a").expect("id");
        let b = SessionId::new("sess-b").expect("id");
        engine.switch_session(&a).expect("switch a");
        let flag = |session: &SessionId| {
            engine
                .inner
                .lock()
                .expect("engine")
                .sessions
                .get(session)
                .expect("session")
                .auto_approve
        };
        assert!(!flag(&a), "off by default");
        engine.set_auto_approve(&a, true).expect("arm a");
        assert!(flag(&a));

        // Switching cannot inherit the flag, and it clears the source too.
        engine.switch_session(&b).expect("switch b");
        assert!(!flag(&a), "the source session is disarmed");
        assert!(!flag(&b), "the target never inherits trust");

        engine.set_auto_approve(&b, true).expect("arm b");
        engine.new_session().expect("new session");
        assert!(!flag(&b), "a new session resets trust");

        // Restart: the flag has no on-disk representation, so a fresh engine
        // restores the session with trust off.
        let restarted = DesktopState::with_settings(skarn_settings::Settings::default());
        restarted.sessions().set_workspace_root(Some(root.clone()));
        restarted
            .sessions()
            .switch_session(&a)
            .expect("restore after restart");
        assert!(
            !restarted
                .sessions()
                .inner
                .lock()
                .expect("engine")
                .sessions
                .get(&a)
                .expect("session")
                .auto_approve
        );
        let name = ".selora/agent-sessions/sess-a/state.json";
        let text = std::fs::read_to_string(root.join(name)).expect("state file");
        assert!(!text.contains("auto_approve"), "{name} persisted trust");
        assert!(
            !text.contains("sk-gw-desktop-test-key"),
            "{name} leaked the key"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn planning_without_a_draft_survives_restart_and_refuses_all_effects_until_approval() {
        let root = tempfile::tempdir().unwrap();
        let state = DesktopState::with_settings(skarn_settings::Settings::default());
        let engine = state.sessions();
        engine.set_workspace_root(Some(root.path().to_path_buf()));
        let id = engine.new_session().unwrap();
        let live = engine.inner.lock().unwrap().sessions[&id]
            .plan_store
            .clone();
        skarn_ai::plan::lock(&live).phase = skarn_ai::PlanPhase::Planning;
        engine.sync_plan_store(&id);
        drop(engine);
        drop(state);

        let restarted = DesktopState::with_settings(skarn_settings::Settings::default());
        let engine = restarted.sessions();
        engine.set_workspace_root(Some(root.path().to_path_buf()));
        engine.switch_session(&id).unwrap();
        let live = engine.inner.lock().unwrap().sessions[&id]
            .plan_store
            .clone();
        assert!(skarn_ai::plan::is_planning(&live));
        let context = ToolContext::new(root.path(), skarn_ai::AgentToolSettings::default())
            .with_plan(live.clone());
        tauri::async_runtime::block_on(async {
            for (name, arguments) in [
                ("write_file", r#"{"path":"never.txt","content":"no"}"#),
                ("bash", r#"{"command":"Set-Content never.txt no"}"#),
                ("run_workflow", r#"{"name":"no","script":"log('no');"}"#),
            ] {
                let call = skarn_ai::ToolCall {
                    id: name.into(),
                    name: name.into(),
                    arguments: arguments.into(),
                };
                let skarn_ai::ToolOutcome::Ready(result) =
                    skarn_ai::execute_tool(&context, call.clone()).await
                else {
                    panic!("planning must refuse before a card");
                };
                assert!(!result.ok);
                assert!(!skarn_ai::execute_approved(&context, call).await.ok);
            }
            let todos = skarn_ai::ToolCall {
                id: "todos".into(),
                name: "manage_todo".into(),
                arguments: r#"{"todos":[{"id":"t1","text":"Inspect","status":"pending"}]}"#.into(),
            };
            assert!(
                matches!(skarn_ai::execute_tool(&context, todos).await, skarn_ai::ToolOutcome::Ready(result) if result.ok)
            );
            let exit = skarn_ai::ToolCall {
                id: "exit".into(),
                name: "exit_plan_mode".into(),
                arguments: r#"{"summary":"Review","plan":["Inspect","Write"]}"#.into(),
            };
            assert!(matches!(
                skarn_ai::execute_tool(&context, exit.clone()).await,
                skarn_ai::ToolOutcome::NeedsApproval(ApprovalCard::Plan { .. })
            ));
            engine.sync_plan_store(&id);
            let restored = engine.load_session_from_disk(&id).unwrap();
            let snapshot = skarn_ai::plan::snapshot(&restored.plan_store);
            assert_eq!(snapshot.phase, skarn_ai::PlanPhase::Planning);
            assert_eq!(snapshot.todos[0].title, "Inspect");
            assert_eq!(snapshot.plan.unwrap().steps, ["Inspect", "Write"]);
            let context = ToolContext::new(root.path(), skarn_ai::AgentToolSettings::default())
                .with_plan(restored.plan_store);
            assert!(skarn_ai::execute_approved(&context, exit).await.ok);
            let write = skarn_ai::ToolCall {
                id: "write".into(),
                name: "write_file".into(),
                arguments: r#"{"path":"approved.txt","content":"yes"}"#.into(),
            };
            assert!(matches!(
                skarn_ai::execute_tool(&context, write.clone()).await,
                skarn_ai::ToolOutcome::NeedsApproval(ApprovalCard::Diff { .. })
            ));
            assert!(skarn_ai::execute_approved(&context, write).await.ok);
        });
        assert!(!root.path().join("never.txt").exists());
        assert_eq!(
            std::fs::read_to_string(root.path().join("approved.txt")).unwrap(),
            "yes"
        );
    }

    #[test]
    fn retry_rewinds_the_durable_transcript_and_resends() {
        let app = mock_app_handle();
        let root = temp_workspace("retry");
        seed_session_store(&root, "sess-retry");
        let durable = skarn_ai::AgentSessionStore::new(&root);
        let mut prefix = durable.load_transcript("sess-retry").unwrap().entries;
        let split = prefix.iter().position(|entry| matches!(entry, TranscriptEntry::User { content, .. } if content == "second prompt")).unwrap();
        prefix.splice(
            split..split,
            [
                TranscriptEntry::ToolCalls {
                    id: "batch-original".into(),
                    timestamp: 1,
                    calls: vec![
                        skarn_ai::ToolCall {
                            id: "write-original".into(),
                            name: "write_file".into(),
                            arguments: r#"{"path":"once.txt","content":"once"}"#.into(),
                        },
                        skarn_ai::ToolCall {
                            id: "bash-original".into(),
                            name: "bash".into(),
                            arguments: r#"{"command":"Add-Content once.txt duplicate"}"#.into(),
                        },
                    ],
                },
                TranscriptEntry::Tool {
                    id: "write-result".into(),
                    timestamp: 2,
                    call_id: "write-original".into(),
                    ok: true,
                    content: "written once".into(),
                },
                TranscriptEntry::Tool {
                    id: "bash-result".into(),
                    timestamp: 3,
                    call_id: "bash-original".into(),
                    ok: true,
                    content: "ran once".into(),
                },
            ],
        );
        durable.rewrite_transcript("sess-retry", &prefix).unwrap();
        std::fs::write(root.join("once.txt"), "once").unwrap();
        let server = mock_gateway(
            vec![(
                "second prompt",
                sse(&[
                    content_chunk("second answer"),
                    usage_chunk("stop", 10, 5, 0.001),
                ]),
            )],
            true,
        );
        let state = DesktopState::with_settings(gateway_settings(server.uri()));
        seed_supported_test_model(&state);
        state.sessions().set_workspace_root(Some(root.clone()));
        let session = SessionId::new("sess-retry").expect("id");
        state
            .sessions()
            .switch_session(&session)
            .expect("restore before retry");

        let chat = state.bootstrap().chat;
        let retry_target = chat
            .messages
            .iter()
            .find(|message| message.role == MessageRole::User && message.text == "second prompt")
            .expect("the second user message")
            .message_id
            .clone();

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::RetryMessage {
                        session_id: session.clone(),
                        message_id: retry_target,
                    },
                ),
            )
            .expect("retry accepted");
        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::AgentTurnDone(_))
        });

        let store = skarn_ai::AgentSessionStore::new(&root);
        let transcript = store.load_transcript("sess-retry").expect("transcript");
        let user_texts: Vec<&str> = transcript
            .entries
            .iter()
            .filter_map(|entry| match entry {
                skarn_ai::TranscriptEntry::User { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            user_texts,
            vec!["first prompt", "second prompt"],
            "the abandoned branch is dropped and the retried message re-sent"
        );
        assert!(
            transcript.entries.iter().any(|entry| matches!(
                entry,
                skarn_ai::TranscriptEntry::Assistant { content, .. }
                    if content == "first answer"
            )),
            "history before the retried message is kept"
        );
        assert!(
            !transcript.entries.iter().any(|entry| matches!(
                entry,
                skarn_ai::TranscriptEntry::Assistant { content, .. }
                    if content == "second answer old"
            )),
            "the abandoned branch is gone from the durable transcript"
        );
        assert!(transcript.entries.iter().any(|entry| matches!(entry, TranscriptEntry::ToolCalls { calls, .. } if calls.len() == 2 && calls[1].id == "bash-original")));
        assert!(transcript.entries.iter().any(|entry| matches!(entry, TranscriptEntry::Tool { call_id, .. } if call_id == "write-original")));
        let requests = tauri::async_runtime::block_on(server.received_requests()).unwrap();
        assert_eq!(requests.len(), 1, "retry sends exactly one request");
        let request: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["tool_call_id"] == "bash-original"));
        assert_eq!(
            std::fs::read_to_string(root.join("once.txt")).unwrap(),
            "once"
        );

        // An abandoned turn with attempted effects cannot be retried blindly.
        durable
            .append(
                "sess-retry",
                &TranscriptEntry::ToolCalls {
                    id: "unfinished-effect".into(),
                    timestamp: 4,
                    calls: vec![skarn_ai::ToolCall {
                        id: "unsafe-bash".into(),
                        name: "bash".into(),
                        arguments: "{}".into(),
                    }],
                },
            )
            .unwrap();
        let target = transcript
            .entries
            .iter()
            .rev()
            .find(|entry| matches!(entry, TranscriptEntry::User { .. }))
            .unwrap();
        let error = state
            .sessions()
            .retry_message(
                state.event_log(),
                app.clone(),
                None,
                session,
                MessageId::new(target.id()).unwrap(),
            )
            .unwrap_err();
        assert_eq!(error.code, "retry_partial_effects");
        assert_eq!(
            std::fs::read_to_string(root.join("once.txt")).unwrap(),
            "once"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn plan_text_is_redacted_on_approval_events_and_bootstrap() {
        const KEY: &str = "sk-gw-desktop-test-key";
        let redacted = redact_card(
            ApprovalCard::Plan {
                summary: Some(format!("use {KEY}")),
                steps: vec![format!("run {KEY}")],
            },
            KEY,
        );
        let ApprovalCard::Plan { summary, steps } = redacted else {
            panic!("redaction preserves the plan card");
        };
        assert_eq!(summary.as_deref(), Some("use [REDACTED]"));
        assert_eq!(steps, vec!["run [REDACTED]"]);

        let root = tempfile::tempdir().unwrap();
        let state = DesktopState::with_settings(gateway_settings("http://127.0.0.1:9".into()));
        state
            .sessions()
            .set_workspace_root(Some(root.path().to_path_buf()));
        let id = state.sessions().new_session().unwrap();
        let live = state.sessions().inner.lock().unwrap().sessions[&id]
            .plan_store
            .clone();
        {
            let mut plan = skarn_ai::plan::lock(&live);
            plan.phase = skarn_ai::PlanPhase::Planning;
            plan.plan = Some(skarn_ai::PlanDraft {
                summary: Some(KEY.into()),
                steps: vec![KEY.into()],
            });
        }
        let session = state
            .bootstrap()
            .chat
            .sessions
            .into_iter()
            .find(|session| session.session_id == id)
            .unwrap();
        let plan = session.plan.expect("plan hydrates");
        assert_eq!(plan.summary.as_deref(), Some("[REDACTED]"));
        assert_eq!(plan.steps, vec!["[REDACTED]"]);
    }

    #[test]
    fn durable_store_fails_closed_and_remembers_rotated_keys() {
        let root = temp_workspace("rotated-redaction");
        let key = Arc::new(Mutex::new(Ok(Some("old-secret".to_owned()))));
        let engine = SessionEngine::new(EngineConfig {
            base_url: "http://127.0.0.1:1".into(),
            active_model: None,
            agent: skarn_settings::AgentSettings::default(),
            key_source: ApiKeySource::Mutable(Arc::clone(&key)),
        });
        engine.set_workspace_root(Some(root.clone()));
        engine.fetch_api_key().unwrap();
        *key.lock().unwrap() = Err(ProtocolErrorDto {
            code: "keyring_unavailable".into(),
            message: "error".into(),
            retryable: true,
        });
        assert!(engine.store().is_none());
        assert!(!root.join(".selora/agent-sessions").exists());
        *key.lock().unwrap() = Ok(Some("new-secret".into()));
        let store = engine.store().unwrap();
        store
            .create(
                "session-1",
                "old-secret new-secret",
                None,
                AgentSessionMode::Execute,
            )
            .unwrap();
        let state =
            std::fs::read_to_string(root.join(".selora/agent-sessions/session-1/state.json"))
                .unwrap();
        assert!(!state.contains("old-secret") && !state.contains("new-secret"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn keyring_failure_queues_tool_records_and_retries_in_order() {
        let root = temp_workspace("queued-tool-records");
        let key = Arc::new(Mutex::new(Ok(Some("secret".to_owned()))));
        let engine = SessionEngine::new(EngineConfig {
            base_url: "http://127.0.0.1:1".into(),
            active_model: None,
            agent: skarn_settings::AgentSettings::default(),
            key_source: ApiKeySource::Mutable(Arc::clone(&key)),
        });
        engine.set_workspace_root(Some(root.clone()));
        engine.fetch_api_key().unwrap();
        let session_id = engine.new_session().unwrap();
        let store = AgentSessionStore::new(&root);
        *key.lock().unwrap() = Err(ProtocolErrorDto {
            code: "keyring_unavailable".into(),
            message: "error".into(),
            retryable: true,
        });
        let call = skarn_ai::ToolCall {
            id: "call-1".into(),
            name: "read_file".into(),
            arguments: "{}".into(),
        };
        // A transient failure between the turn's key check and the record
        // queues the data; it is never dropped and never fails the turn.
        assert_eq!(
            engine.record_tool_calls(&session_id, vec![call]),
            Err(TranscriptAppend::Queued)
        );
        assert_eq!(
            engine.record_tool_result(&session_id, &skarn_ai::ToolResult::success("call-1", "ok")),
            Err(TranscriptAppend::Queued)
        );
        assert_eq!(engine.pending_transcript.lock().unwrap().len(), 2);
        assert!(store
            .load_transcript(session_id.as_str())
            .unwrap()
            .entries
            .is_empty());
        // The key comes back: the retry writes both records, call before result.
        *key.lock().unwrap() = Ok(Some("secret".to_owned()));
        engine.persist_pending_transcript().unwrap();
        assert!(engine.pending_transcript.lock().unwrap().is_empty());
        let entries = store.load_transcript(session_id.as_str()).unwrap().entries;
        let order: Vec<&str> = entries
            .iter()
            .map(|entry| match entry {
                TranscriptEntry::ToolCalls { .. } => "call",
                TranscriptEntry::Tool { .. } => "result",
                _ => "other",
            })
            .collect();
        assert_eq!(order, vec!["call", "result"]);
        let transcript = std::fs::read_to_string(
            root.join(".selora/agent-sessions")
                .join(session_id.as_str())
                .join("transcript.jsonl"),
        )
        .unwrap();
        assert!(!transcript.contains("secret"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn session_state_on_disk_never_contains_the_api_key() {
        let root = temp_workspace("no-key");
        let state = DesktopState::with_settings(skarn_settings::Settings::default());
        state.sessions().set_workspace_root(Some(root.clone()));
        let session = state.sessions().new_session().expect("new session");
        // The engine writes the state through its redacting store; even a
        // title that echoed the key must not survive.
        let engine = state.sessions();
        {
            let mut inner = engine.inner.lock().expect("engine");
            if let Some(open) = inner.sessions.get_mut(&session) {
                open.title = "sk-gw-desktop-test-key".into();
            }
        }
        engine.record_compaction(
            &session,
            skarn_ai::CompactionCursor {
                compacted_at: 1,
                ..Default::default()
            },
        );
        let dir = root.join(".selora/agent-sessions").join(session.as_str());
        let state_json = std::fs::read_to_string(dir.join("state.json")).expect("state");
        assert!(!state_json.contains("sk-gw-desktop-test-key"));
        assert!(state_json.contains("[REDACTED]"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn phase_9e_background_completion_emits_one_typed_event() {
        let state = DesktopState::with_settings(skarn_settings::Settings::default());
        let log = Arc::new(EventLog::new(state.core_instance_id()));
        let app = mock_app_handle();
        let engine = Arc::new(SessionEngine::new(EngineConfig {
            base_url: "http://127.0.0.1:1".into(),
            active_model: None,
            agent: skarn_settings::AgentSettings::default(),
            key_source: ApiKeySource::Fixed(None),
        }));

        let spawn = tauri::async_runtime::block_on(async {
            engine.spawn_background_subagent(
                Arc::clone(&log),
                app,
                skarn_ai::SpawnRequest::new(
                    "session-1",
                    "child-desktop-1",
                    skarn_ai::SubagentPolicy::ReadOnly,
                    "review",
                ),
                async {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    skarn_ai::ChildOutcome::success("review complete")
                },
            )
        });
        spawn.expect("spawn");

        let deadline = Instant::now() + TEST_TIMEOUT;
        let events = loop {
            let events = log.events_after(0);
            if events
                .iter()
                .any(|event| matches!(event.payload, FrontendEvent::SubagentCompleted(_)))
                || Instant::now() >= deadline
            {
                break events;
            }
            tauri::async_runtime::block_on(async {
                tokio::time::sleep(Duration::from_millis(5)).await;
            });
        };

        let completions: Vec<_> = events
            .into_iter()
            .filter_map(|event| match event.payload {
                FrontendEvent::SubagentCompleted(completion) => Some(completion),
                _ => None,
            })
            .collect();
        assert_eq!(completions.len(), 1, "one event, never a duplicate");
        let completion = &completions[0];
        assert_eq!(completion.child_id, "child-desktop-1");
        assert_eq!(
            completion.parent_session_id,
            SessionId::new("session-1").unwrap()
        );
        assert!(completion.ok);
        assert_eq!(completion.final_message, "review complete");
    }

    #[test]
    fn subagent_delta_buffer_redacts_split_keys_and_flushes_prefixes() {
        let key = "sëcret";
        for split in key
            .char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(key.len()))
        {
            let mut buffer = SubagentDeltaBuffer::default();
            let mut text = buffer.push(&format!("before {}", &key[..split]), key, false);
            text.push_str(&buffer.push(&format!("{} after së", &key[split..]), key, false));
            text.push_str(&buffer.push("", key, true));
            assert_eq!(text, "before [REDACTED] after së");
        }
        let mut buffer = SubagentDeltaBuffer::default();
        assert_eq!(buffer.push("sësëcret", key, false), "së[REDACTED]");
        assert_eq!(buffer.push("plain", "", true), "plain");
        let mut buffer = SubagentDeltaBuffer::default();
        assert_eq!(buffer.push("", "abc", false), "");
        assert_eq!(buffer.push("abc", "abcdef", false), "");
        assert_eq!(buffer.push("def", "abcdef", false), "[REDACTED]");
        assert_eq!(
            redact_with_keys("abcdef", &["abc".into(), "abcdef".into()]),
            "[REDACTED]"
        );
    }

    #[test]
    fn tool_json_redacts_escaped_credentials_in_keys_and_values() {
        let secret = "a\"b\\c";
        let input = serde_json::json!({(secret): secret}).to_string();
        let output = redact_json_text(&input, &[secret.into()]);
        let decoded: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(decoded, serde_json::json!({"[REDACTED]": "[REDACTED]"}));
    }

    #[test]
    fn subagent_activity_redacts_tool_identifiers_with_all_keys() {
        let keys = vec!["old-key".into(), "new-key".into()];
        for kind in [
            skarn_ai::SubagentActivityKind::ToolCallStarted {
                call_id: "old-key".into(),
                tool_name: "new-key".into(),
                args_json: "old-key".into(),
            },
            skarn_ai::SubagentActivityKind::ToolResult {
                call_id: "old-key".into(),
                result_summary: "new-key".into(),
                is_err: false,
            },
            skarn_ai::SubagentActivityKind::StateChanged {
                state: skarn_ai::SubagentRunState::RunningTool,
                tool_name: Some("new-key".into()),
            },
        ] {
            let dto = subagent_activity_dto(
                skarn_ai::SubagentActivity::new("child", "session-1", kind),
                &keys,
            )
            .unwrap();
            let json = serde_json::to_string(&dto).unwrap();
            assert!(
                !json.contains("old-key") && !json.contains("new-key"),
                "{json}"
            );
            assert!(json.contains("[REDACTED]"));
        }
    }

    #[test]
    fn subagent_approval_redacts_overlapping_keys_in_one_pass() {
        let card = ApprovalCard::Plan {
            summary: Some("abcdef abc".into()),
            steps: vec!["abcdef".into()],
        };
        let card = redact_card_with_keys(card, &["abc".into(), "abcdef".into()]);
        let ApprovalCard::Plan { summary, steps } = card else {
            panic!("expected plan")
        };
        assert_eq!(summary.as_deref(), Some("[REDACTED] [REDACTED]"));
        assert_eq!(steps, vec!["[REDACTED]"]);
    }

    #[test]
    fn overflow_cancellation_is_relayed_after_queue_drains() {
        let state = DesktopState::with_settings(skarn_settings::Settings::default());
        let log = Arc::new(EventLog::new(state.core_instance_id()));
        let engine = Arc::new(SessionEngine::new(EngineConfig {
            base_url: "http://127.0.0.1:1".into(),
            active_model: None,
            agent: skarn_settings::AgentSettings::default(),
            key_source: ApiKeySource::Fixed(Some("secret".into())),
        }));
        engine
            .subagents
            .register(skarn_ai::SpawnRequest::new(
                "session-1",
                "child-1",
                skarn_ai::SubagentPolicy::ReadOnly,
                "mission",
            ))
            .unwrap();
        for sequence in 1..=257 {
            engine
                .subagents
                .emit_activity(skarn_ai::SubagentActivity::new(
                    "child-1",
                    "session-1",
                    skarn_ai::SubagentActivityKind::Delta {
                        message_id: "message".into(),
                        sequence,
                        text: "text ".into(),
                    },
                ));
        }
        assert_eq!(
            engine.subagents.child_state("child-1"),
            Some(skarn_ai::ChildState::Cancelled)
        );
        let rx = engine.subagent_events.lock().unwrap().take().unwrap();
        let task = tauri::async_runtime::spawn(subagent_relay(SubagentRelayCtx {
            log: Arc::clone(&log),
            engine,
            app: mock_app_handle(),
            events_rx: rx,
        }));
        let deadline = Instant::now() + TEST_TIMEOUT;
        while !log
            .events_after(0)
            .iter()
            .any(|event| matches!(event.payload, FrontendEvent::SubagentCancelled(_)))
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        let events = log.events_after(0);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event.payload, FrontendEvent::SubagentCancelled(_)))
                .count(),
            1
        );
        task.abort();
    }

    #[test]
    fn keyring_failure_cancels_parked_subagent_approval() {
        let state = DesktopState::with_settings(skarn_settings::Settings::default());
        let log = Arc::new(EventLog::new(state.core_instance_id()));
        let source = Arc::new(Mutex::new(Ok(Some("secret".into()))));
        let engine = Arc::new(SessionEngine::new(EngineConfig {
            base_url: "http://127.0.0.1:1".into(),
            active_model: None,
            agent: skarn_settings::AgentSettings::default(),
            key_source: ApiKeySource::Mutable(Arc::clone(&source)),
        }));
        engine
            .subagents
            .register(skarn_ai::SpawnRequest::new(
                "session-1",
                "child-1",
                skarn_ai::SubagentPolicy::Full,
                "mission",
            ))
            .unwrap();
        let approval_id = Uuid::new_v4();
        let (decisions, _decision_rx) = mpsc::unbounded_channel();
        engine
            .subagents
            .register_decision("child-1", approval_id, decisions);
        *source.lock().unwrap() = Err(ProtocolErrorDto {
            code: "keyring_unavailable".into(),
            message: "error".into(),
            retryable: true,
        });
        let (tx, rx) = mpsc::channel(1);
        tx.try_send(skarn_ai::SubagentEvent::ApprovalNeeded(
            skarn_ai::SubagentCard {
                child_id: "child-1".into(),
                parent_session: "session-1".into(),
                approval_id,
                card: ApprovalCard::Command {
                    command: "secret".into(),
                },
            },
        ))
        .unwrap();
        drop(tx);
        tauri::async_runtime::block_on(subagent_relay(SubagentRelayCtx {
            log: Arc::clone(&log),
            engine: Arc::clone(&engine),
            app: mock_app_handle(),
            events_rx: rx,
        }));
        assert!(log.events_after(0).is_empty());
        assert!(engine
            .subagents
            .decide(
                approval_id,
                (approval_id, skarn_ai::AgentDecision::Approve)
            )
            .is_err());
    }

    #[test]
    fn subagent_relay_fails_closed_without_key() {
        let state = DesktopState::with_settings(skarn_settings::Settings::default());
        let log = Arc::new(EventLog::new(state.core_instance_id()));
        let key = Arc::new(Mutex::new(Ok(None)));
        let engine = Arc::new(SessionEngine::new(EngineConfig {
            base_url: "http://127.0.0.1:1".into(),
            active_model: None,
            agent: skarn_settings::AgentSettings::default(),
            key_source: ApiKeySource::Mutable(Arc::clone(&key)),
        }));
        let (tx, rx) = mpsc::channel(256);
        tx.try_send(skarn_ai::SubagentEvent::Activity(
            skarn_ai::SubagentActivity::new(
                "child",
                "session-1",
                skarn_ai::SubagentActivityKind::Delta {
                    message_id: "m".into(),
                    sequence: 1,
                    text: "sensitive".into(),
                },
            ),
        ))
        .unwrap();
        drop(tx);
        let task = tauri::async_runtime::spawn(subagent_relay(SubagentRelayCtx {
            log: Arc::clone(&log),
            engine,
            app: mock_app_handle(),
            events_rx: rx,
        }));
        std::thread::sleep(Duration::from_millis(250));
        assert!(log.events_after(0).is_empty());
        tauri::async_runtime::block_on(task).unwrap();
        assert!(log.events_after(0).is_empty());
    }

    #[test]
    fn subagent_relay_refreshes_rotated_key_and_recovers_from_keyring_error() {
        let state = DesktopState::with_settings(skarn_settings::Settings::default());
        let log = Arc::new(EventLog::new(state.core_instance_id()));
        let key = Arc::new(Mutex::new(Ok(Some("old-key".to_string()))));
        let engine = Arc::new(SessionEngine::new(EngineConfig {
            base_url: "http://127.0.0.1:1".into(),
            active_model: None,
            agent: skarn_settings::AgentSettings::default(),
            key_source: ApiKeySource::Mutable(Arc::clone(&key)),
        }));
        engine.register_subagent_redaction_key("original-key");
        let (tx, rx) = mpsc::channel(256);
        let task = tauri::async_runtime::spawn(subagent_relay(SubagentRelayCtx {
            log: Arc::clone(&log),
            engine,
            app: mock_app_handle(),
            events_rx: rx,
        }));
        let activity = |text: &str, sequence| {
            skarn_ai::SubagentEvent::Activity(skarn_ai::SubagentActivity::new(
                "child",
                "session-1",
                skarn_ai::SubagentActivityKind::Delta {
                    message_id: "m".into(),
                    sequence,
                    text: text.into(),
                },
            ))
        };
        tx.try_send(activity("original-key old-key ", 1)).unwrap();
        let deadline = Instant::now() + TEST_TIMEOUT;
        while log.events_after(0).is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(log.events_after(0).len(), 1);
        assert!(
            matches!(&log.events_after(0)[0].payload, FrontendEvent::SubagentActivity(SubagentActivityDto::Delta { text, .. }) if text == "[REDACTED] [REDACTED] ")
        );
        *key.lock().unwrap() = Ok(Some("new-key".into()));
        tx.try_send(activity("new-key old-key ", 2)).unwrap();
        while log.events_after(0).len() < 2 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(log.events_after(0).len(), 2);
        *key.lock().unwrap() = Err(ProtocolErrorDto {
            code: "keyring_unavailable".into(),
            message: "error".into(),
            retryable: true,
        });
        tx.try_send(activity("new-key recovered", 3)).unwrap();
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(log.events_after(0).len(), 2);
        *key.lock().unwrap() = Ok(Some("new-key".into()));
        tx.try_send(activity("new-key recovered", 4)).unwrap();
        drop(tx);
        tauri::async_runtime::block_on(task).unwrap();
        let events = log.events_after(0);
        assert_eq!(events.len(), 3);
        assert!(
            matches!(&events[1].payload, FrontendEvent::SubagentActivity(SubagentActivityDto::Delta { text, .. }) if text == "[REDACTED] [REDACTED] ")
        );
        assert!(
            matches!(&events[2].payload, FrontendEvent::SubagentActivity(SubagentActivityDto::Delta { text, .. }) if text == "[REDACTED] recovered")
        );
    }

    #[test]
    fn subagent_relay_keeps_prefix_across_ordinary_activity() {
        let state = DesktopState::with_settings(skarn_settings::Settings::default());
        let log = Arc::new(EventLog::new(state.core_instance_id()));
        let engine = Arc::new(SessionEngine::new(EngineConfig {
            base_url: "http://127.0.0.1:1".into(),
            active_model: None,
            agent: skarn_settings::AgentSettings::default(),
            key_source: ApiKeySource::Fixed(Some("secret".into())),
        }));
        let (tx, rx) = mpsc::channel(256);
        for kind in [
            skarn_ai::SubagentActivityKind::Delta {
                message_id: "m".into(),
                sequence: 1,
                text: "pre sec".into(),
            },
            skarn_ai::SubagentActivityKind::StateChanged {
                state: skarn_ai::SubagentRunState::Thinking,
                tool_name: None,
            },
            skarn_ai::SubagentActivityKind::Delta {
                message_id: "m".into(),
                sequence: 2,
                text: "ret post".into(),
            },
        ] {
            tx.try_send(skarn_ai::SubagentEvent::Activity(
                skarn_ai::SubagentActivity::new("child", "session-1", kind),
            ))
            .unwrap();
        }
        drop(tx);
        tauri::async_runtime::block_on(subagent_relay(SubagentRelayCtx {
            log: Arc::clone(&log),
            engine,
            app: mock_app_handle(),
            events_rx: rx,
        }));
        let events = log.events_after(0);
        let deltas: Vec<(u64, String)> = events
            .iter()
            .filter_map(|event| match &event.payload {
                FrontendEvent::SubagentActivity(SubagentActivityDto::Delta {
                    sequence,
                    text,
                    ..
                }) => Some((*sequence, text.clone())),
                _ => None,
            })
            .collect();
        // The ordinary state event did not seal the "sec" prefix; the stream
        // resumed and completed the key once.
        assert_eq!(
            deltas,
            vec![(1, "pre ".to_string()), (2, "[REDACTED] post".to_string())]
        );
        let state_event = events
            .iter()
            .position(|event| {
                matches!(
                    &event.payload,
                    FrontendEvent::SubagentActivity(SubagentActivityDto::StateChanged { .. })
                )
            })
            .expect("state event relayed");
        assert!(
            !events[..state_event].iter().any(|event| matches!(
                &event.payload,
                FrontendEvent::SubagentActivity(SubagentActivityDto::Delta { text, .. })
                    if text.contains("[REDACTED]")
            )),
            "a key prefix must not be replaced before the stream resolves"
        );
    }

    #[test]
    fn subagent_relay_flushes_split_deltas_on_cancel_and_close() {
        const KEY: &str = "sk-secret";
        let state = DesktopState::with_settings(skarn_settings::Settings::default());
        let log = Arc::new(EventLog::new(state.core_instance_id()));
        let engine = Arc::new(SessionEngine::new(EngineConfig {
            base_url: "http://127.0.0.1:1".into(),
            active_model: None,
            agent: skarn_settings::AgentSettings::default(),
            key_source: ApiKeySource::Fixed(Some(KEY.into())),
        }));
        let (tx, rx) = mpsc::channel(256);
        let task = tauri::async_runtime::spawn(subagent_relay(SubagentRelayCtx {
            log: Arc::clone(&log),
            engine,
            app: mock_app_handle(),
            events_rx: rx,
        }));
        for (child, message, sequence, text) in [
            ("child-1", "m1", 1, "hello sk-"),
            ("child-1", "m1", 2, "secret tail sk-"),
        ] {
            tx.try_send(skarn_ai::SubagentEvent::Activity(
                skarn_ai::SubagentActivity::new(
                    child,
                    "session-1",
                    skarn_ai::SubagentActivityKind::Delta {
                        message_id: message.into(),
                        sequence,
                        text: text.into(),
                    },
                ),
            ))
            .unwrap();
        }
        tx.try_send(skarn_ai::SubagentEvent::Cancelled {
            child_id: "child-1".into(),
            parent_session: "session-1".into(),
        })
        .unwrap();
        tx.try_send(skarn_ai::SubagentEvent::Activity(
            skarn_ai::SubagentActivity::new(
                "child-2",
                "session-1",
                skarn_ai::SubagentActivityKind::Delta {
                    message_id: "m2".into(),
                    sequence: 1,
                    text: "sk-".into(),
                },
            ),
        ))
        .unwrap();
        drop(tx);
        tauri::async_runtime::block_on(task).unwrap();
        let events = log.events_after(0);
        let mut children = BTreeMap::<String, String>::new();
        let mut sequences = BTreeMap::<String, Vec<u64>>::new();
        for event in &events {
            if let FrontendEvent::SubagentActivity(SubagentActivityDto::Delta {
                child_id,
                text,
                sequence,
                ..
            }) = &event.payload
            {
                children.entry(child_id.clone()).or_default().push_str(text);
                sequences
                    .entry(child_id.clone())
                    .or_default()
                    .push(*sequence);
            }
        }
        assert_eq!(children["child-1"], "hello [REDACTED] tail sk-");
        assert_eq!(children["child-2"], "sk-");
        assert_eq!(sequences["child-1"], vec![1, 2, 3]);
        let cancel = events
            .iter()
            .position(|e| matches!(e.payload, FrontendEvent::SubagentCancelled(_)))
            .unwrap();
        assert!(events[..cancel].iter().any(|e| matches!(&e.payload, FrontendEvent::SubagentActivity(SubagentActivityDto::Delta { child_id, text, .. }) if child_id == "child-1" && text == "sk-")));
    }

    #[test]
    fn subagent_activity_relay_keeps_transcripts_separate_and_redacts() {
        const KEY: &str = "sk-secret-desktop-key";
        let state = DesktopState::with_settings(skarn_settings::Settings::default());
        let log = Arc::new(EventLog::new(state.core_instance_id()));
        let app = mock_app_handle();
        let engine = Arc::new(SessionEngine::new(EngineConfig {
            base_url: "http://127.0.0.1:1".into(),
            active_model: None,
            agent: skarn_settings::AgentSettings::default(),
            key_source: ApiKeySource::Fixed(Some(KEY.into())),
        }));
        let (tx, rx) = mpsc::channel(256);
        tauri::async_runtime::spawn(subagent_relay(SubagentRelayCtx {
            log: Arc::clone(&log),
            engine: Arc::clone(&engine),
            app,
            events_rx: rx,
        }));
        tx.try_send(skarn_ai::SubagentEvent::Activity(
            skarn_ai::SubagentActivity::new(
                "child-1",
                "session-1",
                skarn_ai::SubagentActivityKind::Started {
                    name: format!("{KEY} reviewer"),
                    policy: skarn_ai::SubagentPolicy::ReadOnly,
                    mission: format!("review {KEY}"),
                },
            ),
        ))
        .unwrap();
        tx.try_send(skarn_ai::SubagentEvent::Activity(
            skarn_ai::SubagentActivity::new(
                "child-1",
                "session-1",
                skarn_ai::SubagentActivityKind::Delta {
                    message_id: "message-1".into(),
                    sequence: 1,
                    text: format!("token {KEY}"),
                },
            ),
        ))
        .unwrap();
        tx.try_send(skarn_ai::SubagentEvent::Cancelled {
            child_id: "child-1".into(),
            parent_session: "session-1".into(),
        })
        .unwrap();
        tx.try_send(skarn_ai::SubagentEvent::ApprovalNeeded(
            skarn_ai::SubagentCard {
                child_id: "child-1".into(),
                parent_session: "session-1".into(),
                approval_id: uuid::Uuid::new_v4(),
                card: ApprovalCard::Plan {
                    summary: None,
                    steps: vec!["Inspect".into()],
                },
            },
        ))
        .unwrap();

        let deadline = Instant::now() + TEST_TIMEOUT;
        let events = loop {
            let events = log.events_after(0);
            if events.len() >= 4 || Instant::now() >= deadline {
                break events;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(events.len(), 4, "{events:#?}");
        let parent = SessionId::new("session-1").unwrap();
        let child = RequestId::new("child-1").unwrap();
        for event in &events {
            assert_eq!(event.session_id.as_ref(), Some(&parent));
        }
        let FrontendEvent::SubagentActivity(SubagentActivityDto::Started {
            child_id,
            parent_session_id,
            name,
            policy,
            mission,
        }) = &events[0].payload
        else {
            panic!("first event must be Started: {events:#?}");
        };
        assert_eq!(child_id, "child-1");
        assert_eq!(parent_session_id, &parent);
        assert_eq!(name.as_deref(), Some("[REDACTED] reviewer"));
        assert_eq!(*policy, SubagentPolicyDto::ReadOnly);
        assert_eq!(mission, "review [REDACTED]");
        assert_eq!(events[0].request_id.as_ref(), Some(&child));
        let FrontendEvent::SubagentActivity(SubagentActivityDto::Delta {
            message_id,
            sequence,
            text,
            ..
        }) = &events[1].payload
        else {
            panic!("second event must be Delta: {events:#?}");
        };
        assert_eq!(message_id, "message-1");
        assert_eq!(*sequence, 1);
        assert_eq!(text, "token [REDACTED]");
        assert_eq!(events[1].request_id.as_ref(), Some(&child));
        let FrontendEvent::SubagentCancelled(cancelled) = &events[2].payload else {
            panic!("third event must be Cancelled: {events:#?}");
        };
        assert_eq!(cancelled.child_id, "child-1");
        assert_eq!(cancelled.parent_session_id, parent);
        assert_eq!(events[2].request_id.as_ref(), Some(&child));
        // The existing child approval rail is unchanged: same parent session,
        // same request routing, and the child id rides `subagent_id`.
        let FrontendEvent::ApprovalNeeded(card) = &events[3].payload else {
            panic!("fourth event must be ApprovalNeeded: {events:#?}");
        };
        assert_eq!(card.subagent_id.as_deref(), Some("child-1"));
        assert_eq!(card.session_id, parent);
        assert_eq!(card.request_id, RequestId::new("subagent-child-1").unwrap());
    }

    #[tokio::test]
    async fn agent_gates_tool_capability_caches_probe_and_keeps_plain_chat_usable() {
        use wiremock::{
            matchers::{body_partial_json, method, path},
            Mock, MockServer, ResponseTemplate,
        };

        let server = MockServer::start().await;
        // Unknown model probe succeeds once (non-streaming, max_tokens: 160), then agent streams.
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(serde_json::json!({
                "model": "unknown/probed-ok",
                "stream": false,
                "max_tokens": 160
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "call_probe",
                            "type": "function",
                            "function": { "name": "skarn_probe_echo", "arguments": "{\"value\":\"ok\"}" }
                        }]
                    }
                }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let done_sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"done\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2}}\n\n",
            "data: [DONE]\n\n"
        );
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(serde_json::json!({ "stream": true })))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(done_sse),
            )
            .mount(&server)
            .await;

        let unknown_plain = server.uri();
        let workspace = tempfile::tempdir().expect("workspace");
        let engine = Arc::new(SessionEngine::new(EngineConfig {
            base_url: format!("{}/v1", unknown_plain),
            active_model: Some("chat/only".into()),
            agent: Default::default(),
            key_source: ApiKeySource::Fixed(Some("nvapi-secret-key".into())),
        }));
        engine.set_workspace_root(Some(workspace.path().to_path_buf()));
        engine.set_models_and_connection(
            vec![
                Model {
                    id: "chat/only".into(),
                    display_name: "Chat Only".into(),
                    context_window: 32_000,
                    price_per_1k_input: 0.0,
                    price_per_1k_output: 0.0,
                    supports_streaming: true,
                    tool_capability: ToolCapability::Unsupported,
                    reasoning_capability: skarn_ai::ReasoningCapability::Unknown,
                    available: true,
                    family: String::new(),
                },
                Model {
                    id: "unknown/probed-ok".into(),
                    display_name: "Unknown Ok".into(),
                    context_window: 32_000,
                    price_per_1k_input: 0.0,
                    price_per_1k_output: 0.0,
                    supports_streaming: true,
                    tool_capability: ToolCapability::Unknown,
                    reasoning_capability: skarn_ai::ReasoningCapability::Unknown,
                    available: true,
                    family: String::new(),
                },
                Model {
                    id: "meta/muse-glimmer-30b".into(),
                    display_name: "Muse Glimmer 30B".into(),
                    context_window: 32_000,
                    price_per_1k_input: 0.0,
                    price_per_1k_output: 0.0,
                    supports_streaming: true,
                    tool_capability: ToolCapability::Unsupported,
                    reasoning_capability: skarn_ai::ReasoningCapability::Unknown,
                    available: false,
                    family: String::new(),
                },
            ],
            ConnectionStatus::Connected,
            Some("chat/only".into()),
        );

        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let log = Arc::new(EventLog::new(
            skarn_protocol::CoreInstanceId::new("core-sess-gate").unwrap(),
        ));
        let session_id = engine.new_session().expect("create session");

        // 1. Agent turn on Unsupported model is rejected locally without touching gateway.
        let agent_err = engine
            .send_agent(
                Arc::clone(&log),
                handle.clone(),
                Some(session_id.clone()),
                RequestId::new("req-unsup").unwrap(),
                skarn_protocol::AgentRequestDto {
                    session_id: session_id.clone(),
                    prompt: "edit".into(),
                    mode: skarn_protocol::AgentModeDto::AskBeforeChanges,
                    model_id: Some("chat/only".into()),
                    reasoning_effort: None,
                    system_instructions: None,
                    temperature: None,
                    editor_context: None,
                    attachments: Vec::new(),
                },
            )
            .expect_err("unsupported model blocked for agent");
        assert_eq!(agent_err.code, "model_capability_unsupported");
        assert_eq!(server.received_requests().await.unwrap().len(), 0);

        // 2. Plain chat remains usable on Unsupported model without running any tool probe.
        engine
            .send_chat(
                Arc::clone(&log),
                handle.clone(),
                Some(session_id.clone()),
                RequestId::new("req-chat").unwrap(),
                skarn_protocol::ChatRequestDto {
                    session_id: session_id.clone(),
                    message: "hello".into(),
                    model_id: Some("chat/only".into()),
                },
            )
            .expect("plain chat succeeds on unsupported model");
        let deadline = Instant::now() + TEST_TIMEOUT;
        while Instant::now() < deadline {
            if server
                .received_requests()
                .await
                .map(|r| !r.is_empty())
                .unwrap_or(false)
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        // 3. Retired/unavailable model fails cleanly before any request.
        let retired_err = engine
            .send_agent(
                Arc::clone(&log),
                handle.clone(),
                Some(session_id.clone()),
                RequestId::new("req-retired").unwrap(),
                skarn_protocol::AgentRequestDto {
                    session_id: session_id.clone(),
                    prompt: "hi".into(),
                    mode: skarn_protocol::AgentModeDto::AskBeforeChanges,
                    model_id: Some("meta/muse-glimmer-30b".into()),
                    reasoning_effort: None,
                    system_instructions: None,
                    temperature: None,
                    editor_context: None,
                    attachments: Vec::new(),
                },
            )
            .expect_err("retired model unavailable");
        assert_eq!(retired_err.code, "model_unavailable");

        // 4. Unknown model probes ONCE (wiremock expect(1) on probe!) and caches for second turn.
        for idx in 0..2 {
            engine
                .send_agent(
                    Arc::clone(&log),
                    handle.clone(),
                    Some(session_id.clone()),
                    RequestId::new(format!("req-probe-{idx}")).unwrap(),
                    skarn_protocol::AgentRequestDto {
                        session_id: session_id.clone(),
                        prompt: format!("run {idx}"),
                        mode: skarn_protocol::AgentModeDto::AskBeforeChanges,
                        model_id: Some("unknown/probed-ok".into()),
                        reasoning_effort: None,
                        system_instructions: None,
                        temperature: None,
                        editor_context: None,
                        attachments: Vec::new(),
                    },
                )
                .expect("probed/cached unknown model dispatches");
            // Wait for streaming request to finish before starting second turn.
            let turn_deadline = Instant::now() + TEST_TIMEOUT;
            while Instant::now() < turn_deadline {
                if !engine
                    .live_sessions()
                    .iter()
                    .any(|s| s.active_request_id.is_some())
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(15));
            }
        }
    }

    // ------------------------------------------------------------------
    // Stage 3 Integration Tests (Section D invariants & end-to-end)
    // ------------------------------------------------------------------

    fn temp_settings_file(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "skarn-s3-settings-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("settings dir");
        dir.join("settings.json")
    }

    fn write_settings_file(path: &Path, settings: &skarn_settings::Settings) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("settings parent");
        }
        settings.save_to(path).expect("save isolated settings");
    }

    fn desktop_with_settings_path(settings_path: PathBuf) -> DesktopState {
        let state = DesktopState::from_settings_path_with_mutable_key(
            settings_path,
            Arc::new(Mutex::new(Ok(Some("sk-gw-desktop-test-key".into())))),
        );
        seed_supported_test_model(&state);
        state
    }

    #[test]
    fn agent_mutation_rejects_dirty_buffer_and_refreshes_clean_buffer() {
        let app = mock_app_handle();
        let root = temp_workspace("s3-dirty-refresh");
        std::fs::write(root.join("dirty.txt"), "dirty-before\n").expect("seed dirty");
        std::fs::write(root.join("clean.txt"), "clean-before\n").expect("seed clean");

        let rounds = vec![
            (
                "write-clean-2",
                answer_round_sse("dirty rejected and clean refreshed", 0.001),
            ),
            (
                "write-dirty-1",
                tool_round_sse(
                    &[(
                        "write-clean-2",
                        "write_file",
                        r#"{"path":"clean.txt","content":"clean-after\n"}"#,
                    )],
                    "writing clean",
                    0.001,
                ),
            ),
            (
                "dirty and clean test",
                tool_round_sse(
                    &[(
                        "write-dirty-1",
                        "write_file",
                        r#"{"path":"dirty.txt","content":"overwritten\n"}"#,
                    )],
                    "writing dirty",
                    0.001,
                ),
            ),
        ];
        let server = mock_gateway(rounds, true);
        let state = DesktopState::with_settings(gateway_settings(server.uri()));
        seed_supported_test_model(&state);

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::OpenFolderAt {
                        root: root.to_string_lossy().into_owned(),
                    },
                ),
            )
            .expect("open folder");

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::OpenFile {
                        path: skarn_protocol::WorkspacePath::new("dirty.txt").unwrap(),
                    },
                ),
            )
            .expect("open dirty.txt");
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::OpenFile {
                        path: skarn_protocol::WorkspacePath::new("clean.txt").unwrap(),
                    },
                ),
            )
            .expect("open clean.txt");

        let boot = state.bootstrap();
        let dirty_buf = boot
            .buffers
            .iter()
            .find(|b| b.path.as_str() == "dirty.txt")
            .expect("dirty buffer open")
            .clone();
        let clean_buf = boot
            .buffers
            .iter()
            .find(|b| b.path.as_str() == "clean.txt")
            .expect("clean buffer open")
            .clone();

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::ApplyEditorChange {
                        buffer_id: dirty_buf.buffer_id,
                        version: dirty_buf.version,
                        changes: vec![skarn_protocol::TextChange {
                            start: TextPosition {
                                line: 0,
                                utf16_column: 0,
                            },
                            end: TextPosition {
                                line: 0,
                                utf16_column: 0,
                            },
                            replacement: "unsaved-".into(),
                        }],
                    },
                ),
            )
            .expect("dirty edit");

        let mut command = send_agent("req-dirty-refresh", "sess-dirty", "dirty and clean test");
        if let FrontendCommand::SendAgent { request, .. } = &mut command {
            request.mode = AgentModeDto::AutoEdit;
        }
        state
            .dispatch(&app, envelope(&state, command))
            .expect("send_agent");

        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::AgentTurnDone(_))
        });

        let dirty_result = events
            .iter()
            .find_map(|event| match &event.payload {
                FrontendEvent::ToolResult(res) if res.call_id == "write-dirty-1" => Some(res),
                _ => None,
            })
            .expect("tool result for dirty write");
        assert!(dirty_result.is_err, "dirty buffer must reject agent mutation");
        assert!(
            dirty_result.result_summary.contains("OpenBufferDirty")
                || dirty_result.result_summary.to_lowercase().contains("dirty"),
            "error should report OpenBufferDirty: {}",
            dirty_result.result_summary
        );
        assert_eq!(
            std::fs::read_to_string(root.join("dirty.txt")).expect("read dirty.txt"),
            "dirty-before\n",
            "dirty target on disk must remain untouched"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("clean.txt")).expect("read clean.txt"),
            "clean-after\n",
            "clean target on disk must be updated"
        );
        let refreshed_clean = events
            .iter()
            .rev()
            .find_map(|event| match &event.payload {
                FrontendEvent::BufferSnapshot(snap)
                    if snap.path.as_str() == "clean.txt" && snap.text == "clean-after\n" =>
                {
                    Some(snap)
                }
                _ => None,
            })
            .expect("refresh_clean_buffers must emit updated BufferSnapshot");
        assert!(refreshed_clean.version > clean_buf.version);

        tauri::async_runtime::block_on(async { server.verify().await });
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn trust_revocation_mid_turn_aborts_parked_command_and_new_trust_works_without_restart() {
        let app = mock_app_handle();
        let root = temp_workspace("s3-trust-revoke");
        let settings_path = temp_settings_file("s3-trust-revoke");
        let mut settings = gateway_settings(String::new());
        let rounds = vec![
            (
                "simple follow-up",
                answer_round_sse("trusted again without restart", 0.001),
            ),
            (
                "run risky cmd",
                tool_round_sse(
                    &[(
                        "cmd-risky-1",
                        "bash",
                        r#"{"command":"New-Item touched.txt -ItemType File"}"#,
                    )],
                    "running command",
                    0.001,
                ),
            ),
        ];
        let server = mock_gateway(rounds, false);
        settings.skarn.base_url = server.uri();
        settings.agent.workspace_trusted = true;
        write_settings_file(&settings_path, &settings);

        let state = desktop_with_settings_path(settings_path.clone());
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::OpenFolderAt {
                        root: root.to_string_lossy().into_owned(),
                    },
                ),
            )
            .expect("open folder");

        state
            .dispatch(
                &app,
                envelope(&state, send_agent("req-risky-1", "sess-trust", "run risky cmd")),
            )
            .expect("send risky turn");

        let parked_events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ApprovalNeeded(_))
        });
        let approval = find_payload(&parked_events, |payload| match payload {
            FrontendEvent::ApprovalNeeded(dto) => Some(dto.clone()),
            _ => None,
        });
        assert_eq!(approval.action_class, Some(skarn_protocol::ActionClassDto::CommandGated));
        assert_eq!(approval.risk, Some(skarn_protocol::RiskLevelDto::High));

        // Revoke trust live mid-turn while the bash approval card is parked.
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::SettingsUpdate {
                        patch: skarn_protocol::SettingsPatchDto {
                            workspace_trusted: Some(false),
                            ..Default::default()
                        },
                    },
                ),
            )
            .expect("revoke trust live");

        let after_revoke = wait_for(&state, |event| {
            matches!(
                &event.payload,
                FrontendEvent::AgentTurnDone(done) if done.request_id.as_str() == "req-risky-1"
            )
        });
        let done_revoked = after_revoke
            .iter()
            .find_map(|e| match &e.payload {
                FrontendEvent::AgentTurnDone(done) if done.request_id.as_str() == "req-risky-1" => {
                    Some(done.clone())
                }
                _ => None,
            })
            .expect("turn done after trust revocation");
        assert!(!done_revoked.completed, "revoked turn must abort");
        assert!(
            !root.join("touched.txt").exists(),
            "revoked bash command must never execute"
        );
        assert!(
            after_revoke.iter().any(|e| matches!(
                &e.payload,
                FrontendEvent::DecisionTrace(trace)
                    if trace.reason_code == "trust_revoked"
                        && trace.phase == skarn_protocol::DecisionPhaseDto::Rejected
            )),
            "DecisionTrace must record trust_revoked rejection"
        );

        // Re-grant trust live and dispatch a new turn WITHOUT restarting DesktopState.
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::SettingsUpdate {
                        patch: skarn_protocol::SettingsPatchDto {
                            workspace_trusted: Some(true),
                            ..Default::default()
                        },
                    },
                ),
            )
            .expect("re-grant trust live");
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    send_agent("req-trust-2", "sess-trust", "simple follow-up"),
                ),
            )
            .expect("new turn must succeed without restart after live trust toggle");

        let done_events = wait_for(&state, |event| {
            matches!(
                &event.payload,
                FrontendEvent::AgentTurnDone(done) if done.request_id.as_str() == "req-trust-2"
            )
        });
        let done2 = done_events
            .iter()
            .find_map(|e| match &e.payload {
                FrontendEvent::AgentTurnDone(done) if done.request_id.as_str() == "req-trust-2" => {
                    Some(done.clone())
                }
                _ => None,
            })
            .expect("follow-up turn done");
        assert!(done2.completed);

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(settings_path.parent().unwrap());
    }

    #[test]
    fn turn_checkpoint_aborts_atomically_on_manual_edit_and_roundtrips_clean_undo_reapply() {
        let app = mock_app_handle();
        let root = temp_workspace("s3-checkpoint");
        let settings_path = temp_settings_file("s3-checkpoint");

        let rounds = vec![
            (
                "w-new",
                answer_round_sse("updated NOTES.md and created new_file.txt", 0.001),
            ),
            (
                "checkpoint two files",
                tool_round_sse(
                    &[
                        (
                            "w-notes",
                            "write_file",
                            r#"{"path":"NOTES.md","content":"agent-notes-v2\n"}"#,
                        ),
                        (
                            "w-new",
                            "write_file",
                            r#"{"path":"new_file.txt","content":"brand-new-v1\n"}"#,
                        ),
                    ],
                    "editing two files",
                    0.001,
                ),
            ),
        ];
        let server = mock_gateway(rounds, true);
        write_settings_file(&settings_path, &gateway_settings(server.uri()));
        let state = desktop_with_settings_path(settings_path.clone());

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::OpenFolderAt {
                        root: root.to_string_lossy().into_owned(),
                    },
                ),
            )
            .expect("open folder");
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::OpenFile {
                        path: skarn_protocol::WorkspacePath::new("NOTES.md").unwrap(),
                    },
                ),
            )
            .expect("open NOTES.md");

        let mut command = send_agent("req-cp-1", "sess-cp", "checkpoint two files");
        if let FrontendCommand::SendAgent { request, .. } = &mut command {
            request.mode = AgentModeDto::AutoEdit;
        }
        state
            .dispatch(&app, envelope(&state, command))
            .expect("send checkpoint turn");

        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::AgentTurnDone(_))
        });
        let summary = events
            .iter()
            .find_map(|e| match &e.payload {
                FrontendEvent::TurnCheckpointUpdated(cp) => Some(cp.clone()),
                _ => None,
            })
            .expect("TurnCheckpointUpdated emitted");
        assert!(!summary.undone);
        assert_eq!(summary.files.len(), 2);
        assert_eq!(
            std::fs::read_to_string(root.join("NOTES.md")).expect("NOTES.md"),
            "agent-notes-v2\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("new_file.txt")).expect("new_file.txt"),
            "brand-new-v1\n"
        );

        // Simulate human manual edit on disk: Undo must abort atomically with zero partial mutations!
        std::fs::write(root.join("NOTES.md"), "human-manual-edit\n").expect("manual edit");
        let conflict_err = state
            .dispatch_result(
                &app,
                envelope(
                    &state,
                    FrontendCommand::UndoAgentTurn {
                        session_id: summary.session_id.clone(),
                        turn_id: summary.turn_id.clone(),
                    },
                ),
            )
            .expect_err("manual edit must trigger atomic conflict abort");
        assert_eq!(conflict_err.code, "checkpoint_conflict");
        assert!(
            root.join("new_file.txt").exists(),
            "conflict abort must NOT delete new_file.txt (all-or-nothing)"
        );

        // Restore expected post-turn bytes, then perform clean Undo + Reapply round-trip.
        std::fs::write(root.join("NOTES.md"), "agent-notes-v2\n").expect("restore after bytes");
        state
            .dispatch_result(
                &app,
                envelope(
                    &state,
                    FrontendCommand::UndoAgentTurn {
                        session_id: summary.session_id.clone(),
                        turn_id: summary.turn_id.clone(),
                    },
                ),
            )
            .expect("clean undo succeeds");
        assert_eq!(
            std::fs::read_to_string(root.join("NOTES.md")).expect("undone NOTES.md"),
            "notes-body-xyz\n"
        );
        assert!(
            !root.join("new_file.txt").exists(),
            "existed_before == false file must be removed on Undo"
        );

        state
            .dispatch_result(
                &app,
                envelope(
                    &state,
                    FrontendCommand::ReapplyAgentTurn {
                        session_id: summary.session_id.clone(),
                        turn_id: summary.turn_id.clone(),
                    },
                ),
            )
            .expect("clean reapply succeeds");
        assert_eq!(
            std::fs::read_to_string(root.join("NOTES.md")).expect("reapplied NOTES.md"),
            "agent-notes-v2\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("new_file.txt")).expect("recreated new_file.txt"),
            "brand-new-v1\n"
        );

        tauri::async_runtime::block_on(async { server.verify().await });
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(settings_path.parent().unwrap());
    }

    #[test]
    fn untrusted_workspace_skips_workspace_rules_and_loads_global_rules_only() {
        let app = mock_app_handle();
        let root = temp_workspace("s3-rules");
        let settings_path = temp_settings_file("s3-rules");
        let app_data_dir = settings_path.parent().unwrap().to_path_buf();

        std::fs::write(app_data_dir.join("AGENTS.md"), "GLOBAL_RULE_MARKER_777\n")
            .expect("global AGENTS.md");
        std::fs::write(root.join("AGENTS.md"), "WORKSPACE_AGENTS_SECRET_888\n")
            .expect("workspace AGENTS.md");
        std::fs::create_dir_all(root.join(".skarn")).expect(".skarn dir");
        std::fs::write(root.join(".skarn").join("rules"), "WORKSPACE_SKARN_RULE_999\n")
            .expect("workspace .skarn/rules");

        let server = mock_gateway(
            vec![("rules check", answer_round_sse("ok", 0.001))],
            true,
        );
        let mut settings = gateway_settings(server.uri());
        settings.agent.workspace_trusted = false;
        write_settings_file(&settings_path, &settings);

        let state = desktop_with_settings_path(settings_path.clone());
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::OpenFolderAt {
                        root: root.to_string_lossy().into_owned(),
                    },
                ),
            )
            .expect("open folder");

        let boot = state.bootstrap();
        let global = boot
            .rules_in_effect
            .iter()
            .find(|r| r.scope == skarn_protocol::RuleScopeDto::Global)
            .expect("global rule loaded");
        assert!(global.active);
        let ws_rules: Vec<_> = boot
            .rules_in_effect
            .iter()
            .filter(|r| r.scope == skarn_protocol::RuleScopeDto::Workspace)
            .collect();
        assert_eq!(ws_rules.len(), 2);
        assert!(ws_rules
            .iter()
            .all(|r| !r.active && r.inactive_reason.as_deref() == Some("workspace_untrusted")));

        let mut cmd = send_agent("req-rules-1", "sess-rules", "rules check");
        if let FrontendCommand::SendAgent { request, .. } = &mut cmd {
            request.attachments.push(skarn_protocol::AttachmentDto {
                path: skarn_protocol::WorkspacePath::new("NOTES.md").unwrap(),
                kind: skarn_protocol::AttachmentKindDto::File,
                selection: None,
                label: Some("Primary note".into()),
            });
        }
        state.dispatch(&app, envelope(&state, cmd)).expect("send_agent");
        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::AgentTurnDone(_))
        });

        let requests =
            tauri::async_runtime::block_on(async { server.received_requests().await }).unwrap();
        let body = String::from_utf8_lossy(&requests[0].body);
        assert!(body.contains("GLOBAL_RULE_MARKER_777"));
        assert!(!body.contains("WORKSPACE_AGENTS_SECRET_888"));
        assert!(!body.contains("WORKSPACE_SKARN_RULE_999"));
        assert!(body.contains("[file] NOTES.md (Primary note)"));

        // Grant trust live -> workspace rules become active immediately via RulesLoaded.
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::SettingsUpdate {
                        patch: skarn_protocol::SettingsPatchDto {
                            workspace_trusted: Some(true),
                            ..Default::default()
                        },
                    },
                ),
            )
            .expect("trust workspace live");
        let trusted_boot = state.bootstrap();
        assert!(trusted_boot
            .rules_in_effect
            .iter()
            .filter(|r| r.scope == skarn_protocol::RuleScopeDto::Workspace)
            .all(|r| r.active));

        tauri::async_runtime::block_on(async { server.verify().await });
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&app_data_dir);
    }

    #[test]
    fn question_countdown_and_high_risk_grant_ceiling_enforced_in_desktop_dispatch() {
        let app = mock_app_handle();
        let root = temp_workspace("s3-question-ceiling");
        let rounds = vec![
            (
                "user denied this action",
                answer_round_sse("understood, not deleting", 0.001),
            ),
            (
                "delete-1",
                tool_round_sse(
                    &[("delete-1", "delete_file", r#"{"path":"NOTES.md"}"#)],
                    "asking delete",
                    0.001,
                ),
            ),
            (
                "Choice A",
                tool_round_sse(
                    &[("delete-1", "delete_file", r#"{"path":"NOTES.md"}"#)],
                    "now delete",
                    0.001,
                ),
            ),
            (
                "question and delete test",
                tool_round_sse(
                    &[(
                        "q-1",
                        "ask_user_question",
                        r#"{"questions":[{"question":"Which option?","options":[{"label":"Choice A","description":"first"}]}]}"#,
                    )],
                    "asking question",
                    0.001,
                ),
            ),
        ];
        let server = mock_gateway(rounds, false);
        let state = DesktopState::with_settings(gateway_settings(server.uri()));
        seed_supported_test_model(&state);
        state.sessions().set_workspace_root(Some(root.clone()));

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    send_agent("req-qc-1", "sess-qc", "question and delete test"),
                ),
            )
            .expect("send_agent");

        // 1. Non-destructive Question carries 300s countdown + supports retroactive pause.
        let q_events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ApprovalNeeded(_))
        });
        let q_approval = find_payload(&q_events, |payload| match payload {
            FrontendEvent::ApprovalNeeded(dto) => Some(dto.clone()),
            _ => None,
        });
        assert!(q_approval
            .question_timeout
            .as_ref()
            .is_some_and(|qt| qt.auto_continue_allowed && !qt.paused && !qt.timed_out));

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::SetQuestionCountdown {
                        request_id: q_approval.request_id.clone(),
                        approval_id: q_approval.approval_id.clone(),
                        paused: true,
                    },
                ),
            )
            .expect("pause countdown");
        let paused_events = wait_for(&state, |event| {
            matches!(
                &event.payload,
                FrontendEvent::QuestionCountdownUpdated(u) if u.timeout.paused
            )
        });
        assert!(paused_events.iter().any(|e| matches!(
            &e.payload,
            FrontendEvent::QuestionCountdownUpdated(u)
                if u.approval_id == q_approval.approval_id && u.timeout.paused
        )));

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::AgentDecision {
                        request_id: q_approval.request_id.clone(),
                        approval_id: q_approval.approval_id.clone(),
                        decision: DecisionDto::Answer,
                        resolution: Some(skarn_protocol::AgentDecisionResolutionDto::Question {
                            option_id: Some("Choice A".into()),
                            text: None,
                        }),
                    },
                ),
            )
            .expect("answer question");

        // 2. Destructive delete_file NEVER carries auto_continue question_timeout & rejects AllowProject.
        let d_approval = wait_for_new_approval(&state, Some(q_approval.approval_id.as_str()));
        let d_events = state.events_after(0);
        let d_card = d_events
            .iter()
            .rev()
            .find_map(|e| match &e.payload {
                FrontendEvent::ApprovalNeeded(dto)
                    if dto.approval_id == d_approval.approval_id =>
                {
                    Some(dto.clone())
                }
                _ => None,
            })
            .expect("delete approval card");
        assert!(d_card.question_timeout.is_none());
        assert_eq!(d_card.risk, Some(skarn_protocol::RiskLevelDto::High));

        let ceiling_err = state
            .dispatch_result(
                &app,
                envelope(
                    &state,
                    FrontendCommand::AgentDecision {
                        request_id: d_card.request_id.clone(),
                        approval_id: d_card.approval_id.clone(),
                        decision: DecisionDto::Approve,
                        resolution: Some(skarn_protocol::AgentDecisionResolutionDto::Scoped {
                            grant: skarn_protocol::GrantDecisionDto::AllowProject,
                            target_pattern: None,
                        }),
                    },
                ),
            )
            .expect_err("High-risk AllowProject must fail with grant_exceeds_risk_ceiling");
        assert_eq!(ceiling_err.code, "grant_exceeds_risk_ceiling");

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::AgentDecision {
                        request_id: d_card.request_id,
                        approval_id: d_card.approval_id,
                        decision: DecisionDto::Deny,
                        resolution: None,
                    },
                ),
            )
            .expect("deny delete");

        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::AgentTurnDone(_))
        });
        assert!(root.join("NOTES.md").exists());

        let _ = std::fs::remove_dir_all(&root);
    }
}
