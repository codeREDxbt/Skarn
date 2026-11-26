//! `SeloraClient` — the HTTP client for the Selora gateway (ARCHITECTURE.md §6).

use std::fmt;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use selora_core::SeloraError;
use serde::Deserialize;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::chat_types::{ToolCall, ToolSpec};
use crate::types::{
    extract_explicit_tool_capability, ChatEvent, ChatMessage, ChatReceipt, ChatRequest, Model,
    SeloraStatus, ToolCapability,
};

/// 15s to establish the connection (ARCHITECTURE.md §6.1). A host that
/// cannot answer a TCP/TLS handshake in this window is unreachable, not slow.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// 2min read timeout: SSE bodies stream for a long time, but any single read
/// gap longer than this is a dead stream. Deliberately *not* an overall
/// request timeout, so long streams stay alive while data keeps flowing.
const READ_TIMEOUT: Duration = Duration::from_secs(2 * 60);
/// Overall deadline for one non-streaming gateway request (models, ghost
/// text, whole-reply chat), retries included. A hung gateway surfaces a typed
/// timeout instead of blocking the caller forever.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(90);
/// Deadline to receive an SSE response's headers. The stream body itself is
/// bounded by [`READ_TIMEOUT`] per read, never by this deadline.
const STREAM_RESPONSE_TIMEOUT: Duration = Duration::from_secs(90);
/// The reachability ping must answer quickly or it is not useful.
const STATUS_TIMEOUT: Duration = Duration::from_secs(5);
/// How long to wait for an error body before giving up on it.
const ERROR_BODY_TIMEOUT: Duration = Duration::from_secs(10);
/// The SSE sentinel that terminates a chat stream.
const DONE_SENTINEL: &str = "[DONE]";
/// The instruction that turns a chat model into a completion engine for
/// ghost text: continuation only, never prose.
const GHOST_SYSTEM_PROMPT: &str = "You are an inline code completion engine. You are given the code before the caret. Reply with ONLY the continuation that could be typed next at the caret — no prose, no explanation, no code fences, no repetition of the given code.";
/// Backoff for transport errors (ARCHITECTURE.md §6.5): 500ms, then 1s.
const NET_RETRY_BASE: Duration = Duration::from_millis(500);
/// Attempts on a transient HTTP status (408/429/5xx): three retries.
const MAX_HTTP_RETRIES: usize = 3;
/// Attempts on a transport error (connect refused, reset, timeout).
const MAX_NET_RETRIES: usize = 2;
/// Hard cap for any single backoff sleep (§6.5's 8s ceiling).
const MAX_RETRY_DELAY: Duration = Duration::from_secs(8);
/// Maximum jitter added on top of a backoff delay (percent of the delay).
const RETRY_JITTER_PERCENT: u64 = 25;
/// Prefix of every timeout message carried by [`SeloraError::Net`]. The
/// shared error type has no timeout variant, so this marker is the typed
/// contract: [`classify_failure`] and [`is_timeout`] key off it.
pub const TIMEOUT_MARKER: &str = "timeout: ";
/// How much of a gateway error body to surface in the error message.
const MAX_ERROR_BODY_CHARS: usize = 512;

/// The user-facing class of a gateway failure.
///
/// `selora-core`'s [`SeloraError`] is a shared contract (it has no timeout
/// variant), so this classification is the typed view the shells map onto
/// protocol error codes. It never inspects secrets: only variants and the
/// bounded, already-redacted message text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayFailure {
    /// The gateway did not answer inside the bounded deadline.
    Timeout,
    /// The key is missing, invalid, or not authorized (401/403).
    Auth,
    /// The host could not be reached (offline, DNS, connection refused).
    Network,
    /// The gateway answered 408/429/5xx even after the retry budget.
    Unavailable,
    /// The model or deployment is retired/end-of-life (HTTP 410) or unavailable.
    ModelUnavailable,
    /// The provider rejected tool calling / function calling (HTTP 400/422).
    ToolUnsupported,
    /// The gateway refused a non-transient request (400/404/422/...).
    Rejected,
    /// The gateway answered with something this client cannot parse.
    InvalidResponse,
}

/// Result of the non-streaming tool-capability runtime probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolProbeOutcome {
    /// The model accepted the probe tool manifest and emitted a tool call.
    Supported,
    /// The provider explicitly rejected tools/function calling for this model.
    Unsupported(String),
    /// Validation failed or was inconclusive (e.g. 404/410 or plain text with no tool call).
    Inconclusive(String),
}

#[derive(Debug, Clone)]
struct PayloadAdaptation {
    include_reasoning_effort: bool,
    include_openrouter_reasoning: bool,
    include_thinking_toggle: bool,
    omit_temperature: bool,
    use_max_completion_tokens: bool,
    drop_max_tokens: bool,
    inject_reasoning_content: bool,
    has_max_tokens: bool,
}

impl PayloadAdaptation {
    fn for_request(base_url: &str, req: &ChatRequest) -> Self {
        let base_lower = base_url.trim().to_ascii_lowercase();
        let model_lower = req.model.trim().to_ascii_lowercase();
        let leaf = model_lower.rsplit('/').next().unwrap_or(model_lower.as_str());

        let is_openrouter = base_lower.contains("openrouter.ai");
        let is_anthropic = base_lower.contains("api.anthropic.com");
        let is_deepseek = base_lower.contains("api.deepseek.com")
            || leaf.starts_with("deepseek-reasoner")
            || leaf.starts_with("deepseek-r1");
        let is_glm = base_lower.contains("bigmodel.cn")
            || base_lower.contains("z.ai")
            || leaf.starts_with("glm-4.5")
            || leaf.starts_with("glm-4.6")
            || leaf.starts_with("glm-5")
            || leaf.starts_with("glm-z1");
        let is_kimi_thinking =
            leaf.starts_with("kimi-k2-thinking") || leaf.starts_with("kimi-k2.5");
        let is_openai_reasoning = !is_openrouter
            && !leaf.starts_with("gpt-5-chat")
            && (leaf.starts_with("o1")
                || leaf.starts_with("o3")
                || leaf.starts_with("o4")
                || leaf.starts_with("gpt-5"));

        let has_effort = req.reasoning_effort.is_some();
        let active_effort = req
            .reasoning_effort
            .as_deref()
            .is_some_and(|effort| effort != "none");

        let include_openrouter_reasoning = has_effort && is_openrouter;
        let include_thinking_toggle = has_effort && !is_openrouter && (is_deepseek || is_glm);
        let suppress_none_effort = !active_effort
            && (is_anthropic
                || (is_openai_reasoning
                    && (leaf.starts_with("o1") || leaf.starts_with("o3") || leaf.starts_with("o4"))));
        let include_reasoning_effort =
            has_effort && !include_openrouter_reasoning && !include_thinking_toggle && !suppress_none_effort;

        Self {
            include_reasoning_effort,
            include_openrouter_reasoning,
            include_thinking_toggle,
            omit_temperature: is_openai_reasoning || (is_anthropic && active_effort),
            use_max_completion_tokens: is_openai_reasoning && req.max_tokens.is_some(),
            drop_max_tokens: false,
            inject_reasoning_content: is_deepseek || is_glm || is_kimi_thinking,
            has_max_tokens: req.max_tokens.is_some(),
        }
    }

    fn is_passthrough(&self, req: &ChatRequest) -> bool {
        self.include_reasoning_effort == req.reasoning_effort.is_some()
            && !self.include_openrouter_reasoning
            && !self.include_thinking_toggle
            && !self.omit_temperature
            && !self.use_max_completion_tokens
            && !self.drop_max_tokens
            && !self.inject_reasoning_content
    }

    fn adapt_on_rejection(&mut self, message: &str) -> bool {
        let lower = message.to_ascii_lowercase();
        let is_rejection = lower.contains("400")
            || lower.contains("422")
            || lower.contains("unknown")
            || lower.contains("unsupported")
            || lower.contains("unrecognized")
            || lower.contains("not allowed")
            || lower.contains("not permitted")
            || lower.contains("does not support")
            || lower.contains("extra_forbidden")
            || lower.contains("invalid");
        if !is_rejection {
            return false;
        }

        if self.include_reasoning_effort && lower.contains("reasoning_effort") {
            self.include_reasoning_effort = false;
            return true;
        }
        if self.include_openrouter_reasoning && lower.contains("reasoning") {
            self.include_openrouter_reasoning = false;
            return true;
        }
        if self.include_thinking_toggle && lower.contains("thinking") {
            self.include_thinking_toggle = false;
            return true;
        }
        if !self.drop_max_tokens
            && !self.use_max_completion_tokens
            && self.has_max_tokens
            && lower.contains("max_tokens")
        {
            self.use_max_completion_tokens = true;
            return true;
        }
        if !self.drop_max_tokens
            && self.use_max_completion_tokens
            && lower.contains("max_completion_tokens")
        {
            self.drop_max_tokens = true;
            return true;
        }
        if !self.omit_temperature && lower.contains("temperature") {
            self.omit_temperature = true;
            return true;
        }
        false
    }
}

fn split_leading_think_blocks(content: &str) -> (String, String) {
    let mut rest = content.trim_start();
    let mut collected = String::new();
    loop {
        let (open_len, close_tag) = if rest.starts_with("<think>") {
            ("<think>".len(), "</think>")
        } else if rest.starts_with("<thinking>") {
            ("<thinking>".len(), "</thinking>")
        } else {
            break;
        };
        let after_open = &rest[open_len..];
        let Some(end_idx) = after_open.find(close_tag) else {
            break;
        };
        let block = after_open[..end_idx].trim();
        if !block.is_empty() {
            if !collected.is_empty() {
                collected.push_str("\n\n");
            }
            collected.push_str(block);
        }
        rest = after_open[end_idx + close_tag.len()..].trim_start();
    }
    (collected, rest.to_owned())
}

fn build_chat_payload(req: &ChatRequest, adapt: &PayloadAdaptation) -> serde_json::Value {
    let mut val = serde_json::to_value(req).unwrap_or_else(|_| serde_json::json!({}));
    if let Some(obj) = val.as_object_mut() {
        // Preserve f32's crisp Ryū formatting (e.g. `0.7` instead of `0.699999988079071`).
        if !adapt.omit_temperature {
            if let Ok(temp_json) = serde_json::to_string(&req.temperature) {
                if let Ok(temp_val) = serde_json::from_str::<serde_json::Value>(&temp_json) {
                    obj.insert("temperature".into(), temp_val);
                }
            }
        }
        if !adapt.include_reasoning_effort {
            obj.remove("reasoning_effort");
        }
        if adapt.include_openrouter_reasoning {
            if let Some(eff) = req.reasoning_effort.as_deref() {
                let reasoning_val = if eff == "none" {
                    serde_json::json!({ "enabled": false })
                } else {
                    serde_json::json!({ "effort": eff })
                };
                obj.insert("reasoning".into(), reasoning_val);
            }
        }
        if adapt.include_thinking_toggle {
            if let Some(eff) = req.reasoning_effort.as_deref() {
                let mode = if eff == "none" { "disabled" } else { "enabled" };
                obj.insert("thinking".into(), serde_json::json!({ "type": mode }));
            }
        }
        if adapt.omit_temperature {
            obj.remove("temperature");
        }
        if adapt.drop_max_tokens {
            obj.remove("max_tokens");
        } else if adapt.use_max_completion_tokens {
            if let Some(max_tok) = obj.remove("max_tokens") {
                obj.insert("max_completion_tokens".into(), max_tok);
            }
        }
        if adapt.inject_reasoning_content {
            if let Some(messages) = obj.get_mut("messages").and_then(|m| m.as_array_mut()) {
                for msg in messages {
                    let Some(msg_obj) = msg.as_object_mut() else {
                        continue;
                    };
                    let is_assistant =
                        msg_obj.get("role").and_then(|r| r.as_str()) == Some("assistant");
                    let has_tool_calls = msg_obj
                        .get("tool_calls")
                        .and_then(|tc| tc.as_array())
                        .is_some_and(|arr| !arr.is_empty());
                    if !is_assistant || !has_tool_calls {
                        continue;
                    }
                    let existing = msg_obj
                        .get("content")
                        .and_then(|c| c.as_str())
                        .unwrap_or("")
                        .to_owned();
                    let (thought, visible) = split_leading_think_blocks(&existing);
                    msg_obj.insert("reasoning_content".into(), serde_json::Value::String(thought));
                    msg_obj.insert("content".into(), serde_json::Value::String(visible));
                }
            }
        }
    }
    val
}

fn extract_stream_error_message(val: &serde_json::Value) -> Option<&str> {
    val.get("error")
        .and_then(|e| e.get("message").and_then(|m| m.as_str()).or_else(|| e.as_str()))
        .or_else(|| val.get("detail").and_then(|d| d.as_str()))
}

fn classify_probe_error(err: SeloraError, _base_url: &str) -> Result<ToolProbeOutcome, SeloraError> {
    match classify_failure(&err) {
        GatewayFailure::ToolUnsupported => Ok(ToolProbeOutcome::Unsupported(user_message(&err))),
        GatewayFailure::ModelUnavailable | GatewayFailure::Rejected => {
            Ok(ToolProbeOutcome::Inconclusive(user_message(&err)))
        }
        GatewayFailure::Auth
        | GatewayFailure::Network
        | GatewayFailure::Timeout
        | GatewayFailure::Unavailable
        | GatewayFailure::InvalidResponse => Err(err),
    }
}

