//! Selora gateway data model (ARCHITECTURE.md §11).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use skarn_core::SkarnError;

use crate::chat_types::{ToolCall, ToolSpec};

/// Identifier of a model on the gateway (e.g. `"claude-opus-5-5"`).
pub type ModelId = String;

/// Authoritative tri-state tool-calling capability for a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCapability {
    Supported,
    Unsupported,
    #[default]
    Unknown,
}

impl ToolCapability {
    /// Convert an explicit boolean capability flag into [`ToolCapability`].
    pub fn from_bool(supported: bool) -> Self {
        if supported {
            Self::Supported
        } else {
            Self::Unsupported
        }
    }

    pub fn is_supported(self) -> bool {
        matches!(self, Self::Supported)
    }

    pub fn is_unsupported(self) -> bool {
        matches!(self, Self::Unsupported)
    }

    pub fn is_unknown(self) -> bool {
        matches!(self, Self::Unknown)
    }
}

/// Authoritative tri-state native reasoning/thinking capability for a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningCapability {
    Native,
    Unsupported,
    #[default]
    Unknown,
}

impl ReasoningCapability {
    pub fn from_bool(supported: bool) -> Self {
        if supported {
            Self::Native
        } else {
            Self::Unsupported
        }
    }

    pub fn is_native(self) -> bool {
        matches!(self, Self::Native)
    }

    pub fn is_unsupported(self) -> bool {
        matches!(self, Self::Unsupported)
    }
}

pub fn infer_reasoning_capability(model_id: &str) -> ReasoningCapability {
    let lower = model_id.trim().to_ascii_lowercase();
    let leaf = lower.rsplit('/').next().unwrap_or(lower.as_str());
    if leaf.is_empty() {
        return ReasoningCapability::Unknown;
    }

    if leaf.starts_with("gpt-5-chat")
        || leaf.starts_with("gpt-4o")
        || leaf.starts_with("gpt-4.1")
        || leaf == "gpt-4"
        || leaf.starts_with("gpt-4-")
        || leaf.starts_with("gpt-3.5")
        || leaf.starts_with("claude-3-5")
        || leaf.starts_with("claude-3.5")
        || leaf.starts_with("claude-3-haiku")
        || leaf.starts_with("claude-3-sonnet")
        || leaf.starts_with("claude-3-opus")
        || leaf.starts_with("gemini-1.")
        || (leaf.starts_with("gemini-2.0") && !leaf.contains("thinking"))
    {
        return ReasoningCapability::Unsupported;
    }

    if leaf.starts_with("o1")
        || leaf.starts_with("o3")
        || leaf.starts_with("o4")
        || leaf.starts_with("gpt-5")
        || leaf.starts_with("gpt-oss")
        || leaf.starts_with("claude-3-7")
        || leaf.starts_with("claude-3.7")
        || leaf.starts_with("claude-sonnet-4")
        || leaf.starts_with("claude-opus-4")
        || leaf.starts_with("claude-haiku-4")
        || leaf.starts_with("claude-4")
        || leaf.starts_with("gemini-2.5")
        || leaf.starts_with("gemini-3")
        || leaf.contains("thinking")
        || leaf.starts_with("deepseek-reasoner")
        || leaf.starts_with("deepseek-r1")
        || leaf.starts_with("deepseek-v3.1")
        || leaf.starts_with("deepseek-v3.2")
        || leaf.starts_with("glm-4.5")
        || leaf.starts_with("glm-4.6")
        || leaf.starts_with("glm-4.7")
        || leaf.starts_with("glm-5")
        || leaf.starts_with("glm-z1")
        || leaf.starts_with("qwen3")
        || leaf.starts_with("qwq")
        || leaf.starts_with("kimi-k2.5")
        || leaf.starts_with("minimax-m")
    {
        return ReasoningCapability::Native;
    }

    ReasoningCapability::Unknown
}

fn deserialize_lenient_u32<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt = Option::<serde_json::Value>::deserialize(deserializer)?;
    match opt {
        Some(serde_json::Value::Number(n)) => Ok(n.as_u64().unwrap_or(128_000) as u32),
        Some(serde_json::Value::String(s)) => Ok(s.parse::<u32>().unwrap_or(128_000)),
        _ => Ok(128_000),
    }
}

