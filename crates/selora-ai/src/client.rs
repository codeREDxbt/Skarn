//! `SeloraClient` — the HTTP client for the Selora gateway (ARCHITECTURE.md §6).

use std::fmt;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use selora_core::SeloraError;
use serde::Deserialize;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::chat_types::ToolCall;
use crate::types::{ChatEvent, ChatMessage, ChatReceipt, ChatRequest, Model, SeloraStatus};

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
    /// The gateway refused a non-transient request (400/404/422/...).
    Rejected,
    /// The gateway answered with something this client cannot parse.
    InvalidResponse,
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
        GatewayFailure::Rejected | GatewayFailure::InvalidResponse => error.to_string(),
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
    /// A client for `base_url` (e.g. `https://api.selora.lol`) authenticated
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

    fn models_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/v1") || base.ends_with("/v4") {
            format!("{base}/models")
        } else {
            format!("{base}/v1/models")
        }
    }

    fn chat_completions_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/v1") || base.ends_with("/v4") {
            format!("{base}/chat/completions")
        } else {
            format!("{base}/v1/chat/completions")
        }
    }

    /// `GET {base_url}/v1/models` — the model catalog, bearer-authenticated.
    pub async fn list_models(&self) -> Result<Vec<Model>, SeloraError> {
        let url = self.models_url();
        let api_key = self.api_key.clone();
        let http = self.http.clone();

        let response_res = self
            .send_with_retry("model catalog", self.request_timeout, || {
                let mut req = http.get(&url);
                if !api_key.trim().is_empty() {
                    req = req.bearer_auth(&api_key);
                }
                req.header("HTTP-Referer", "https://selora.ide")
                    .header("X-Title", "Selora IDE")
                    .header("User-Agent", "Selora-IDE/0.1.0")
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
                                let models = parse_models_from_json(&tags_text);
                                if !models.is_empty() {
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
        let body = req.clone().with_stream(true);

        let mut stream_retries = 0;
        'attempt: loop {
            let response = match self
                .send_with_retry("chat stream", self.stream_response_timeout, || {
                    let mut http_req = self.http.post(&url);
                    if !self.api_key.trim().is_empty() {
                        http_req = http_req.bearer_auth(&self.api_key);
                    }
                    http_req
                        .header("HTTP-Referer", "https://selora.ide")
                        .header("X-Title", "Selora IDE")
                        .header("User-Agent", "Selora-IDE/0.1.0")
                        .json(&body)
                })
                .await
            {
                Ok(response) => response,
                Err(err) => {
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
                                        if let Some(msg) = val
                                            .get("error")
                                            .and_then(|e| e.get("message"))
                                            .and_then(|m| m.as_str())
                                        {
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
                            let delta = first_choice.and_then(|choice| choice.delta.as_ref());

                            let reasoning_opt = delta
                                .and_then(|d| d.reasoning_content.as_deref())
                                .or_else(|| delta.and_then(|d| d.reasoning.as_deref()))
                                .or_else(|| delta.and_then(|d| d.thought.as_deref()))
                                .filter(|s| !s.is_empty());

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

            if !emitted && !tx.is_closed() && stream_retries < self.max_net_retries {
                let delay = with_jitter(retry_delay(self.net_retry_delay, stream_retries));
                stream_retries += 1;
                tokio::time::sleep(delay).await;
                continue 'attempt;
            }
            // The body ended without the `[DONE]` sentinel: the reply was
            // truncated (RISKS.md §3). Never report a receipt we cannot trust.
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
            .send_with_retry("ghost completion", self.request_timeout, || {
                self.http.post(&url).bearer_auth(&self.api_key).json(&body)
            })
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
        let body = req.with_stream(false);

        let response = self
            .send_with_retry("chat completion", self.request_timeout, || {
                self.http.post(&url).bearer_auth(&self.api_key).json(&body)
            })
            .await?;
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
                        return Err(SeloraError::Auth(format!(
                            "key invalid or not authorized (HTTP {status}{body})"
                        )));
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
                            "gateway unavailable (HTTP {status}){body}"
                        )));
                    }
                    let body = self.error_body(response).await;
                    return Err(SeloraError::Ai(format!(
                        "gateway rejected request (HTTP {status}){body}"
                    )));
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
                        self.base_url
                    )));
                }
            }
        }
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
                        supports_tools: true,
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
    text: Option<String>,
    /// Streaming tool-call fragments (the OpenAI `delta.tool_calls[]`
    /// array) — emitted as [`ChatEvent::ToolCallDelta`] for the caller's
    /// [`crate::ToolCallAccumulator`] to reassemble.
    #[serde(default)]
    tool_calls: Option<Vec<ToolCallFragment>>,
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
}