/// Safe, credential-free provider label derived from a gateway base URL.
pub fn safe_provider_label(base_url: &str) -> String {
    if let Ok(parsed) = reqwest::Url::parse(base_url.trim()) {
        if let Some(host) = parsed.host_str() {
            let host_lower = host.to_ascii_lowercase();
            let host_port = match parsed.port() {
                Some(port) => format!("{host}:{port}"),
                None => host.to_owned(),
            };
            if host_lower.contains("nvidia") {
                return format!("NVIDIA ({host_port})");
            }
            return host_port;
        }
    }
    "gateway".to_owned()
}

/// Apply narrow, evidence-based capability/availability policy when explicit
/// provider metadata is absent. Unverified models remain [`ToolCapability::Unknown`].
pub fn apply_model_policy(_base_url: &str, model: &mut Model) {
    let id_lower = model.id.trim().to_ascii_lowercase();
    if id_lower == "mistralai/mistral-large-4-1t-a52b" {
        if model.tool_capability == ToolCapability::Unknown {
            model.tool_capability = ToolCapability::Supported;
        }
    } else if id_lower == "meta/muse-glimmer-30b" {
        model.available = false;
        if model.tool_capability == ToolCapability::Unknown {
            model.tool_capability = ToolCapability::Unsupported;
        }
    }
    if model.reasoning_capability == crate::types::ReasoningCapability::Unknown {
        model.reasoning_capability = crate::types::infer_reasoning_capability(&model.id);
    }
}

/// Apply [`apply_model_policy`] across a discovered catalog.
pub fn apply_catalog_policy(base_url: &str, models: &mut [Model]) {
    for model in models {
        apply_model_policy(base_url, model);
    }
}

fn is_tool_capability_error_body(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    let mentions_tools = lower.contains("tool")
        || lower.contains("function calling")
        || lower.contains("function_calling")
        || lower.contains("functions");
    if !mentions_tools {
        return false;
    }
    lower.contains("not support")
        || lower.contains("doesn't support")
        || lower.contains("does not support")
        || lower.contains("unsupported")
        || lower.contains("not allowed")
        || lower.contains("unrecognized")
        || lower.contains("unknown field")
        || lower.contains("invalid parameter")
        || lower.contains("not enabled")
        || lower.contains("cannot use")
}

/// Classify a client failure for error codes and user-facing copy.
pub fn classify_failure(error: &SeloraError) -> GatewayFailure {
    match error {
        SeloraError::Auth(_) => GatewayFailure::Auth,
        SeloraError::Net(message) if message.starts_with(TIMEOUT_MARKER) => GatewayFailure::Timeout,
        SeloraError::Net(_) => GatewayFailure::Network,
        SeloraError::Ai(message) => {
            if message.contains("gateway unavailable") {
                GatewayFailure::Unavailable
            } else if message.contains("(HTTP 410") || message.contains("retired/end-of-life") {
                GatewayFailure::ModelUnavailable
            } else if message.contains("Tool calling is not supported")
                || is_tool_capability_error_body(message)
            {
                GatewayFailure::ToolUnsupported
            } else if message.contains("gateway rejected") {
                GatewayFailure::Rejected
            } else {
                GatewayFailure::InvalidResponse
            }
        }
        _ => GatewayFailure::InvalidResponse,
    }
}

/// True when `error` is the bounded-deadline timeout this client produces.
pub fn is_timeout(error: &SeloraError) -> bool {
    classify_failure(error) == GatewayFailure::Timeout
}

/// A short, user-readable sentence for `error`. Never contains the API key:
/// gateway bodies are redacted before they are stored in the error, and this
/// function only adds fixed copy (or the already-safe error text).
pub fn user_message(error: &SeloraError) -> String {
    match classify_failure(error) {
        GatewayFailure::Timeout => {
            "The AI gateway did not answer in time. Check your connection and try again.".into()
        }
        GatewayFailure::Auth => {
            "The API key was rejected. Check the key in Settings, then try again.".into()
        }
        GatewayFailure::Network => {
            "Cannot reach the AI gateway. Check your internet connection and try again.".into()
        }
        GatewayFailure::Unavailable => {
            "The AI gateway is temporarily unavailable. Try again in a moment.".into()
        }
        GatewayFailure::ModelUnavailable
        | GatewayFailure::ToolUnsupported
        | GatewayFailure::Rejected
        | GatewayFailure::InvalidResponse => error.to_string(),
    }
}

/// The exponential backoff delay for `attempt` (0-based) over `base`, capped
/// at [`MAX_RETRY_DELAY`]. Jitter is applied separately by [`with_jitter`].
fn retry_delay(base: Duration, attempt: usize) -> Duration {
    let factor = 1u32.checked_shl(attempt.min(16) as u32).unwrap_or(u32::MAX);
    base.saturating_mul(factor).min(MAX_RETRY_DELAY)
}

/// Add up to [`RETRY_JITTER_PERCENT`] jitter so synchronized clients do not
/// retry in lockstep. The spread seed is cheap process time, not a secret.
fn with_jitter(delay: Duration) -> Duration {
    let spread_ms = (delay.as_millis() as u64).saturating_mul(RETRY_JITTER_PERCENT) / 100;
    if spread_ms == 0 {
        return delay;
    }
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| u64::from(elapsed.subsec_nanos()))
        .unwrap_or(0);
    delay + Duration::from_millis(seed % spread_ms)
}

/// Format a bounded duration for an error message (`50ms` / `90s`).
fn describe_duration(duration: Duration) -> String {
    if duration < Duration::from_secs(1) {
        format!("{}ms", duration.as_millis())
    } else {
        format!("{}s", duration.as_secs())
    }
}

/// The one timeout error shape: [`SeloraError::Net`] with the typed marker.
fn timeout_error(what: &str, budget: Duration) -> SeloraError {
    SeloraError::Net(format!(
        "{TIMEOUT_MARKER}{what} timed out after {}",
        describe_duration(budget)
    ))
}

/// True when a transport error is worth retrying. Builder/decode failures are
/// deterministic and would fail identically on every attempt.
fn is_transient_transport(err: &reqwest::Error) -> bool {
    err.is_timeout() || err.is_connect() || err.is_request()
}

/// True for the transient HTTP statuses worth retrying: request timeout,
/// rate limit, and every 5xx. 401/403/400 and the rest of the 4xx range are
/// never retried.
fn is_transient_status(status: reqwest::StatusCode) -> bool {
    status.as_u16() == 408 || status.as_u16() == 429 || status.is_server_error()
}

/// Sleep `delay`, but never past `deadline`. Returns `false` when the
/// deadline would be crossed (the caller surfaces a typed timeout).
async fn sleep_within(deadline: tokio::time::Instant, delay: Duration) -> bool {
    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
    if delay >= remaining {
        return false;
    }
    tokio::time::sleep(delay).await;
    true
}

/// Client for the Selora gateway.
///
/// The API key is only ever sent in the `Authorization` header. It never
/// appears in error messages or `Debug` output: error bodies are redacted and
/// the `Debug` impl prints `[REDACTED]`.
pub struct SeloraClient {
    base_url: String,
    api_key: String,
    http: reqwest::Client,
    /// Test seam: the production retry policy with millisecond delays.
    net_retry_delay: Duration,
    http_retry_delay: Duration,
    max_net_retries: usize,
    max_http_retries: usize,
    /// Overall deadline for one non-streaming request (retries included).
    request_timeout: Duration,
    /// Deadline to receive a streaming response's headers.
    stream_response_timeout: Duration,
}