fn deserialize_lenient_f64<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt = Option::<serde_json::Value>::deserialize(deserializer)?;
    match opt {
        Some(serde_json::Value::Number(n)) => Ok(n.as_f64().unwrap_or(0.0)),
        Some(serde_json::Value::String(s)) => Ok(s.parse::<f64>().unwrap_or(0.0)),
        _ => Ok(0.0),
    }
}

/// A model offered by the gateway (`GET /v1/models`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Model {
    pub id: ModelId,
    /// Human-facing name. Falls back to `id` when the gateway omits it.
    pub display_name: String,
    /// Maximum context length in tokens.
    pub context_window: u32,
    /// USD per 1k input tokens.
    pub price_per_1k_input: f64,
    /// USD per 1k output tokens.
    pub price_per_1k_output: f64,
    pub supports_streaming: bool,
    /// Authoritative tri-state tool-calling capability (`Unknown` when absent).
    pub tool_capability: ToolCapability,
    /// Authoritative tri-state native thinking/reasoning capability.
    #[serde(default)]
    pub reasoning_capability: ReasoningCapability,
    /// Whether the model is active and available for selection (`false` for retired/410 models).
    #[serde(default = "default_true")]
    pub available: bool,
    /// Model family: "claude" / "gpt" / "kimi" / "glm" / ...
    pub family: String,
}

impl Model {
    /// Read-only helper: true only when tool calling is confirmed supported.
    pub fn supports_tools(&self) -> bool {
        self.tool_capability.is_supported()
    }

    /// Read-only helper: true only when native thinking/reasoning is supported.
    pub fn supports_reasoning(&self) -> bool {
        self.reasoning_capability.is_native()
    }

    /// Provider-native UI reasoning modes (`none`, `quick`, `standard`, `max`).
    pub fn reasoning_modes(&self) -> Vec<&'static str> {
        if self.reasoning_capability.is_unsupported() {
            return Vec::new();
        }
        let lower = self.id.trim().to_ascii_lowercase();
        let leaf = lower.rsplit('/').next().unwrap_or(lower.as_str());
        if self.reasoning_capability.is_native() {
            if leaf.starts_with("o1")
                || leaf.starts_with("o3")
                || leaf.starts_with("o4")
                || leaf.starts_with("qwq")
                || leaf.starts_with("gemini-2.5-pro")
            {
                return vec!["quick", "standard", "max"];
            }
            if leaf.starts_with("deepseek-reasoner") || leaf.starts_with("deepseek-r1") {
                return vec!["standard", "max"];
            }
            if leaf.starts_with("deepseek-chat")
                || leaf.starts_with("deepseek-v3.1")
                || leaf.starts_with("deepseek-v3.2")
                || leaf.starts_with("glm-4.5")
                || leaf.starts_with("glm-4.6")
                || leaf.starts_with("glm-4.7")
                || leaf.starts_with("glm-5")
                || leaf.starts_with("glm-z1")
            {
                return vec!["none", "standard"];
            }
        }
        vec!["none", "quick", "standard", "max"]
    }

    /// Read-only helper: true when the model is active and supports streaming.
    pub fn is_available(&self) -> bool {
        self.available && self.supports_streaming
    }
}

#[derive(Deserialize)]
struct RawModel {
    #[serde(default, alias = "model")]
    id: ModelId,
    #[serde(default, alias = "name", alias = "displayName")]
    display_name: String,
    #[serde(
        default,
        alias = "context_length",
        alias = "contextWindow",
        alias = "max_context_length",
        deserialize_with = "deserialize_lenient_u32"
    )]
    context_window: u32,
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    price_per_1k_input: f64,
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    price_per_1k_output: f64,
    #[serde(default = "default_true")]
    supports_streaming: bool,
    #[serde(default)]
    tool_capability: Option<ToolCapability>,
    #[serde(default)]
    supports_tools: Option<bool>,
    #[serde(default)]
    tool_calling: Option<bool>,
    #[serde(default)]
    function_calling: Option<bool>,
    #[serde(default)]
    reasoning_capability: Option<ReasoningCapability>,
    #[serde(default)]
    supports_reasoning: Option<bool>,
    #[serde(default)]
    capabilities: Option<serde_json::Value>,
    #[serde(default)]
    supported_parameters: Option<Vec<String>>,
    #[serde(default = "default_true")]
    available: bool,
    #[serde(default)]
    family: String,
}

pub(crate) fn extract_explicit_reasoning_capability(
    value: &serde_json::Value,
) -> Option<ReasoningCapability> {
    if let Some(cap) = value
        .get("reasoning_capability")
        .and_then(|v| serde_json::from_value::<ReasoningCapability>(v.clone()).ok())
    {
        return Some(cap);
    }
    for key in ["supports_reasoning", "reasoning", "thinking"] {
        if let Some(flag) = value.get(key).and_then(serde_json::Value::as_bool) {
            return Some(ReasoningCapability::from_bool(flag));
        }
    }
    if let Some(caps) = value.get("capabilities").and_then(serde_json::Value::as_object) {
        for key in ["reasoning", "thinking", "reasoning_effort", "supports_reasoning"] {
            if let Some(flag) = caps.get(key).and_then(serde_json::Value::as_bool) {
                return Some(ReasoningCapability::from_bool(flag));
            }
        }
    }
    if let Some(params) = value
        .get("supported_parameters")
        .and_then(serde_json::Value::as_array)
    {
        let strings: Vec<&str> = params.iter().filter_map(serde_json::Value::as_str).collect();
        if !strings.is_empty() {
            let has_reasoning = strings.iter().any(|p| {
                *p == "reasoning"
                    || *p == "include_reasoning"
                    || *p == "reasoning_effort"
                    || *p == "thinking"
            });
            return Some(ReasoningCapability::from_bool(has_reasoning));
        }
    }
    None
}

pub(crate) fn extract_explicit_tool_capability(value: &serde_json::Value) -> Option<ToolCapability> {
    if let Some(cap) = value
        .get("tool_capability")
        .and_then(|v| serde_json::from_value::<ToolCapability>(v.clone()).ok())
    {
        return Some(cap);
    }
    for key in ["supports_tools", "tool_calling", "function_calling"] {
        if let Some(flag) = value.get(key).and_then(serde_json::Value::as_bool) {
            return Some(ToolCapability::from_bool(flag));
        }
    }
    if let Some(caps) = value.get("capabilities").and_then(serde_json::Value::as_object) {
        for key in ["tools", "tool_calling", "function_calling", "supports_tools"] {
            if let Some(flag) = caps.get(key).and_then(serde_json::Value::as_bool) {
                return Some(ToolCapability::from_bool(flag));
            }
        }
    }
    if let Some(params) = value
        .get("supported_parameters")
        .and_then(serde_json::Value::as_array)
    {
        let strings: Vec<&str> = params.iter().filter_map(serde_json::Value::as_str).collect();
        if !strings.is_empty() {
            let has_tools = strings
                .iter()
                .any(|p| *p == "tools" || *p == "tool_choice" || *p == "functions");
            return Some(ToolCapability::from_bool(has_tools));
        }
    }
    None
}

impl<'de> Deserialize<'de> for Model {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawModel::deserialize(deserializer)?;
        let mut tool_capability = raw.tool_capability;
        if tool_capability.is_none() {
            tool_capability = raw
                .supports_tools
                .or(raw.tool_calling)
                .or(raw.function_calling)
                .map(ToolCapability::from_bool);
        }
        if tool_capability.is_none() {
            if let Some(caps) = raw.capabilities.as_ref().and_then(|v| v.as_object()) {
                for key in ["tools", "tool_calling", "function_calling", "supports_tools"] {
                    if let Some(flag) = caps.get(key).and_then(serde_json::Value::as_bool) {
                        tool_capability = Some(ToolCapability::from_bool(flag));
                        break;
                    }
                }
            }
        }
        if tool_capability.is_none() {
            if let Some(params) = raw.supported_parameters.as_ref() {
                if !params.is_empty() {
                    let has_tools = params
                        .iter()
                        .any(|p| p == "tools" || p == "tool_choice" || p == "functions");
                    tool_capability = Some(ToolCapability::from_bool(has_tools));
                }
            }
        }