impl fmt::Debug for SeloraClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The API key must never reach logs or panics (RISKS.md §8.3).
        f.debug_struct("SeloraClient")
            .field("base_url", &self.base_url)
            .field("api_key", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl SeloraClient {
    /// A client for `base_url` (e.g. `https://api.openai.com/v1`) authenticated
    /// with `api_key`. No settings or keyring access: the caller owns key
    /// retrieval. A trailing `/` on `base_url` is tolerated.
    pub fn new(base_url: &str, api_key: &str) -> Self {
        // `build()` fails only if the TLS backend fails to initialize, which
        // a default client would equally fail on; fall back rather than panic.
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            api_key: api_key.to_owned(),
            http,
            net_retry_delay: NET_RETRY_BASE,
            http_retry_delay: Duration::from_secs(1),
            max_net_retries: MAX_NET_RETRIES,
            max_http_retries: MAX_HTTP_RETRIES,
            request_timeout: REQUEST_TIMEOUT,
            stream_response_timeout: STREAM_RESPONSE_TIMEOUT,
        }
    }

    fn resolved_endpoint_url(&self, suffix: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with(suffix) {
            return base.to_owned();
        }
        const COMPAT_PREFIXES: [&str; 7] = [
            "/v1beta/openai",
            "/v1beta",
            "/openai",
            "/v1",
            "/v2",
            "/v3",
            "/v4",
        ];
        if COMPAT_PREFIXES.iter().any(|prefix| base.ends_with(prefix)) {
            format!("{base}{suffix}")
        } else {
            format!("{base}/v1{suffix}")
        }
    }

    fn models_url(&self) -> String {
        self.resolved_endpoint_url("/models")
    }

    fn chat_completions_url(&self) -> String {
        self.resolved_endpoint_url("/chat/completions")
    }

    fn apply_provider_headers(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let trimmed_key = self.api_key.trim();
        if !trimmed_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
            if self
                .base_url
                .trim()
                .to_ascii_lowercase()
                .contains("api.anthropic.com")
            {
                req = req
                    .header("x-api-key", trimmed_key)
                    .header("anthropic-version", "2023-06-01");
            }
        }
        req.header("HTTP-Referer", "https://github.com/codeREDxbt/Skarn")
            .header("X-Title", "Skarn")
            .header("User-Agent", concat!("Skarn/", env!("CARGO_PKG_VERSION")))
    }

    /// `GET {base_url}/v1/models` — the model catalog, bearer-authenticated.
    pub async fn list_models(&self) -> Result<Vec<Model>, SeloraError> {
        let url = self.models_url();
        let http = self.http.clone();

        let response_res = self
            .send_with_retry("model catalog", None, self.request_timeout, || {
                self.apply_provider_headers(http.get(&url))
            })
            .await;

        let text = match response_res {
            Ok(resp) => self.read_text(resp, "model catalog").await?,
            Err(err) => {
                // If standard /v1/models failed, check if this is an Ollama host and try /api/tags
                let base = self.base_url.trim_end_matches('/');
                if base.contains("11434") || base.contains("ollama") {
                    let tags_url = format!("{base}/api/tags");
                    if let Ok(resp) = self.bounded_get(&tags_url).await {
                        if resp.status().is_success() {
                            if let Ok(tags_text) = self.read_text(resp, "model catalog").await {
                                let mut models = parse_models_from_json(&tags_text);
                                if !models.is_empty() {
                                    apply_catalog_policy(&self.base_url, &mut models);
                                    return Ok(models);
                                }
                            }
                        }
                    }
                }
                return Err(err);
            }
        };

        if let Err(json_err) = serde_json::from_str::<serde_json::Value>(&text) {
            return Err(SeloraError::Ai(format!(
                "invalid models response: {json_err}"
            )));
        }

        let mut models = parse_models_from_json(&text);
        // Fallback check for Ollama /api/tags if /v1/models gave an empty list
        if models.is_empty() {
            let base = self.base_url.trim_end_matches('/');
            if base.contains("11434") || base.contains("ollama") {
                let tags_url = format!("{base}/api/tags");
                if let Ok(resp) = self.bounded_get(&tags_url).await {
                    if resp.status().is_success() {
                        if let Ok(tags_text) = self.read_text(resp, "model catalog").await {
                            models = parse_models_from_json(&tags_text);
                        }
                    }
                }
            }
        }

        for model in &mut models {
            if model.id.is_empty() && !model.display_name.is_empty() {
                model.id = model.display_name.clone();
            }
            if model.display_name.is_empty() {
                model.display_name = model.id.clone();
            }
        }
        apply_catalog_policy(&self.base_url, &mut models);
        Ok(models)
    }

    /// `POST {base_url}/v1/chat/completions` with `stream: true`.
    ///
    /// Consumes the SSE body: one [`ChatEvent::Delta`] per content delta,
    /// [`ChatEvent::Done`] on the `[DONE]` sentinel, and the receipt is both
    /// sent on the channel and returned. On any failure the same error is
    /// sent as [`ChatEvent::Error`] before being returned.
    ///
    /// A broken stream may be retried before any content or tool-call fragment
    /// is emitted. Once output is visible, replay would duplicate it and is
    /// never attempted.
    pub async fn stream_chat(
        &self,
        req: ChatRequest,
        tx: mpsc::UnboundedSender<ChatEvent>,
    ) -> Result<ChatReceipt, SeloraError> {
        let request_id = Uuid::new_v4();
        let started = Instant::now();
        let url = self.chat_completions_url();
        let mut body = req.clone().with_stream(true);
        body.reasoning_effort = body
            .reasoning_effort
            .as_deref()
            .and_then(crate::types::normalize_reasoning_effort);
        let mut adapt = PayloadAdaptation::for_request(&self.base_url, &body);

        let mut stream_retries = 0;
        let mut param_retries = 0;
        'attempt: loop {
            let payload = (!adapt.is_passthrough(&body)).then(|| build_chat_payload(&body, &adapt));
            let response = match self
                .send_with_retry(
                    "chat stream",
                    Some(req.model.as_str()),
                    self.stream_response_timeout,
                    || {
                        let req_builder = self.apply_provider_headers(self.http.post(&url));
                        if let Some(ref custom_payload) = payload {
                            req_builder.json(custom_payload)
                        } else {
                            req_builder.json(&body)
                        }
                    },
                )
                .await
            {
                Ok(response) => response,
                Err(err) => {
                    if param_retries < 4 && adapt.adapt_on_rejection(&err.to_string()) {
                        param_retries += 1;
                        continue 'attempt;
                    }
                    let _ = send_event(&tx, ChatEvent::Error(duplicate_error(&err)));
                    return Err(err);
                }
            };

            // Receipt inputs: the last `usage` the gateway sent, and the raw
            // character counts for the chars/4 fallback (RISKS.md §3).
            let prompt_chars: usize = req.messages.iter().map(|m| m.content.chars().count()).sum();
            let mut usage: Option<Usage> = None;
            let mut completion_chars: usize = 0;
            let mut in_reasoning = false;
            let mut emitted = false;
            let mut saw_finish_reason = false;

            let mut buffer: Vec<u8> = Vec::new();
            let mut stream = response.bytes_stream();

            while let Some(chunk_result) = stream.next().await {
                let chunk = match chunk_result {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        if !emitted && !tx.is_closed() && stream_retries < self.max_net_retries {
                            let delay =
                                with_jitter(retry_delay(self.net_retry_delay, stream_retries));
                            stream_retries += 1;
                            tokio::time::sleep(delay).await;
                            continue 'attempt;
                        }
                        let error = if err.is_timeout() {
                            timeout_error("chat stream read", READ_TIMEOUT)
                        } else {
                            SeloraError::Net("stream read failed: connection interrupted".into())
                        };
                        let _ = send_event(&tx, ChatEvent::Error(duplicate_error(&error)));
                        return Err(error);
                    }
                };
                buffer.extend_from_slice(&chunk);

                loop {
                    match crate::sse::parse_sse_event(&buffer) {
                        Err(err) => {
                            let _ = send_event(&tx, ChatEvent::Error(duplicate_error(&err)));
                            return Err(err);
                        }
                        Ok(None) => break, // incomplete event: wait for more bytes
                        Ok(Some((event, consumed))) => {
                            buffer.drain(..consumed);
                            let Some(data) = event.data else {
                                continue; // fieldless/comment-only event
                            };
                            if data.trim().is_empty() {
                                continue;
                            }
                            if data == DONE_SENTINEL {
                                if in_reasoning {
                                    let _ = send_event(
                                        &tx,
                                        ChatEvent::Delta("</think>\n\n".to_owned()),
                                    );
                                }
                                let receipt = build_receipt(
                                    &req,
                                    request_id,
                                    started,
                                    usage,
                                    prompt_chars,
                                    completion_chars,
                                );
                                send_event(&tx, ChatEvent::Done(receipt.clone()))?;
                                return Ok(receipt);
                            }
                            let chunk: ChatChunk = match serde_json::from_str(&data) {
                                Ok(chunk) => chunk,
                                Err(err) => {
                                    if let Ok(val) =
                                        serde_json::from_str::<serde_json::Value>(&data)
                                    {
                                        if let Some(msg) = extract_stream_error_message(&val) {
                                            let error =
                                                SeloraError::Ai(format!("gateway error: {msg}"));
                                            let _ = send_event(
                                                &tx,
                                                ChatEvent::Error(duplicate_error(&error)),
                                            );
                                            return Err(error);
                                        }
                                    }
                                    let error =
                                        SeloraError::Ai(format!("malformed stream chunk: {err}"));
                                    let _ =
                                        send_event(&tx, ChatEvent::Error(duplicate_error(&error)));
                                    return Err(error);
                                }
                            };
                            if let Some(chunk_usage) = chunk.usage {
                                usage = Some(chunk_usage);
                            }
                            let first_choice = chunk.choices.first();
                            if first_choice
                                .and_then(|choice| choice.finish_reason.as_deref())
                                .is_some_and(|reason| !reason.is_empty())
                            {
                                saw_finish_reason = true;
                            }
                            let delta = first_choice.and_then(|choice| choice.delta.as_ref());

                            let reasoning_owned = delta.and_then(ChunkDelta::reasoning_text);
                            let reasoning_opt = reasoning_owned.as_deref().filter(|s| !s.is_empty());

                            let content_opt = delta
                                .and_then(|d| d.content.as_deref())
                                .or_else(|| delta.and_then(|d| d.text.as_deref()))
                                .or_else(|| first_choice.and_then(|c| c.text.as_deref()))
                                .or_else(|| {
                                    first_choice.and_then(|c| {
                                        c.message.as_ref().and_then(|m| m.content.as_deref())
                                    })
                                })
                                .filter(|s| !s.is_empty());

                            if let Some(reasoning) = reasoning_opt {
                                emitted = true;
                                if !in_reasoning {
                                    in_reasoning = true;
                                    send_event(&tx, ChatEvent::Delta("<think>".to_owned()))?;
                                }
                                completion_chars += reasoning.chars().count();
                                send_event(&tx, ChatEvent::Delta(reasoning.to_owned()))?;
                            }

                            if let Some(content) = content_opt {
                                emitted = true;
                                if in_reasoning {
                                    in_reasoning = false;
                                    send_event(&tx, ChatEvent::Delta("</think>\n\n".to_owned()))?;
                                }
                                completion_chars += content.chars().count();
                                send_event(&tx, ChatEvent::Delta(content.to_owned()))?;
                            }

                            let Some(delta) = delta else {
                                continue;
                            };
                            if let Some(fragments) = delta.tool_calls.as_deref() {
                                for fragment in fragments {
                                    let function = fragment.function.as_ref();
                                    let arguments = function
                                        .and_then(|function| function.arguments.as_deref())
                                        .unwrap_or_default();
                                    let name = function.and_then(|function| function.name.clone());
                                    let meaningful =
                                        fragment.id.as_deref().is_some_and(|id| !id.is_empty())
                                            || name.as_deref().is_some_and(|name| !name.is_empty())
                                            || !arguments.is_empty();
                                    if !meaningful {
                                        continue; // empty trailing fragment
                                    }
                                    emitted = true;
                                    send_event(
                                        &tx,
                                        ChatEvent::ToolCallDelta {
                                            // Gateways that omit `index` (single-call
                                            // responses) land in slot 0.
                                            index: fragment.index.unwrap_or(0),
                                            id: fragment.id.clone(),
                                            name,
                                            arguments: arguments.to_owned(),
                                        },
                                    )?;
                                }
                            }
                        }
                    }
                }
            }

            if saw_finish_reason {
                if in_reasoning {
                    let _ = send_event(&tx, ChatEvent::Delta("</think>\n\n".to_owned()));
                }
                let receipt = build_receipt(
                    &req,
                    request_id,
                    started,
                    usage,
                    prompt_chars,
                    completion_chars,
                );
                send_event(&tx, ChatEvent::Done(receipt.clone()))?;
                return Ok(receipt);
            }
            if !emitted && !tx.is_closed() && stream_retries < self.max_net_retries {
                let delay = with_jitter(retry_delay(self.net_retry_delay, stream_retries));
                stream_retries += 1;
                tokio::time::sleep(delay).await;
                continue 'attempt;
            }
            // The body ended without the `[DONE]` sentinel or a `finish_reason`:
            // the reply was truncated (RISKS.md §3). Never report a receipt we cannot trust.
            let error = SeloraError::Ai("stream closed before [DONE]".into());
            let _ = send_event(&tx, ChatEvent::Error(duplicate_error(&error)));
            return Err(error);
        }
    }

    /// `POST {base_url}/v1/chat/completions` with `stream: false` — one
    /// ghost-text completion (ARCHITECTURE.md §10.6, PLAN.md §6.4).
    ///
    /// `model` is the caller's choice (the shell passes the active model);
    /// `prompt` is the editor's up-to-caret prefix. The reply's first
    /// choice content is the suggestion, trimmed. Empty or whitespace-only
    /// content is `Ok(None)`: a suggestion of nothing is no suggestion.
    ///
    /// Non-streaming on purpose — a ghost completion is short and the
    /// editor needs the whole string in one piece to paint at the caret.
    /// Receipts for ghost completions are not returned: PLAN.md §6.4 keeps
    /// the cost meter fed by the `usage` of the *chat* stream; the shell
    /// can add ghost receipts to the total via the response length if the
    /// owner asks for it.
    pub async fn ghost_complete(
        &self,
        model: &str,
        prompt: &str,
    ) -> Result<Option<String>, SeloraError> {
        let url = self.chat_completions_url();
        let body = ChatRequest::new(
            model,
            vec![
                ChatMessage::system(GHOST_SYSTEM_PROMPT),
                ChatMessage::user(prompt),
            ],
        );

        let response = self
            .send_with_retry(
                "ghost completion",
                Some(model),
                self.request_timeout,
                || {
                    self.http.post(&url).bearer_auth(&self.api_key).json(&body)
                },
            )
            .await?;
        let reply: CompletionResponse = self.read_json(response, "ghost completion").await?;

        Ok(reply
            .choices
            .into_iter()
            .next()
            .and_then(|choice| choice.message)
            .map(|message| message.content.unwrap_or_default())
            .map(|content| content.trim().to_owned())
            .filter(|content| !content.is_empty()))
    }

    /// `POST {base_url}/v1/chat/completions` with `stream: false` — the
    /// whole assistant reply in one piece: the content plus, when the
    /// model invoked tools, the complete `tool_calls` of the message
    /// (ARCHITECTURE.md §17.6 non-streaming shape). `Ok(None)` when the
    /// gateway returns no choices.
    ///
    /// The agent loop itself streams (§17.3 — it needs live deltas for
    /// the chat bubble); this is the non-streaming seam for callers that
    /// want a finished message in one shot. `req.stream` is forced off
    /// regardless of what the caller left in it. Tool arguments stay raw
    /// JSON strings — the router (Phase 5b) parses them against the
    /// schema, never this layer.
    pub async fn complete_chat(
        &self,
        req: ChatRequest,
    ) -> Result<Option<ChatMessage>, SeloraError> {
        let url = self.chat_completions_url();
        let mut body = req.with_stream(false);
        body.reasoning_effort = body
            .reasoning_effort
            .as_deref()
            .and_then(crate::types::normalize_reasoning_effort);
        let mut adapt = PayloadAdaptation::for_request(&self.base_url, &body);
        let mut param_retries = 0;

        let response = loop {
            let payload = (!adapt.is_passthrough(&body)).then(|| build_chat_payload(&body, &adapt));
            match self
                .send_with_retry(
                    "chat completion",
                    Some(body.model.as_str()),
                    self.request_timeout,
                    || {
                        let req_builder = self.apply_provider_headers(self.http.post(&url));
                        if let Some(ref custom_payload) = payload {
                            req_builder.json(custom_payload)
                        } else {
                            req_builder.json(&body)
                        }
                    },
                )
                .await
            {
                Ok(response) => break response,
                Err(err) if param_retries < 4 && adapt.adapt_on_rejection(&err.to_string()) => {
                    param_retries += 1;
                }
                Err(err) => return Err(err),
            }
        };
        let reply: CompletionResponse = self.read_json(response, "chat completion").await?;

        Ok(reply
            .choices
            .into_iter()
            .next()
            .and_then(|choice| choice.message)
            .map(|message| {
                let mut reply = ChatMessage::assistant(message.content.unwrap_or_default());
                reply.tool_calls = message.tool_calls;
                reply
            }))
    }

    /// Run a non-streaming minimal benign tool-call probe via [`Self::complete_chat`]
    /// to verify whether `model` accepts a tool manifest and emits `tool_calls`.
    ///
    /// Uses a 160-token budget (32 tokens was insufficient for real endpoints
    /// such as NVIDIA Mistral). Treats 404/410/capability errors or missing
    /// `tool_calls` as failed validation, never as proof of support.
    pub async fn probe_tool_capability(&self, model: &str) -> Result<ToolProbeOutcome, SeloraError> {
        let probe_tool = ToolSpec::new(
            "selora_probe_echo",
            "Return the provided text verbatim to verify tool calling support.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "Short confirmation word."
                    }
                },
                "required": ["text"],
                "additionalProperties": false
            }),
        );
        let mut req = ChatRequest::new(
            model,
            vec![
                ChatMessage::system(
                    "You are verifying tool calling capability. Call the `selora_probe_echo` tool once with {\"text\":\"ok\"}. Do not answer with plain text.",
                ),
                ChatMessage::user(
                    "Verify tool calling by invoking selora_probe_echo with text \"ok\".",
                ),
            ],
        )
        .with_tools(vec![probe_tool])
        .with_temperature(0.0);
        req.max_tokens = Some(160);

        let first = self.complete_chat(req.clone()).await;
        let outcome = match first {
            Ok(Some(message)) => {
                if message
                    .tool_calls
                    .as_ref()
                    .is_some_and(|calls| !calls.is_empty())
                {
                    Ok(ToolProbeOutcome::Supported)
                } else {
                    // Reasoning models often burn a 160-token probe budget on internal
                    // reasoning tokens before emitting `tool_calls`. Retry once with 1024.
                    let (_, visible) = split_leading_think_blocks(&message.content);
                    if message.content.trim().is_empty()
                        || message.content.contains("<think")
                        || visible.chars().count() < 16
                    {
                        let mut expanded = req;
                        expanded.max_tokens = Some(1024);
                        match self.complete_chat(expanded).await {
                            Ok(Some(retry_msg))
                                if retry_msg
                                    .tool_calls
                                    .as_ref()
                                    .is_some_and(|calls| !calls.is_empty()) =>
                            {
                                return Ok(ToolProbeOutcome::Supported);
                            }
                            Ok(_) => {}
                            Err(err) => return classify_probe_error(err, &self.base_url),
                        }
                    }
                    Ok(ToolProbeOutcome::Inconclusive(format!(
                        "Model '{model}' at {} responded without emitting a tool call during capability verification.",
                        safe_provider_label(&self.base_url)
                    )))
                }
            }
            Ok(None) => Ok(ToolProbeOutcome::Inconclusive(format!(
                "Provider {} returned no choices when verifying tool capability for model '{model}'.",
                safe_provider_label(&self.base_url)
            ))),
            Err(err) => return classify_probe_error(err, &self.base_url),
        };
        outcome
    }

    /// A cheap reachability ping (ARCHITECTURE.md §10.6): `HEAD {base_url}`,
    /// no retries, no body. Any HTTP response — even an error status —
    /// means the gateway is reachable; only a transport failure is
    /// "disconnected". The latency rides along for the status bar.
    pub async fn status(&self) -> SeloraStatus {
        let started = Instant::now();
        let ping = self.http.head(&self.base_url).send();
        match tokio::time::timeout(STATUS_TIMEOUT, ping).await {
            Ok(Ok(_)) => SeloraStatus::Connected {
                model: None,
                latency_ms: started.elapsed().as_millis() as u32,
            },
            // A timeout or a transport failure both mean "not reachable right now".
            Ok(Err(_)) | Err(_) => SeloraStatus::Disconnected,
        }
    }

    /// Send a request under the §6.5 retry policy, bounded by `budget`:
    ///
    /// * transport error → up to [`Self::max_net_retries`] retries with
    ///   exponential backoff and jitter;
    /// * 408/429/5xx → up to [`Self::max_http_retries`] retries on the same
    ///   capped backoff;
    /// * 401/403 → no retry, `SeloraError::Auth`;
    /// * any other 4xx → no retry, surface the gateway's message;
    /// * the whole exchange (including backoff sleeps) is bounded by
    ///   `budget`; an overrun is a typed timeout, never a hang.
    ///
    /// `build_request` is re-invoked per attempt because `RequestBuilder`
    /// is not reusable. Retries happen on one logical request, so a user turn
    /// is never duplicated.
    async fn send_with_retry(
        &self,
        what: &str,
        model: Option<&str>,
        budget: Duration,
        build_request: impl Fn() -> reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, SeloraError> {
        let mut network_retries = 0;
        let mut http_retries = 0;
        let deadline = tokio::time::Instant::now() + budget;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(timeout_error(what, budget));
            }
            let sent = tokio::time::timeout(remaining, build_request().send()).await;
            match sent {
                Err(_) => return Err(timeout_error(what, budget)),
                Ok(Ok(response)) => {
                    let status = response.status();
                    if status.is_success() {
                        return Ok(response);
                    }
                    if status.as_u16() == 401 || status.as_u16() == 403 {
                        let body = self.error_body(response).await;
                        return Err(self.format_http_error(what, model, status, &body));
                    }
                    if is_transient_status(status) {
                        if http_retries < self.max_http_retries {
                            let delay =
                                with_jitter(retry_delay(self.http_retry_delay, http_retries));
                            http_retries += 1;
                            if !sleep_within(deadline, delay).await {
                                return Err(timeout_error(what, budget));
                            }
                            continue;
                        }
                        let body = self.error_body(response).await;
                        return Err(SeloraError::Ai(format!(
                            "gateway unavailable at {} (HTTP {status}){body}",
                            safe_provider_label(&self.base_url)
                        )));
                    }
                    let body = self.error_body(response).await;
                    return Err(self.format_http_error(what, model, status, &body));
                }
                Ok(Err(err)) => {
                    if is_transient_transport(&err) && network_retries < self.max_net_retries {
                        let delay = with_jitter(retry_delay(self.net_retry_delay, network_retries));
                        network_retries += 1;
                        if !sleep_within(deadline, delay).await {
                            return Err(timeout_error(what, budget));
                        }
                        continue;
                    }
                    if err.is_timeout() {
                        return Err(timeout_error(what, budget));
                    }
                    return Err(SeloraError::Net(format!(
                        "cannot reach the AI gateway ({}): request failed",
                        safe_provider_label(&self.base_url)
                    )));
                }
            }
        }
    }

    fn format_http_error(
        &self,
        what: &str,
        model: Option<&str>,
        status: reqwest::StatusCode,
        body: &str,
    ) -> SeloraError {
        let provider = safe_provider_label(&self.base_url);
        let model_part = model
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(|m| format!(" for model '{m}'"))
            .unwrap_or_default();

        if status.as_u16() == 401 || status.as_u16() == 403 {
            return SeloraError::Auth(format!(
                "key invalid or not authorized at {provider} (HTTP {status}{body})"
            ));
        }

        if status == reqwest::StatusCode::GONE {
            return SeloraError::Ai(format!(
                "gateway rejected request{model_part} at {provider} (HTTP {status}){body}. \
                 This model is retired/end-of-life and cannot be used. \
                 Refresh the model catalog and select an active model instead."
            ));
        }

        if status == reqwest::StatusCode::NOT_FOUND {
            if what == "model catalog" {
                return SeloraError::Ai(format!(
                    "gateway rejected model catalog request at {provider} (HTTP {status}){body}. \
                     Check the provider base URL."
                ));
            }
            return SeloraError::Ai(format!(
                "gateway rejected request{model_part} at {provider} (HTTP {status}){body}. \
                 The selected model or deployment is not available on this provider account at /v1/chat/completions \
                 (the account may lack entitlement/access, or the provider's model/function mapping may be stale). \
                 Refresh the model catalog, verify model entitlement/account access, and choose a chat-capable model confirmed available for this account."
            ));
        }

        if (status == reqwest::StatusCode::BAD_REQUEST
            || status == reqwest::StatusCode::UNPROCESSABLE_ENTITY)
            && is_tool_capability_error_body(body)
        {
            return SeloraError::Ai(format!(
                "gateway rejected request{model_part} at {provider} (HTTP {status}){body}. \
                 Tool calling is not supported by this model or provider endpoint. \
                 Choose a tool-capable model for agent turns, or use plain chat."
            ));
        }

        SeloraError::Ai(format!(
            "gateway rejected request{model_part} at {provider} (HTTP {status}){body}"
        ))
    }

    /// One bounded GET, used by the Ollama fallback probes. No retries: a
    /// fallback that hangs would defeat the timeout it exists beside.
    async fn bounded_get(&self, url: &str) -> Result<reqwest::Response, SeloraError> {
        match tokio::time::timeout(STATUS_TIMEOUT, self.http.get(url).send()).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(SeloraError::Net(
                "cannot reach the AI gateway: fallback request failed".into(),
            )),
            Err(_) => Err(timeout_error("model catalog", STATUS_TIMEOUT)),
        }
    }

    /// Read a response body as text under the error-body deadline.
    async fn read_text(
        &self,
        response: reqwest::Response,
        what: &str,
    ) -> Result<String, SeloraError> {
        match tokio::time::timeout(ERROR_BODY_TIMEOUT, response.text()).await {
            Ok(Ok(text)) => Ok(text),
            Ok(Err(err)) => Err(SeloraError::Ai(format!("invalid {what} response: {err}"))),
            Err(_) => Err(timeout_error(what, ERROR_BODY_TIMEOUT)),
        }
    }

    /// Parse a JSON response body under the error-body deadline.
    async fn read_json<T: serde::de::DeserializeOwned>(
        &self,
        response: reqwest::Response,
        what: &str,
    ) -> Result<T, SeloraError> {
        match tokio::time::timeout(ERROR_BODY_TIMEOUT, response.json::<T>()).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(err)) => Err(SeloraError::Ai(format!("invalid {what} response: {err}"))),
            Err(_) => Err(timeout_error(what, ERROR_BODY_TIMEOUT)),
        }
    }

    /// Read a (bounded, redacted) error body for inclusion in an error
    /// message. The gateway's own words are surfaced, never paraphrased
    /// (RISKS.md §8) — but the API key is scrubbed if a proxy echoes it back.
    async fn error_body(&self, response: reqwest::Response) -> String {
        let text = match tokio::time::timeout(ERROR_BODY_TIMEOUT, response.text()).await {
            Ok(Ok(text)) => text,
            // A slow or unreadable error body must not turn a clear failure
            // into a hang; the status code still identifies it.
            Ok(Err(_)) | Err(_) => return String::new(),
        };
        let text = text.trim();
        if text.is_empty() {
            return String::new();
        }
        let truncated: String = text.chars().take(MAX_ERROR_BODY_CHARS).collect();
        format!(": {}", self.redact(&truncated))
    }

    /// Replace any occurrence of the API key in `text` with `[REDACTED]`.
    fn redact(&self, text: &str) -> String {
        if self.api_key.is_empty() {
            text.to_owned()
        } else {
            text.replace(&self.api_key, "[REDACTED]")
        }
    }
}