        let mut reasoning_capability = raw.reasoning_capability;
        if reasoning_capability.is_none() {
            reasoning_capability = raw
                .supports_reasoning
                .map(ReasoningCapability::from_bool);
        }
        if reasoning_capability.is_none() {
            if let Some(caps) = raw.capabilities.as_ref().and_then(|v| v.as_object()) {
                for key in ["reasoning", "thinking", "reasoning_effort", "supports_reasoning"] {
                    if let Some(flag) = caps.get(key).and_then(serde_json::Value::as_bool) {
                        reasoning_capability = Some(ReasoningCapability::from_bool(flag));
                        break;
                    }
                }
            }
        }
        if reasoning_capability.is_none() {
            if let Some(params) = raw.supported_parameters.as_ref() {
                if !params.is_empty() {
                    let has_reasoning = params.iter().any(|p| {
                        p == "reasoning"
                            || p == "include_reasoning"
                            || p == "reasoning_effort"
                            || p == "thinking"
                    });
                    reasoning_capability = Some(ReasoningCapability::from_bool(has_reasoning));
                }
            }
        }
        let resolved_reasoning = reasoning_capability
            .unwrap_or_else(|| infer_reasoning_capability(&raw.id));

        Ok(Model {
            id: raw.id,
            display_name: raw.display_name,
            context_window: raw.context_window,
            price_per_1k_input: raw.price_per_1k_input,
            price_per_1k_output: raw.price_per_1k_output,
            supports_streaming: raw.supports_streaming,
            tool_capability: tool_capability.unwrap_or(ToolCapability::Unknown),
            reasoning_capability: resolved_reasoning,
            available: raw.available,
            family: raw.family,
        })
    }
}

fn default_true() -> bool {
    true
}

/// A chat role (wire names are lowercase, OpenAI-compatible).
///
/// `Tool` is the tool-result role (ARCHITECTURE.md §17.6): a message with
/// this role carries `tool_call_id` and the tool's output content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// One message in a chat conversation.
///
/// The agent loop grows this additively (ARCHITECTURE.md §17.6): an
/// assistant message may carry `tool_calls` (the model's tool invocations),
/// and a tool-result message carries `tool_call_id`. Both are `None` for
/// every pre-agent message, so old transcripts deserialize unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    /// Assistant-side tool invocations (`None` on plain messages).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// The call this tool-result message answers (tool role only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// A tool-result message (ARCHITECTURE.md §17.3: results are appended
    /// as `{ role: "tool", tool_call_id, content }` and fed back into the
    /// next request).
    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

pub fn normalize_reasoning_effort(raw: &str) -> Option<String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" => None,
        "quick" => Some("low".to_owned()),
        "standard" => Some("medium".to_owned()),
        "max" | "xhigh" => Some("high".to_owned()),
        "none" | "minimal" | "low" | "medium" | "high" => Some(raw.trim().to_ascii_lowercase()),
        _ => None,
    }
}

/// A chat completion request (`POST /v1/chat/completions`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: ModelId,
    pub messages: Vec<ChatMessage>,
    pub temperature: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    pub stream: bool,
    /// The tool manifest (ARCHITECTURE.md §17.3: every agent request arms
    /// the full fixed manifest; plain chat sends `None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolSpec>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

impl ChatRequest {
    /// New request with a default temperature of 0.7 and streaming off.
    /// `stream_chat` always sets `stream: true` on the wire regardless.
    pub fn new(model: impl Into<ModelId>, messages: Vec<ChatMessage>) -> Self {
        Self {
            model: model.into(),
            messages,
            temperature: 0.7,
            max_tokens: None,
            stream: false,
            tools: None,
            reasoning_effort: None,
        }
    }

    /// Builder-style override of temperature.
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = temperature;
        self
    }

    /// Builder-style override of reasoning effort, normalizing Selora UI labels
    /// (`quick`, `standard`, `max`) to OpenAI-compatible wire values.
    pub fn with_reasoning_effort(mut self, reasoning_effort: Option<String>) -> Self {
        self.reasoning_effort = reasoning_effort
            .as_deref()
            .and_then(normalize_reasoning_effort);
        self
    }

    /// Builder-style override of the `stream` flag.
    pub fn with_stream(mut self, stream: bool) -> Self {
        self.stream = stream;
        self
    }

    /// Builder-style override of the tool manifest (agent mode).
    pub fn with_tools(mut self, tools: Vec<ToolSpec>) -> Self {
        self.tools = Some(tools);
        self
    }
}

/// An event emitted on the chat channel while a completion streams.
#[derive(Debug)]
pub enum ChatEvent {
    /// One content delta from the model.
    Delta(String),
    /// A streaming fragment of a tool call (ARCHITECTURE.md §17.6: the
    /// same accumulate-then-parse pattern as content deltas — the
    /// arguments arrive as partial JSON across many deltas). `index`
    /// identifies which call of the response is growing.
    ToolCallDelta {
        /// The call's position in the response's `tool_calls` array.
        index: usize,
        /// The call id, present only on the first fragment of a call.
        id: Option<String>,
        /// The tool name, present only on the first fragment of a call.
        name: Option<String>,
        /// The partial `arguments` JSON fragment.
        arguments: String,
    },
    /// The stream completed; carries the receipt for the cost meter.
    Done(ChatReceipt),
    /// The stream failed; the same error is returned by `stream_chat`.
    Error(SkarnError),
}

/// The receipt for one completed chat request — the source of truth for the
/// cost meter (RISKS.md §3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatReceipt {
    pub request_id: Uuid,
    pub model: ModelId,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cost_usd: f64,
    pub duration_ms: u32,
    /// `true` when the gateway did not return usage and the token counts are
    /// the chars/4 estimate (RISKS.md §3).
    pub estimated: bool,
}

/// Gateway connection state for the status bar.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum SkarnStatus {
    #[default]
    Disconnected,
    Connecting,
    Connected {
        model: Option<ModelId>,
        latency_ms: u32,
    },
    Error(String),
}