/// Send an event, failing loudly if the UI dropped the channel: continuing to
/// stream into a dead receiver would burn gateway tokens for nothing.
fn send_event(tx: &mpsc::UnboundedSender<ChatEvent>, event: ChatEvent) -> Result<(), SeloraError> {
    tx.send(event)
        .map_err(|_| SeloraError::Ai("chat event receiver dropped".into()))
}

/// `SeloraError` is not `Clone` (it wraps `io::Error`, which is not either),
/// and `selora-core` is not ours to change. The stream needs the same error
/// twice — once on the channel, once as the return value — so we duplicate it
/// here, preserving the variant and the message. The `Io` variant is the only
/// lossy one: it re-wraps as a plain message-bearing `io::Error`.
fn duplicate_error(err: &SeloraError) -> SeloraError {
    match err {
        SeloraError::Io(_) => SeloraError::Io(std::io::Error::other(err.to_string())),
        SeloraError::Net(message) => SeloraError::Net(message.clone()),
        SeloraError::Lsp(message) => SeloraError::Lsp(message.clone()),
        SeloraError::Git(message) => SeloraError::Git(message.clone()),
        SeloraError::Ai(message) => SeloraError::Ai(message.clone()),
        SeloraError::Auth(message) => SeloraError::Auth(message.clone()),
        SeloraError::Config(message) => SeloraError::Config(message.clone()),
        SeloraError::Other(message) => SeloraError::Other(message.clone()),
    }
}

/// Build the receipt from the gateway's `usage` (exact) or chars/4
/// (estimated) — RISKS.md §3.
fn build_receipt(
    req: &ChatRequest,
    request_id: Uuid,
    started: Instant,
    usage: Option<Usage>,
    prompt_chars: usize,
    completion_chars: usize,
) -> ChatReceipt {
    let estimated = match &usage {
        Some(usage) => usage.prompt_tokens.is_none() || usage.completion_tokens.is_none(),
        None => true,
    };
    let prompt_tokens = usage
        .as_ref()
        .and_then(|usage| usage.prompt_tokens)
        .unwrap_or((prompt_chars / 4) as u32);
    let completion_tokens = usage
        .as_ref()
        .and_then(|usage| usage.completion_tokens)
        .unwrap_or((completion_chars / 4) as u32);
    let cost_usd = usage.and_then(|usage| usage.cost_usd).unwrap_or(0.0);

    ChatReceipt {
        request_id,
        model: req.model.clone(),
        prompt_tokens,
        completion_tokens,
        cost_usd,
        duration_ms: started.elapsed().as_millis() as u32,
        estimated,
    }
}

// ---- Private wire types (not part of the public surface) ----

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    #[serde(default, alias = "models")]
    data: Vec<Model>,
}