/// USD cost of one completion, from a model's price table
/// (ARCHITECTURE.md §6.6). The cost meter sums these over a session.
pub fn compute_cost_usd(model: &Model, prompt_tokens: u32, completion_tokens: u32) -> f64 {
    (f64::from(prompt_tokens) * model.price_per_1k_input
        + f64::from(completion_tokens) * model.price_per_1k_output)
        / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_request_serializes_stream_flag_and_omits_max_tokens() {
        let req = ChatRequest::new("m", vec![ChatMessage::user("hi")]).with_stream(true);
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["model"], "m");
        assert_eq!(json["stream"], true);
        assert_eq!(json["messages"][0]["role"], "user");
        assert!(json.get("max_tokens").is_none());

        let req = req.with_stream(false);
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["stream"], false);
    }

    #[test]
    fn agent_fields_round_trip_and_stay_absent_on_plain_messages() {
        // A plain pre-agent request carries no tool fields on the wire.
        let plain = ChatRequest::new("m", vec![ChatMessage::user("hi")]);
        let json = serde_json::to_value(&plain).unwrap();
        assert!(json.get("tools").is_none());
        let first_message = &json["messages"][0];
        assert!(first_message.get("tool_calls").is_none());
        assert!(first_message.get("tool_call_id").is_none());
        // And deserializes back (old transcripts keep working).
        let back: ChatRequest = serde_json::from_value(json).unwrap();
        assert_eq!(back, plain);

        // An agent request: manifest armed, an assistant tool_call
        // message, and a tool-result message.
        let manifest = vec![ToolSpec::new(
            "read_file",
            "Read a workspace file",
            serde_json::json!({"type": "object"}),
        )];
        let call = ToolCall {
            id: "call_1".into(),
            name: "read_file".into(),
            arguments: r#"{"path":"src/main.rs"}"#.into(),
        };
        let mut message = ChatMessage::assistant("I'll read it.");
        message.tool_calls = Some(vec![call]);
        let request = ChatRequest::new(
            "m",
            vec![message, ChatMessage::tool_result("call_1", "fn main() {}")],
        )
        .with_tools(manifest);
        let json = serde_json::to_value(&request).unwrap();
        assert!(json["tools"].is_array());
        assert!(json["messages"][0]["tool_calls"].is_array());
        assert_eq!(json["messages"][1]["role"], "tool");
        assert_eq!(json["messages"][1]["tool_call_id"], "call_1");

        let back: ChatRequest = serde_json::from_value(json).unwrap();
        assert_eq!(back, request);
    }

    #[test]
    fn model_deserializes_minimal_and_aliased_fields() {
        let json = serde_json::json!({ "id": "glm-5.3" });
        let model: Model = serde_json::from_value(json).unwrap();
        assert_eq!(model.id, "glm-5.3");
        assert!(model.supports_streaming, "streaming defaults to true");
        assert_eq!(model.tool_capability, ToolCapability::Unknown);
        assert!(model.available);
        assert_eq!(model.price_per_1k_input, 0.0);

        let json = serde_json::json!({
            "id": "m2", "name": "Model Two", "context_length": 128000
        });
        let model: Model = serde_json::from_value(json).unwrap();
        assert_eq!(model.display_name, "Model Two");
        assert_eq!(model.context_window, 128000);
        assert_eq!(model.tool_capability, ToolCapability::Unknown);
    }

    #[test]
    fn model_honors_explicit_tool_capability_and_keeps_missing_unknown() {
        let supported: Model =
            serde_json::from_value(serde_json::json!({ "id": "m1", "supports_tools": true }))
                .unwrap();
        assert_eq!(supported.tool_capability, ToolCapability::Supported);
        assert!(supported.supports_tools());

        let unsupported: Model =
            serde_json::from_value(serde_json::json!({ "id": "m2", "supports_tools": false }))
                .unwrap();
        assert_eq!(unsupported.tool_capability, ToolCapability::Unsupported);
        assert!(!unsupported.supports_tools());

        let tri_state: Model = serde_json::from_value(
            serde_json::json!({ "id": "m3", "tool_capability": "unsupported" }),
        )
        .unwrap();
        assert_eq!(tri_state.tool_capability, ToolCapability::Unsupported);

        let missing: Model = serde_json::from_value(
            serde_json::json!({ "id": "m4", "object": "model", "owned_by": "nvidia" }),
        )
        .unwrap();
        assert_eq!(missing.tool_capability, ToolCapability::Unknown);
        assert!(!missing.supports_tools());

        let openrouter_thinking: Model = serde_json::from_value(serde_json::json!({
            "id": "openrouter/custom-reasoner",
            "supported_parameters": ["tools", "reasoning", "include_reasoning"]
        }))
        .unwrap();
        assert_eq!(openrouter_thinking.reasoning_capability, ReasoningCapability::Native);
        assert_eq!(
            openrouter_thinking.reasoning_modes(),
            vec!["none", "quick", "standard", "max"]
        );

        let openrouter_plain: Model = serde_json::from_value(serde_json::json!({
            "id": "openrouter/plain-chat",
            "supported_parameters": ["tools", "temperature"]
        }))
        .unwrap();
        assert_eq!(openrouter_plain.reasoning_capability, ReasoningCapability::Unsupported);
        assert!(openrouter_plain.reasoning_modes().is_empty());

        let deepseek_r1: Model = serde_json::from_value(serde_json::json!({
            "id": "deepseek-reasoner"
        }))
        .unwrap();
        assert_eq!(deepseek_r1.reasoning_capability, ReasoningCapability::Native);
        assert_eq!(deepseek_r1.reasoning_modes(), vec!["standard", "max"]);
    }

    #[test]
    fn compute_cost_usd_uses_per_1k_prices() {
        let model = Model {
            id: "m".into(),
            display_name: "m".into(),
            context_window: 0,
            price_per_1k_input: 0.5,
            price_per_1k_output: 1.5,
            supports_streaming: true,
            tool_capability: ToolCapability::Unsupported,
            reasoning_capability: ReasoningCapability::Unknown,
            available: true,
            family: String::new(),
        };
        assert!((compute_cost_usd(&model, 1000, 1000) - 2.0).abs() < 1e-12);
        assert!((compute_cost_usd(&model, 250, 0) - 0.125).abs() < 1e-12);
    }

    #[test]
    fn normalize_reasoning_effort_maps_ui_labels_to_openai_wire_values() {
        assert_eq!(normalize_reasoning_effort("quick").as_deref(), Some("low"));
        assert_eq!(
            normalize_reasoning_effort("standard").as_deref(),
            Some("medium")
        );
        assert_eq!(normalize_reasoning_effort("max").as_deref(), Some("high"));
        assert_eq!(normalize_reasoning_effort("none").as_deref(), Some("none"));
        assert_eq!(
            normalize_reasoning_effort("xhigh").as_deref(),
            Some("high")
        );
        assert_eq!(normalize_reasoning_effort("  ").as_deref(), None);

        let req = ChatRequest::new("m", vec![])
            .with_reasoning_effort(Some("standard".into()));
        assert_eq!(req.reasoning_effort.as_deref(), Some("medium"));
    }
}