/// Lenient multi-format parser for model catalogs.
pub fn parse_models_from_json(text: &str) -> Vec<Model> {
    // 1. Try ModelsResponse ({ data: [...] } or { models: [...] })
    if let Ok(resp) = serde_json::from_str::<ModelsResponse>(text) {
        if !resp.data.is_empty() {
            return resp.data;
        }
    }
    // 2. Try raw array of Model
    if let Ok(list) = serde_json::from_str::<Vec<Model>>(text) {
        if !list.is_empty() {
            return list;
        }
    }
    // 3. Fallback: inspect arbitrary JSON Value
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
        let items_opt = v.as_array().or_else(|| {
            v.get("data")
                .and_then(|d| d.as_array())
                .or_else(|| v.get("models").and_then(|m| m.as_array()))
        });

        if let Some(items) = items_opt {
            let mut result = Vec::new();
            for item in items {
                let id = item
                    .get("id")
                    .or_else(|| item.get("name"))
                    .or_else(|| item.get("model"))
                    .and_then(|v| v.as_str());
                if let Some(id_str) = id {
                    let id_str = id_str.trim();
                    if id_str.is_empty() {
                        continue;
                    }
                    let display_name = item
                        .get("display_name")
                        .or_else(|| item.get("displayName"))
                        .or_else(|| item.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or(id_str)
                        .trim();

                    let context_window = item
                        .get("context_window")
                        .or_else(|| item.get("context_length"))
                        .or_else(|| item.get("max_context_length"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(128_000) as u32;

                    let family = if id_str.contains("claude") {
                        "claude".to_string()
                    } else if id_str.contains("gpt")
                        || id_str.contains("o1")
                        || id_str.contains("o3")
                    {
                        "gpt".to_string()
                    } else if id_str.contains("deepseek") {
                        "deepseek".to_string()
                    } else if id_str.contains("glm") {
                        "glm".to_string()
                    } else if id_str.contains("llama") {
                        "llama".to_string()
                    } else {
                        "default".to_string()
                    };

                    let tool_capability =
                        extract_explicit_tool_capability(item).unwrap_or(ToolCapability::Unknown);
                    let reasoning_capability =
                        crate::types::extract_explicit_reasoning_capability(item)
                            .unwrap_or_else(|| crate::types::infer_reasoning_capability(id_str));
                    let available = item
                        .get("available")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(true);

                    result.push(Model {
                        id: id_str.to_string(),
                        display_name: if display_name.is_empty() {
                            id_str.to_string()
                        } else {
                            display_name.to_string()
                        },
                        context_window,
                        price_per_1k_input: 0.0,
                        price_per_1k_output: 0.0,
                        supports_streaming: true,
                        tool_capability,
                        reasoning_capability,
                        available,
                        family,
                    });
                }
            }
            return result;
        }
    }
    Vec::new()
}

/// One streamed chat chunk (OpenAI-compatible shape).
#[derive(Debug, Deserialize)]
struct ChatChunk {
    #[serde(default)]
    choices: Vec<ChunkChoice>,
    #[serde(default)]
    usage: Option<Usage>,
}

/// One non-streaming completion reply (ghost text, ARCHITECTURE.md §10.6).
#[derive(Debug, Deserialize)]
struct CompletionResponse {
    #[serde(default)]
    choices: Vec<CompletionChoice>,
}

#[derive(Debug, Deserialize)]
struct CompletionChoice {
    #[serde(default)]
    message: Option<CompletionMessage>,
}

#[derive(Debug, Deserialize)]
struct CompletionMessage {
    #[serde(default)]
    content: Option<String>,
    /// Complete tool calls on a non-streaming assistant message
    /// (ARCHITECTURE.md §17.6) — parsed by [`ToolCall`]'s tolerant
    /// deserializer in the OpenAI nested shape.
    #[serde(default)]
    tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Deserialize)]
struct ChunkChoice {
    #[serde(default)]
    delta: Option<ChunkDelta>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    message: Option<CompletionMessage>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReasoningDetail {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    summary: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChunkDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    thought: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    reasoning_details: Option<Vec<ReasoningDetail>>,
    #[serde(default)]
    text: Option<String>,
    /// Streaming tool-call fragments (the OpenAI `delta.tool_calls[]`
    /// array) — emitted as [`ChatEvent::ToolCallDelta`] for the caller's
    /// [`crate::ToolCallAccumulator`] to reassemble.
    #[serde(default)]
    tool_calls: Option<Vec<ToolCallFragment>>,
}

impl ChunkDelta {
    fn reasoning_text(&self) -> Option<String> {
        for candidate in [
            self.reasoning_content.as_deref(),
            self.reasoning.as_deref(),
            self.thought.as_deref(),
            self.thinking.as_deref(),
        ] {
            if let Some(text) = candidate.filter(|s| !s.is_empty()) {
                return Some(text.to_owned());
            }
        }
        if let Some(details) = self.reasoning_details.as_deref() {
            let mut combined = String::new();
            for item in details {
                if let Some(piece) = item
                    .text
                    .as_deref()
                    .or(item.summary.as_deref())
                    .filter(|s| !s.is_empty())
                {
                    combined.push_str(piece);
                }
            }
            if !combined.is_empty() {
                return Some(combined);
            }
        }
        None
    }
}

/// One `delta.tool_calls[]` fragment: `{index?, id?, function:{name?,
/// arguments?}}`. Every field may be absent — the gateway sends the id
/// and name on the first fragment of a call and the arguments across
/// many. An absent `index` (single-call gateway quirk) resolves to 0.
#[derive(Debug, Deserialize)]
struct ToolCallFragment {
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<FragmentFunction>,
}

#[derive(Debug, Deserialize)]
struct FragmentFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

/// Usage block on the final chunk. Absent → chars/4 estimate (RISKS.md §3).
#[derive(Debug, Clone, Deserialize)]
struct Usage {
    #[serde(default)]
    prompt_tokens: Option<u32>,
    #[serde(default)]
    completion_tokens: Option<u32>,
    #[serde(default, alias = "cost")]
    cost_usd: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_types::{ToolCall, ToolCallAccumulator, ToolSpec};
    use crate::sse::MAX_DATA_FIELD_BYTES;
    use crate::types::{ChatMessage, Role};

    use wiremock::matchers::{body_json, body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const TEST_KEY: &str = "sk-gw-test-key-000000000000";

    fn test_client(base_url: &str) -> SeloraClient {
        let mut client = SeloraClient::new(base_url, TEST_KEY);
        // Same retry policy, millisecond delays: retries stay observable
        // without slowing the suite down.
        client.net_retry_delay = Duration::from_millis(2);
        client.http_retry_delay = Duration::from_millis(2);
        client
    }

    fn chat_request() -> ChatRequest {
        // "1234" → 4 chars → 1 estimated prompt token.
        ChatRequest::new("test-model", vec![ChatMessage::user("1234")])
    }

    // A short Content-Length body makes reqwest report a body decoding error.
    fn interrupted_server(
        first_body: &'static str,
        retry: bool,
    ) -> (String, std::thread::JoinHandle<usize>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let count = if retry { 2 } else { 1 };
            for attempt in 0..count {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut chunk = [0; 4096];
                loop {
                    let n = socket.read(&mut chunk).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&chunk[..n]);
                    if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]);
                        let length: usize = headers
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse().unwrap())
                            })
                            .unwrap_or(0);
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let body = if attempt == 0 {
                    first_body
                } else {
                    "data: {\"choices\":[{\"delta\":{\"content\":\"recovered\"}}]}\n\ndata: [DONE]\n\n"
                };
                let length = if attempt == 0 {
                    body.len() + 100
                } else {
                    body.len()
                };
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n{body}").unwrap();
            }
            count
        });
        (format!("http://{address}"), handle)
    }

    #[tokio::test]
    async fn interrupted_stream_before_output_recovers_without_duplicate_events() {
        let (url, server) = interrupted_server(
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n",
            true,
        );
        let (tx, mut rx) = mpsc::unbounded_channel();
        test_client(&url)
            .stream_chat(chat_request(), tx)
            .await
            .unwrap();
        let events = drain(&mut rx);
        assert_eq!(deltas(&events), ["recovered"]);
        assert!(matches!(events.last(), Some(ChatEvent::Done(_))));
        assert_eq!(server.join().unwrap(), 2);
    }

    #[tokio::test]
    async fn interrupted_stream_after_output_never_replays() {
        let (url, server) = interrupted_server(
            "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n",
            false,
        );
        let (tx, mut rx) = mpsc::unbounded_channel();
        let error = test_client(&url)
            .stream_chat(chat_request(), tx)
            .await
            .unwrap_err();
        assert!(matches!(error, SeloraError::Net(_)));
        let events = drain(&mut rx);
        assert_eq!(deltas(&events), ["partial"]);
        assert!(matches!(events.last(), Some(ChatEvent::Error(_))));
        assert_eq!(server.join().unwrap(), 1);
    }

    fn drain(rx: &mut mpsc::UnboundedReceiver<ChatEvent>) -> Vec<ChatEvent> {
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        events
    }

    fn deltas(events: &[ChatEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                ChatEvent::Delta(delta) => Some(delta.clone()),
                _ => None,
            })
            .collect()
    }

    fn models_json() -> serde_json::Value {
        serde_json::json!({
            "object": "list",
            "data": [
                {
                    "id": "claude-opus-5-5",
                    "display_name": "Claude Opus 5.5",
                    "context_window": 200000,
                    "price_per_1k_input": 0.015,
                    "price_per_1k_output": 0.075,
                    "supports_streaming": true,
                    "supports_tools": true,
                    "family": "claude"
                },
                { "id": "glm-5.3" }
            ]
        })
    }

    #[tokio::test]
    async fn list_models_returns_models_with_bearer_auth() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", format!("Bearer {TEST_KEY}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(models_json()))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let models = client.list_models().await.expect("list_models succeeds");

        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "claude-opus-5-5");
        assert_eq!(models[0].price_per_1k_output, 0.075);
        assert_eq!(models[0].family, "claude");
        // Missing display_name falls back to the id.
        assert_eq!(models[1].display_name, "glm-5.3");
    }

    #[tokio::test]
    async fn list_models_invalid_json_is_an_ai_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let err = client.list_models().await.unwrap_err();
        assert!(matches!(&err, SeloraError::Ai(msg) if msg.contains("models response")));
    }

    #[tokio::test]
    async fn stream_chat_happy_path_streams_deltas_and_returns_receipt() {
        let server = MockServer::start().await;
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":2,\"cost_usd\":0.0013}}\n\n",
            "data: [DONE]\n\n",
        );
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            // The wire request must carry stream: true.
            .and(body_partial_json(
                serde_json::json!({ "model": "test-model", "stream": true }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let receipt = client
            .stream_chat(chat_request(), tx)
            .await
            .expect("stream_chat succeeds");

        let events = drain(&mut rx);
        assert_eq!(deltas(&events), ["Hello", " world"]);
        assert!(matches!(events.last(), Some(ChatEvent::Done(_))));

        assert_eq!(receipt.model, "test-model");
        assert_eq!(receipt.prompt_tokens, 9);
        assert_eq!(receipt.completion_tokens, 2);
        assert_eq!(receipt.cost_usd, 0.0013);
        assert!(!receipt.estimated, "gateway sent exact usage");
        assert!(!receipt.request_id.is_nil());
        assert!(matches!(events.last(), Some(ChatEvent::Done(done)) if *done == receipt));
    }

    #[tokio::test]
    async fn stream_chat_reasoning_content_wrapped_in_think_tags() {
        let server = MockServer::start().await;
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking step 1\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"direct answer\"}}]}\n\n",
            "data: [DONE]\n\n",
        );
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _receipt = client
            .stream_chat(chat_request(), tx)
            .await
            .expect("stream_chat succeeds");

        let events = drain(&mut rx);
        let joined: String = deltas(&events).join("");
        assert!(joined.contains("<think>thinking step 1</think>\n\ndirect answer"));
    }

    #[tokio::test]
    async fn stream_chat_without_usage_estimates_tokens_from_chars() {
        let server = MockServer::start().await;
        // 8 output chars → 2 estimated completion tokens; no usage block.
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"abcd\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"abcd\"}}]}\n\n",
            "data: [DONE]\n\n",
        );
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let receipt = client
            .stream_chat(chat_request(), tx)
            .await
            .expect("stream_chat succeeds");

        assert!(receipt.estimated, "no usage → estimated");
        // Prompt "1234" → 1 token; completion "abcdabcd" → 2 tokens (chars/4).
        assert_eq!(receipt.prompt_tokens, 1);
        assert_eq!(receipt.completion_tokens, 2);
        assert_eq!(receipt.cost_usd, 0.0);
        assert_eq!(deltas(&drain(&mut rx)), ["abcd", "abcd"]);
    }

    #[tokio::test]
    async fn stream_chat_401_maps_to_auth_error_with_no_retry() {
        let server = MockServer::start().await;
        let _mock = Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
            .expect(1) // 401 is never retried (ARCHITECTURE.md §6.5)
            .mount_as_scoped(&server)
            .await;

        let client = test_client(&server.uri());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let err = client.stream_chat(chat_request(), tx).await.unwrap_err();

        assert!(
            matches!(&err, SeloraError::Auth(msg) if msg.contains("401")),
            "expected Auth(401), got: {err:?}"
        );
        let events = drain(&mut rx);
        assert!(matches!(events.first(), Some(ChatEvent::Error(_))));
    }

    #[tokio::test]
    async fn stream_chat_502_retries_three_times_then_fails() {
        let server = MockServer::start().await;
        let _mock = Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(502).set_body_string("bad gateway"))
            .expect(4) // 1 attempt + 3 retries (ARCHITECTURE.md §6.5)
            .mount_as_scoped(&server)
            .await;

        let client = test_client(&server.uri());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let err = client.stream_chat(chat_request(), tx).await.unwrap_err();

        assert!(
            matches!(&err, SeloraError::Ai(msg) if msg.contains("502") && msg.contains("bad gateway")),
            "expected Ai(502), got: {err:?}"
        );
        assert!(matches!(drain(&mut rx).first(), Some(ChatEvent::Error(_))));
    }

    #[tokio::test]
    async fn stream_chat_other_4xx_surfaces_gateway_message_without_retry() {
        let server = MockServer::start().await;
        let _mock = Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(400).set_body_string("model `nope` not found"))
            .expect(1)
            .mount_as_scoped(&server)
            .await;

        let client = test_client(&server.uri());
        let (tx, _rx) = mpsc::unbounded_channel();
        let err = client.stream_chat(chat_request(), tx).await.unwrap_err();

        assert!(
            matches!(&err, SeloraError::Ai(msg) if msg.contains("model `nope` not found")),
            "gateway's own words must surface, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn stream_chat_404_explains_provider_model_account_mismatch() {
        let server = MockServer::start().await;
        let _mock = Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(404).set_body_string(
                "{\"status\":404,\"title\":\"Not Found\",\"detail\":\"Function 'internal-id': Not found for account 'account-id'\"}",
            ))
            .expect(1)
            .mount_as_scoped(&server)
            .await;

        let client = test_client(&server.uri());
        let (tx, _rx) = mpsc::unbounded_channel();
        let err = client.stream_chat(chat_request(), tx).await.unwrap_err();
        let message = err.to_string();

        assert!(message.contains("Function 'internal-id'"), "provider body: {message}");
        assert!(
            message.contains("not available on this provider account"),
            "account guidance: {message}"
        );
        assert!(
            message.contains("chat-capable model"),
            "model guidance: {message}"
        );
    }

    #[tokio::test]
    async fn stream_chat_missing_done_is_a_stream_error() {
        let server = MockServer::start().await;
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n";
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let err = client.stream_chat(chat_request(), tx).await.unwrap_err();

        assert!(
            matches!(&err, SeloraError::Ai(msg) if msg.contains("closed before [DONE]")),
            "expected missing-[DONE] error, got: {err:?}"
        );
        let events = drain(&mut rx);
        assert_eq!(deltas(&events), ["Hi"]);
        assert!(matches!(events.last(), Some(ChatEvent::Error(_))));
    }

    #[tokio::test]
    async fn stream_chat_oversized_chunk_aborts_the_stream() {
        let server = MockServer::start().await;
        let mut sse = String::from("data: ");
        sse.push_str(&"x".repeat(MAX_DATA_FIELD_BYTES + 1));
        sse.push_str("\n\n");
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let (tx, _rx) = mpsc::unbounded_channel();
        let err = client.stream_chat(chat_request(), tx).await.unwrap_err();
        assert!(
            matches!(&err, SeloraError::Ai(msg) if msg.contains("oversized")),
            "expected oversized-chunk error, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn ghost_complete_returns_the_trimmed_first_choice() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            // Ghost completions are non-streaming on the wire.
            .and(body_partial_json(
                serde_json::json!({ "model": "glm-5.3", "stream": false }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [
                    { "message": { "role": "assistant", "content": "  foo(bar);\n" } },
                    { "message": { "role": "assistant", "content": "ignored" } }
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let suggestion = client
            .ghost_complete("glm-5.3", "fn main() { /* caret */ ")
            .await
            .expect("ghost_complete succeeds");
        assert_eq!(suggestion.as_deref(), Some("foo(bar);"));
    }

    #[tokio::test]
    async fn ghost_complete_blank_reply_is_none_not_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [ { "message": { "content": "   " } } ]
            })))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        assert_eq!(
            client.ghost_complete("m", "prefix").await.expect("ok"),
            None
        );
    }

    #[tokio::test]
    async fn ghost_complete_no_choices_is_none() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": []
            })))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        assert_eq!(
            client.ghost_complete("m", "prefix").await.expect("ok"),
            None
        );
    }

    #[tokio::test]
    async fn ghost_complete_401_is_an_auth_error() {
        let server = MockServer::start().await;
        let _mock = Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount_as_scoped(&server)
            .await;

        let client = test_client(&server.uri());
        let err = client.ghost_complete("m", "prefix").await.unwrap_err();
        assert!(matches!(&err, SeloraError::Auth(msg) if msg.contains("401")));
    }

    #[tokio::test]
    async fn status_reports_connected_on_any_http_response() {
        // Even a 404 on HEAD means the gateway answered: reachable.
        let server = MockServer::start().await;
        Mock::given(method("HEAD"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        match client.status().await {
            SeloraStatus::Connected { model, latency_ms } => {
                assert_eq!(model, None);
                assert!(latency_ms < 60_000, "latency should be small: {latency_ms}");
            }
            other => panic!("expected Connected, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn status_reports_disconnected_on_transport_failure() {
        // Bind a port and drop the listener: connection refused.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let port = listener.local_addr().expect("local addr").port();
        drop(listener);

        let client = test_client(&format!("http://127.0.0.1:{port}"));
        assert_eq!(client.status().await, SeloraStatus::Disconnected);
    }

    #[tokio::test]
    async fn list_models_502_then_success_recovers() {
        let server = MockServer::start().await;
        // Mounted first → matched after the flaky one exhausts itself.
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(models_json()))
            .mount(&server)
            .await;
        // Mounted last → tried first (wiremock matches newest first), exactly once.
        // The guard must stay bound: dropping it early would unmount the mock.
        let _flaky = Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(502))
            .up_to_n_times(1)
            .mount_as_scoped(&server)
            .await;

        let client = test_client(&server.uri());
        let models = client.list_models().await.expect("recovers after one 502");
        assert_eq!(models.len(), 2);
    }

    #[tokio::test]
    async fn network_error_is_retried_once_then_surfaced() {
        // Bind a port and drop the listener: nothing is listening there, so
        // the connection is refused — a network error, not an HTTP status.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let port = listener.local_addr().expect("local addr").port();
        drop(listener);

        let client = test_client(&format!("http://127.0.0.1:{port}"));
        let err = client.list_models().await.unwrap_err();
        assert!(
            matches!(&err, SeloraError::Net(msg) if msg.contains("cannot reach the AI gateway")),
            "expected a readable Net error, got: {err:?}"
        );
        assert_eq!(classify_failure(&err), GatewayFailure::Network);
        assert!(!is_timeout(&err));
        let message = user_message(&err);
        assert!(
            message.contains("internet connection"),
            "network copy must be actionable: {message}"
        );
        assert!(!message.contains(TEST_KEY), "key leaked: {message}");
    }

    #[test]
    fn backoff_is_capped_exponential_and_jitter_stays_bounded() {
        let base = Duration::from_secs(1);
        assert_eq!(retry_delay(base, 0), Duration::from_secs(1));
        assert_eq!(retry_delay(base, 1), Duration::from_secs(2));
        assert_eq!(retry_delay(base, 2), Duration::from_secs(4));
        assert_eq!(retry_delay(base, 3), Duration::from_secs(8));
        // The §6.5 ceiling holds for every later attempt.
        assert_eq!(retry_delay(base, 9), MAX_RETRY_DELAY);
        assert_eq!(retry_delay(base, usize::MAX), MAX_RETRY_DELAY);

        let delay = Duration::from_millis(2000);
        for _ in 0..32 {
            let jittered = with_jitter(delay);
            assert!(jittered >= delay, "jitter must only add: {jittered:?}");
            assert!(
                jittered <= delay + delay * RETRY_JITTER_PERCENT as u32 / 100,
                "jitter exceeded its cap: {jittered:?}"
            );
        }
    }

    #[tokio::test]
    async fn stream_chat_403_is_an_auth_error_with_no_retry() {
        let server = MockServer::start().await;
        let _mock = Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(403).set_body_string("key not allowed"))
            .expect(1)
            .mount_as_scoped(&server)
            .await;

        let client = test_client(&server.uri());
        let (tx, _rx) = mpsc::unbounded_channel();
        let err = client.stream_chat(chat_request(), tx).await.unwrap_err();

        assert!(matches!(&err, SeloraError::Auth(msg) if msg.contains("403")));
        assert_eq!(classify_failure(&err), GatewayFailure::Auth);
        assert!(
            user_message(&err).contains("Check the key in Settings"),
            "bad-key copy must be actionable"
        );
    }

    #[tokio::test]
    async fn models_timeout_is_typed_and_bounded() {
        let server = MockServer::start().await;
        let _mock = Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(400))
                    .set_body_json(models_json()),
            )
            .expect(1)
            .mount_as_scoped(&server)
            .await;

        let mut client = test_client(&server.uri());
        client.request_timeout = Duration::from_millis(50);
        let started = Instant::now();
        let err = client.list_models().await.unwrap_err();
        let elapsed = started.elapsed();

        assert!(is_timeout(&err), "expected a typed timeout, got: {err:?}");
        assert_eq!(classify_failure(&err), GatewayFailure::Timeout);
        assert!(
            matches!(&err, SeloraError::Net(msg) if msg.starts_with(TIMEOUT_MARKER) && msg.contains("model catalog")),
            "timeout must name the request: {err:?}"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "the deadline must be enforced, took {elapsed:?}"
        );
        assert!(user_message(&err).contains("did not answer in time"));
    }

    #[tokio::test]
    async fn stream_chat_header_timeout_ends_the_turn_typed() {
        let server = MockServer::start().await;
        let _mock = Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(400))
                    .set_body_string("data: [DONE]\n\n"),
            )
            .expect(1)
            .mount_as_scoped(&server)
            .await;

        let mut client = test_client(&server.uri());
        client.stream_response_timeout = Duration::from_millis(50);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let err = client.stream_chat(chat_request(), tx).await.unwrap_err();

        assert!(is_timeout(&err), "expected a typed timeout, got: {err:?}");
        let events = drain(&mut rx);
        assert!(
            matches!(events.first(), Some(ChatEvent::Error(error)) if is_timeout(error)),
            "the timeout must be surfaced as a terminal error event: {events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, ChatEvent::Done(_))),
            "a timed-out turn must not report Done"
        );
    }

    #[tokio::test]
    async fn bad_request_is_never_retried() {
        let server = MockServer::start().await;
        let _mock = Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(400).set_body_string("bad model"))
            .expect(1)
            .mount_as_scoped(&server)
            .await;

        let client = test_client(&server.uri());
        let mut request = chat_request();
        request.messages.clear(); // a gateway would reject this
        let (tx, _rx) = mpsc::unbounded_channel();
        let err = client.stream_chat(request, tx).await.unwrap_err();
        assert_eq!(classify_failure(&err), GatewayFailure::Rejected);
    }

    #[tokio::test]
    async fn gateway_body_echoing_the_key_is_redacted_in_errors() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(
                ResponseTemplate::new(403)
                    .set_body_string(format!("key {TEST_KEY} is not allowed")),
            )
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let err = client.list_models().await.unwrap_err();
        let rendered = err.to_string();
        assert!(!rendered.contains(TEST_KEY), "key leaked: {rendered}");
        assert!(
            rendered.contains("[REDACTED]"),
            "expected redaction: {rendered}"
        );
    }

    #[test]
    fn debug_output_never_contains_the_api_key() {
        let client = SeloraClient::new("http://example.test/", TEST_KEY);
        let rendered = format!("{client:?}");
        assert!(!rendered.contains(TEST_KEY), "key leaked: {rendered}");
        assert!(rendered.contains("[REDACTED]"));
        assert!(rendered.contains("http://example.test")); // trailing / trimmed
    }

    // ---- Phase 5a: tool-call request plumbing (ARCHITECTURE.md §17.6) ----

    /// The two-tool manifest the scripted agent conversation arms.
    fn agent_manifest() -> Vec<ToolSpec> {
        vec![
            ToolSpec::new(
                "read_file",
                "Read a workspace file",
                serde_json::json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                }),
            ),
            ToolSpec::new(
                "write_file",
                "Write a workspace file",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" }
                    },
                    "required": ["path", "content"]
                }),
            ),
        ]
    }

    /// One streamed SSE chunk carrying a content delta.
    fn content_chunk(text: &str) -> serde_json::Value {
        serde_json::json!({ "choices": [ { "delta": { "content": text } } ] })
    }

    /// One streamed SSE chunk carrying a single `delta.tool_calls[]`
    /// fragment. Absent fields are sent as JSON `null` — the parser also
    /// handles fully omitted keys.
    fn fragment_chunk(
        index: Option<usize>,
        id: Option<&str>,
        name: Option<&str>,
        arguments: &str,
    ) -> serde_json::Value {
        serde_json::json!({
            "choices": [ {
                "delta": { "tool_calls": [ {
                    "index": index,
                    "id": id,
                    "type": "function",
                    "function": { "name": name, "arguments": arguments }
                } ] }
            } ]
        })
    }

    /// The terminal chunk: the gateway's `finish_reason`.
    fn finish_chunk(reason: &str) -> serde_json::Value {
        serde_json::json!({ "choices": [ { "delta": {}, "finish_reason": reason } ] })
    }

    /// Wrap streamed chunks into an SSE body terminated by `[DONE]`.
    fn sse_body(chunks: &[serde_json::Value]) -> String {
        let mut body = String::new();
        for chunk in chunks {
            body.push_str("data: ");
            body.push_str(&chunk.to_string());
            body.push_str("\n\n");
        }
        body.push_str("data: [DONE]\n\n");
        body
    }

    /// Stream one armed agent turn and return `(text, tool_calls)` —
    /// exactly what the Phase 5c loop does per iteration: relay events,
    /// accumulate tool-call fragments, drain at `Done`.
    async fn run_agent_turn(
        client: &SeloraClient,
        conversation: Vec<ChatMessage>,
    ) -> (String, Vec<ToolCall>) {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let receipt = client
            .stream_chat(
                ChatRequest::new("test-model", conversation).with_tools(agent_manifest()),
                tx,
            )
            .await
            .expect("turn streams successfully");

        let events = drain(&mut rx);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, ChatEvent::Error(_))),
            "no error events expected: {events:?}"
        );
        assert!(
            matches!(events.last(), Some(ChatEvent::Done(done)) if *done == receipt),
            "the stream must end with the returned receipt"
        );

        let mut accumulator = ToolCallAccumulator::new();
        for event in &events {
            accumulator.feed(event);
        }
        (deltas(&events).concat(), accumulator.finish())
    }

    /// The scripted 3-turn agent conversation (deliverable): turn 1 asks
    /// for a read, turn 2 for a write (arguments split mid-JSON across
    /// chunks), turn 3 answers with plain text. Every request body is
    /// exact-matched against the hand-written OpenAI wire shape — tools
    /// armed, assistant `tool_calls` replayed, tool results appended.
    #[tokio::test]
    async fn agent_three_turn_conversation_read_write_then_answer() {
        let server = MockServer::start().await;

        let user_prompt = "Read src/main.rs and add an entry-point comment.";
        let read_args = r#"{"path":"src/main.rs"}"#;
        let write_args = r#"{"path":"src/lib.rs","content":"// entry point\n"}"#;
        let read_result = "fn main() { println!(\"hello, selora\"); }";
        let write_result = "wrote 25 bytes to src/lib.rs";

        let read_call = ToolCall {
            id: "call_read_1".into(),
            name: "read_file".into(),
            arguments: read_args.into(),
        };
        let write_call = ToolCall {
            id: "call_write_1".into(),
            name: "write_file".into(),
            arguments: write_args.into(),
        };

        // The expected wire bodies, hand-written (not produced by the
        // serializer under test): the OpenAI `tools` array shape, the
        // assistant message's nested `tool_calls`, the tool-result
        // messages.
        let expected_tools = serde_json::json!([
            {
                "type": "function",
                "function": {
                    "name": "read_file",
                    "description": "Read a workspace file",
                    "parameters": {
                        "type": "object",
                        "properties": { "path": { "type": "string" } },
                        "required": ["path"]
                    }
                }
            },
            {
                "type": "function",
                "function": {
                    "name": "write_file",
                    "description": "Write a workspace file",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" },
                            "content": { "type": "string" }
                        },
                        "required": ["path", "content"]
                    }
                }
            }
        ]);
        let assistant_read = serde_json::json!({
            "role": "assistant",
            "content": "Let me read that.",
            "tool_calls": [
                { "id": "call_read_1", "type": "function",
                  "function": { "name": "read_file", "arguments": read_args } }
            ]
        });
        let assistant_write = serde_json::json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [
                { "id": "call_write_1", "type": "function",
                  "function": { "name": "write_file", "arguments": write_args } }
            ]
        });
        let turn_bodies = [
            serde_json::json!({
                "model": "test-model",
                "messages": [ { "role": "user", "content": user_prompt } ],
                "temperature": 0.7,
                "stream": true,
                "tools": expected_tools
            }),
            serde_json::json!({
                "model": "test-model",
                "messages": [
                    { "role": "user", "content": user_prompt },
                    assistant_read.clone(),
                    { "role": "tool", "tool_call_id": "call_read_1", "content": read_result }
                ],
                "temperature": 0.7,
                "stream": true,
                "tools": expected_tools.clone()
            }),
            serde_json::json!({
                "model": "test-model",
                "messages": [
                    { "role": "user", "content": user_prompt },
                    assistant_read,
                    { "role": "tool", "tool_call_id": "call_read_1", "content": read_result },
                    assistant_write,
                    { "role": "tool", "tool_call_id": "call_write_1", "content": write_result }
                ],
                "temperature": 0.7,
                "stream": true,
                "tools": expected_tools
            }),
        ];

        // Turn 1: text commentary, then a read_file call whose arguments
        // stream in three pieces (id+name first, then two argument
        // fragments), finish_reason "tool_calls".
        let turn1_sse = sse_body(&[
            content_chunk("Let me read that."),
            fragment_chunk(Some(0), Some("call_read_1"), Some("read_file"), ""),
            fragment_chunk(Some(0), None, None, "{\"path\""),
            fragment_chunk(Some(0), None, None, ":\"src/main.rs\"}"),
            finish_chunk("tool_calls"),
        ]);
        // Turn 2: a write_file call with arguments split mid-JSON, one
        // fragment missing its `index` (single-call gateway quirk), one
        // empty trailing fragment.
        let turn2_sse = sse_body(&[
            fragment_chunk(Some(0), Some("call_write_1"), Some("write_file"), ""),
            fragment_chunk(None, None, None, "{\"path\":\"src/lib.rs\",\"co"),
            fragment_chunk(Some(0), None, None, "ntent\":\"// entry point\\n\"}"),
            fragment_chunk(Some(0), None, None, ""),
            finish_chunk("tool_calls"),
        ]);
        // Turn 3: the final answer, finish_reason "stop", with usage.
        let turn3_sse = sse_body(&[
            content_chunk("Done — src/lib.rs now opens "),
            content_chunk("with `// entry point`."),
            serde_json::json!({
                "choices": [ { "delta": {}, "finish_reason": "stop" } ],
                "usage": { "prompt_tokens": 42, "completion_tokens": 9, "cost_usd": 0.0021 }
            }),
        ]);

        // Scoped guards live to the end of the test (each verifies its
        // expect(1) on drop — one exact-body hit per turn).
        let mut guards = Vec::new();
        for (body, sse) in std::iter::zip(turn_bodies, [turn1_sse, turn2_sse, turn3_sse]) {
            guards.push(
                Mock::given(method("POST"))
                    .and(path("/v1/chat/completions"))
                    .and(body_json(body))
                    .respond_with(ResponseTemplate::new(200).set_body_string(sse))
                    .expect(1)
                    .mount_as_scoped(&server)
                    .await,
            );
        }

        let client = test_client(&server.uri());
        let mut conversation = vec![ChatMessage::user(user_prompt)];

        // Turn 1 — the model asks for a read (finish_reason: tool_calls).
        let (text, calls) = run_agent_turn(&client, conversation.clone()).await;
        assert_eq!(text, "Let me read that.");
        assert_eq!(calls, vec![read_call]);
        let mut assistant = ChatMessage::assistant(text);
        assistant.tool_calls = Some(calls);
        conversation.push(assistant);
        conversation.push(ChatMessage::tool_result("call_read_1", read_result));

        // Turn 2 — the model asks for a write (arguments reassembled).
        let (text, calls) = run_agent_turn(&client, conversation.clone()).await;
        assert_eq!(text, "");
        assert_eq!(calls, vec![write_call]);
        let mut assistant = ChatMessage::assistant(text);
        assistant.tool_calls = Some(calls);
        conversation.push(assistant);
        conversation.push(ChatMessage::tool_result("call_write_1", write_result));

        // Turn 3 — the final answer (finish_reason: stop, no tool calls).
        let (text, calls) = run_agent_turn(&client, conversation).await;
        assert!(
            calls.is_empty(),
            "a stop turn must drain no tool calls: {calls:?}"
        );
        assert_eq!(text, "Done — src/lib.rs now opens with `// entry point`.");
    }

    /// Plain chat never arms tools: an exact body match with no `tools`
    /// key (any `tools` field on the wire would miss the mock and fail).
    #[tokio::test]
    async fn plain_chat_request_carries_no_tools_on_the_wire() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_json(serde_json::json!({
                "model": "test-model",
                "messages": [ { "role": "user", "content": "1234" } ],
                "temperature": 0.7,
                "stream": true
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(sse_body(&[content_chunk("hi"), finish_chunk("stop")])),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let receipt = client
            .stream_chat(chat_request(), tx)
            .await
            .expect("plain chat streams");
        assert_eq!(deltas(&drain(&mut rx)), ["hi"]);
        assert!(!receipt.request_id.is_nil());
    }

    /// The loop's tool_calls-vs-stop seam: `finish_reason: "tool_calls"`
    /// streams drain into calls (and at most commentary text);
    /// `finish_reason: "stop"` streams drain nothing and the text is the
    /// final answer.
    #[tokio::test]
    async fn finish_reason_distinguishes_tool_calls_from_stop() {
        let tool_calls_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse_body(&[
                fragment_chunk(
                    Some(0),
                    Some("call_1"),
                    Some("read_file"),
                    r#"{"path":"a.rs"}"#,
                ),
                finish_chunk("tool_calls"),
            ])))
            .expect(1)
            .mount(&tool_calls_server)
            .await;
        let client = test_client(&tool_calls_server.uri());
        let (text, calls) = run_agent_turn(&client, vec![ChatMessage::user("read a.rs")]).await;
        assert_eq!(text, "");
        assert_eq!(
            calls,
            vec![ToolCall {
                id: "call_1".into(),
                name: "read_file".into(),
                arguments: r#"{"path":"a.rs"}"#.into(),
            }]
        );

        let stop_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse_body(&[
                content_chunk("The answer is 42."),
                finish_chunk("stop"),
            ])))
            .expect(1)
            .mount(&stop_server)
            .await;
        let client = test_client(&stop_server.uri());
        let (text, calls) = run_agent_turn(&client, vec![ChatMessage::user("the answer?")]).await;
        assert!(calls.is_empty(), "stop drains no calls: {calls:?}");
        assert_eq!(text, "The answer is 42.");
    }

    /// Gateway quirk: single-call responses that omit `index` on every
    /// fragment still reassemble into one call.
    #[tokio::test]
    async fn stream_fragments_without_index_reassemble_into_the_first_call() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse_body(&[
                fragment_chunk(None, Some("call_9"), Some("list_dir"), ""),
                fragment_chunk(None, None, None, "{\"path\""),
                fragment_chunk(None, None, None, ":\".\"}"),
                finish_chunk("tool_calls"),
            ])))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let (_, calls) = run_agent_turn(&client, vec![ChatMessage::user("list")]).await;
        assert_eq!(
            calls,
            vec![ToolCall {
                id: "call_9".into(),
                name: "list_dir".into(),
                arguments: "{\"path\":\".\"}".into(),
            }]
        );
    }

    /// Non-streaming replies carry complete `tool_calls` on the assistant
    /// message (ARCHITECTURE.md §17.6) — including the gateway quirk of
    /// arguments already parsed into a JSON object.
    #[tokio::test]
    async fn complete_chat_parses_tool_calls_from_the_assistant_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(
                serde_json::json!({ "model": "test-model", "stream": false }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [ {
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [
                            {
                                "id": "call_1",
                                "type": "function",
                                "function": {
                                    "name": "read_file",
                                    "arguments": "{\"path\":\"src/main.rs\"}"
                                }
                            },
                            {
                                "id": "call_2",
                                "type": "function",
                                "function": { "name": "list_dir", "arguments": { "path": "." } }
                            }
                        ]
                    }
                } ]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let message = client
            .complete_chat(chat_request())
            .await
            .expect("complete_chat succeeds")
            .expect("a message");
        assert_eq!(message.role, Role::Assistant);
        assert_eq!(message.content, "");
        let calls = message.tool_calls.expect("two tool calls");
        assert_eq!(
            calls[0],
            ToolCall {
                id: "call_1".into(),
                name: "read_file".into(),
                arguments: r#"{"path":"src/main.rs"}"#.into(),
            }
        );
        assert_eq!(calls[1].name, "list_dir");
        assert_eq!(calls[1].arguments, "{\"path\":\".\"}");
    }

    /// Non-streaming plain replies come back as ordinary assistant
    /// messages; no choices is `Ok(None)`, not an error.
    #[tokio::test]
    async fn complete_chat_returns_plain_replies_and_none_without_choices() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [ { "message": { "role": "assistant", "content": "hello there" } } ]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = test_client(&server.uri());
        let message = client
            .complete_chat(chat_request())
            .await
            .expect("complete_chat succeeds")
            .expect("a message");
        assert_eq!(message, ChatMessage::assistant("hello there"));

        let empty = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "choices": [] })),
            )
            .expect(1)
            .mount(&empty)
            .await;
        let client = test_client(&empty.uri());
        assert!(
            matches!(client.complete_chat(chat_request()).await, Ok(None)),
            "no choices is Ok(None)"
        );
    }

    #[tokio::test]
    async fn nvidia_catalog_applies_narrow_evidence_based_capability_policy() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "object": "list",
                "data": [
                    {
                        "id": "mistralai/mistral-large-4-1t-a52b",
                        "object": "model",
                        "created": 1700000000,
                        "owned_by": "mistralai"
                    },
                    {
                        "id": "meta/muse-glimmer-30b",
                        "object": "model",
                        "created": 1700000000,
                        "owned_by": "meta"
                    },
                    {
                        "id": "qwen/qwen3-coder-480b",
                        "object": "model",
                        "created": 1700000000,
                        "owned_by": "qwen"
                    }
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = SeloraClient::new(&format!("{}/v1", server.uri()), "nvapi-secret-key-9999");
        let models = client.list_models().await.expect("list_models succeeds");
        assert_eq!(models.len(), 3);

        let mistral = models
            .iter()
            .find(|m| m.id == "mistralai/mistral-large-4-1t-a52b")
            .unwrap();
        assert_eq!(mistral.tool_capability, ToolCapability::Supported);
        assert!(mistral.is_available());

        let muse = models
            .iter()
            .find(|m| m.id == "meta/muse-glimmer-30b")
            .unwrap();
        assert_eq!(muse.tool_capability, ToolCapability::Unsupported);
        assert!(!muse.is_available());

        // Other NVIDIA models without explicit metadata must stay Unknown, never guessed Supported.
        let unverified = models
            .iter()
            .find(|m| m.id == "qwen/qwen3-coder-480b")
            .unwrap();
        assert_eq!(unverified.tool_capability, ToolCapability::Unknown);
        assert!(!unverified.supports_tools());
        assert!(unverified.is_available());
    }

    #[tokio::test]
    async fn provider_410_and_400_tool_errors_and_404_redact_api_key() {
        let secret = "nvapi-super-secret-do-not-leak-4321";

        let server_410 = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(410).set_body_string("Gone: model retired"))
            .mount(&server_410)
            .await;
        let client_410 = SeloraClient::new(&format!("{}/v1", server_410.uri()), secret);
        let req_410 = ChatRequest {
            model: "meta/muse-glimmer-30b".into(),
            messages: vec![ChatMessage::user("hi")],
            stream: false,
            ..chat_request()
        };
        let err_410 = client_410.complete_chat(req_410).await.unwrap_err().to_string();
        assert!(err_410.contains("410") || err_410.contains("retired"));
        assert!(err_410.contains("meta/muse-glimmer-30b"));
        assert!(!err_410.contains(secret));

        let server_400 = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(400).set_body_string(
                "This model does not support tool calling or function_calling",
            ))
            .mount(&server_400)
            .await;
        let client_400 = SeloraClient::new(&format!("{}/v1", server_400.uri()), secret);
        let outcome_400 = client_400
            .probe_tool_capability("chat/only-model")
            .await
            .expect("probe returns typed outcome");
        assert!(
            matches!(outcome_400, ToolProbeOutcome::Unsupported(ref msg) if !msg.contains(secret)),
            "expected Unsupported without leaking secret, got {outcome_400:?}"
        );

        let server_404 = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(404).set_body_string("Function abc-123: Not found for account"),
            )
            .mount(&server_404)
            .await;
        let client_404 = SeloraClient::new(&format!("{}/v1", server_404.uri()), secret);
        let outcome_404 = client_404
            .probe_tool_capability("unentitled/model")
            .await
            .expect("probe returns typed outcome");
        assert!(
            matches!(outcome_404, ToolProbeOutcome::Inconclusive(ref msg) if !msg.contains(secret) && msg.contains("unentitled/model")),
            "404 must fail probe cleanly instead of proving support: {outcome_404:?}"
        );
    }

    #[tokio::test]
    async fn probe_tool_capability_uses_160_token_budget_and_requires_real_tool_call() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(serde_json::json!({
                "model": "mistralai/mistral-large-4-1t-a52b",
                "stream": false,
                "max_tokens": 160
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "call_probe_1",
                            "type": "function",
                            "function": {
                                "name": "selora_probe_echo",
                                "arguments": "{\"value\":\"ok\"}"
                            }
                        }]
                    }
                }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = SeloraClient::new(&format!("{}/v1", server.uri()), "nvapi-secret");
        let outcome = client
            .probe_tool_capability("mistralai/mistral-large-4-1t-a52b")
            .await
            .expect("probe returns outcome");
        assert_eq!(outcome, ToolProbeOutcome::Supported);
    }

    #[tokio::test]
    async fn stream_chat_normalizes_standard_effort_and_falls_back_when_rejected() {
        let server_norm = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(serde_json::json!({
                "model": "minimaxai/minimax-m3.1-flash",
                "reasoning_effort": "medium",
                "stream": true
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body(&[content_chunk("ok"), finish_chunk("stop")])),
            )
            .expect(1)
            .mount(&server_norm)
            .await;

        let client_norm = SeloraClient::new(&format!("{}/v1", server_norm.uri()), "nvapi-secret");
        let mut req = ChatRequest::new(
            "minimaxai/minimax-m3.1-flash",
            vec![ChatMessage::user("hi")],
        );
        // Simulate raw UI label even if struct field was populated directly.
        req.reasoning_effort = Some("standard".into());
        let (tx, _rx) = mpsc::unbounded_channel();
        client_norm
            .stream_chat(req, tx)
            .await
            .expect("normalized reasoning_effort succeeds");

        // If gateway rejects reasoning_effort via HTTP 400, client retries once without it.
        let server_fallback = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(serde_json::json!({ "reasoning_effort": "medium" })))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_string("Unrecognized request argument: reasoning_effort"),
            )
            .expect(1)
            .mount(&server_fallback)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body(&[content_chunk("recovered"), finish_chunk("stop")])),
            )
            .expect(1)
            .mount(&server_fallback)
            .await;

        let client_fb = SeloraClient::new(&format!("{}/v1", server_fallback.uri()), "nvapi-secret");
        let req_fb = ChatRequest::new("m", vec![ChatMessage::user("hi")])
            .with_reasoning_effort(Some("standard".into()));
        let (tx_fb, _rx_fb) = mpsc::unbounded_channel();
        client_fb
            .stream_chat(req_fb, tx_fb)
            .await
            .expect("fallback retry without reasoning_effort succeeds");
        let recorded = server_fallback.received_requests().await.unwrap();
        let second_body: serde_json::Value = serde_json::from_slice(&recorded[1].body).unwrap();
        assert!(second_body.get("reasoning_effort").is_none());
    }

    #[tokio::test]
    async fn gemini_v1beta_openai_url_and_finish_reason_eof_complete_cleanly() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1beta/openai/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(concat!(
                        "data: {\"choices\":[{\"delta\":{\"thought\":\"plan step\"}}]}\n\n",
                        "data: {\"choices\":[{\"delta\":{\"content\":\"done\"},\"finish_reason\":\"stop\"}]}\n\n"
                    )),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = SeloraClient::new(&format!("{}/v1beta/openai", server.uri()), "goog-key");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let req = ChatRequest::new("gemini-2.5-pro", vec![ChatMessage::user("hi")])
            .with_reasoning_effort(Some("high".into()));
        let receipt = client
            .stream_chat(req, tx)
            .await
            .expect("stream with finish_reason completes without [DONE]");
        assert_eq!(receipt.completion_tokens, 3); // "plan step" (9) + "done" (4) = 13 chars -> 13/4 = 3
        let events = drain(&mut rx);
        assert_eq!(deltas(&events), ["<think>", "plan step", "</think>\n\n", "done"]);
        assert!(matches!(events.last(), Some(ChatEvent::Done(_))));
    }

    #[tokio::test]
    async fn openrouter_uses_reasoning_object_and_extracts_reasoning_details_without_duplication() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openrouter.ai/api/v1/chat/completions"))
            .and(body_partial_json(serde_json::json!({
                "model": "anthropic/claude-sonnet-4",
                "reasoning": { "effort": "high" }
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(concat!(
                        "data: {\"choices\":[{\"delta\":{\"reasoning_details\":[{\"text\":\"trace\"}]}}]}\n\n",
                        "data: {\"choices\":[{\"delta\":{\"content\":\"answer\"},\"finish_reason\":\"stop\"}]}\n\n",
                        "data: [DONE]\n\n"
                    )),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = SeloraClient::new(&format!("{}/openrouter.ai/api/v1", server.uri()), "or-key");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let req = ChatRequest::new("anthropic/claude-sonnet-4", vec![ChatMessage::user("hi")])
            .with_reasoning_effort(Some("max".into()));
        client.stream_chat(req, tx).await.expect("openrouter stream succeeds");

        let recorded = server.received_requests().await.unwrap();
        let wire: serde_json::Value = serde_json::from_slice(&recorded[0].body).unwrap();
        assert!(wire.get("reasoning_effort").is_none());
        let events = drain(&mut rx);
        assert_eq!(deltas(&events), ["<think>", "trace", "</think>\n\n", "answer"]);
    }

    #[tokio::test]
    async fn deepseek_uses_thinking_toggle_and_splits_assistant_tool_reasoning_content() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(serde_json::json!({
                "model": "deepseek-reasoner",
                "thinking": { "type": "enabled" }
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body(&[content_chunk("ok"), finish_chunk("stop")])),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = SeloraClient::new(&format!("{}/v1", server.uri()), "ds-key");
        let mut assistant_with_tools = ChatMessage::assistant("<think>step 1</think>\n\ncalling tool");
        assistant_with_tools.tool_calls = Some(vec![ToolCall {
            id: "call_1".into(),
            name: "read_file".into(),
            arguments: "{}".into(),
        }]);
        let req = ChatRequest::new(
            "deepseek-reasoner",
            vec![
                ChatMessage::user("go"),
                assistant_with_tools,
                ChatMessage::tool_result("call_1", "ok"),
            ],
        )
        .with_reasoning_effort(Some("standard".into()));
        let (tx, _rx) = mpsc::unbounded_channel();
        client.stream_chat(req, tx).await.expect("deepseek stream succeeds");

        let recorded = server.received_requests().await.unwrap();
        let wire: serde_json::Value = serde_json::from_slice(&recorded[0].body).unwrap();
        assert!(wire.get("reasoning_effort").is_none());
        assert_eq!(wire["messages"][1]["reasoning_content"], "step 1");
        assert_eq!(wire["messages"][1]["content"], "calling tool");
    }

    #[tokio::test]
    async fn openai_reasoning_model_and_generic_temperature_fallback_succeed() {
        // Direct OpenAI o3 omits default temperature & maps max_tokens -> max_completion_tokens
        let o3_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({
                        "choices": [{ "message": { "role": "assistant", "content": "hi" } }]
                    })),
            )
            .expect(1)
            .mount(&o3_server)
            .await;
        let o3_client = SeloraClient::new(&format!("{}/v1", o3_server.uri()), "sk-test");
        let mut o3_req = ChatRequest::new("o3", vec![ChatMessage::user("hi")]);
        o3_req.max_tokens = Some(512);
        o3_client.complete_chat(o3_req).await.expect("o3 succeeds");
        let rec = o3_server.received_requests().await.unwrap();
        let wire: serde_json::Value = serde_json::from_slice(&rec[0].body).unwrap();
        assert!(wire.get("temperature").is_none());
        assert!(wire.get("max_tokens").is_none());
        assert_eq!(wire["max_completion_tokens"], 512);

        // Generic gateway rejecting temperature + max_tokens recovers cleanly on retry
        let fb_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(serde_json::json!({ "max_tokens": 160 })))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_string("Unsupported parameter: 'max_tokens' is not supported with this model. Use 'max_completion_tokens' instead."),
            )
            .expect(1)
            .mount(&fb_server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(serde_json::json!({ "max_completion_tokens": 160 })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "call_1",
                            "type": "function",
                            "function": { "name": "selora_probe_echo", "arguments": "{\"text\":\"ok\"}" }
                        }]
                    }
                }]
            })))
            .expect(1)
            .mount(&fb_server)
            .await;

        let fb_client = SeloraClient::new(&format!("{}/v1", fb_server.uri()), "sk-test");
        let outcome = fb_client.probe_tool_capability("custom-reasoning-model").await.unwrap();
        assert_eq!(outcome, ToolProbeOutcome::Supported);
    }

    #[tokio::test]
    async fn probe_retries_with_1024_budget_when_reasoning_exhausts_160_tokens() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(serde_json::json!({ "max_tokens": 160 })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{ "message": { "role": "assistant", "content": "<think>still thinking" } }]
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(serde_json::json!({ "max_tokens": 1024 })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "",
                        "tool_calls": [{
                            "id": "call_2",
                            "type": "function",
                            "function": { "name": "selora_probe_echo", "arguments": "{\"text\":\"ok\"}" }
                        }]
                    }
                }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = SeloraClient::new(&format!("{}/v1", server.uri()), "sk-test");
        let outcome = client.probe_tool_capability("thinking-model").await.unwrap();
        assert_eq!(outcome, ToolProbeOutcome::Supported);
    }

    #[tokio::test]
    async fn anthropic_sends_headers_and_omits_temp_when_thinking_or_reasoning_when_none() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api.anthropic.com/v1/chat/completions"))
            .and(header("x-api-key", "ant-secret"))
            .and(header("anthropic-version", "2023-06-01"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body(&[content_chunk("ok"), finish_chunk("stop")])),
            )
            .expect(2)
            .mount(&server)
            .await;

        let client = SeloraClient::new(&format!("{}/api.anthropic.com/v1", server.uri()), "ant-secret");
        let (tx1, _rx1) = mpsc::unbounded_channel();
        let req_on = ChatRequest::new("claude-sonnet-4", vec![ChatMessage::user("hi")])
            .with_reasoning_effort(Some("max".into()));
        client.stream_chat(req_on, tx1).await.expect("anthropic thinking succeeds");

        let (tx2, _rx2) = mpsc::unbounded_channel();
        let req_off = ChatRequest::new("claude-sonnet-4", vec![ChatMessage::user("hi")])
            .with_reasoning_effort(Some("none".into()));
        client.stream_chat(req_off, tx2).await.expect("anthropic off succeeds");

        let recs = server.received_requests().await.unwrap();
        let on_wire: serde_json::Value = serde_json::from_slice(&recs[0].body).unwrap();
        assert_eq!(on_wire["reasoning_effort"], "high");
        assert!(on_wire.get("temperature").is_none());

        let off_wire: serde_json::Value = serde_json::from_slice(&recs[1].body).unwrap();
        assert!(off_wire.get("reasoning_effort").is_none());
        assert_eq!(off_wire["temperature"], 0.7);
    }
}
