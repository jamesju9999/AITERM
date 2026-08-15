//! Anthropic Claude provider — `https://api.anthropic.com/v1/messages` (SSE).
//!
//! API reference: https://docs.anthropic.com/en/api/messages
//!
//! Key differences from OpenAI format:
//! - `system` is a top-level field, NOT inside the messages array.
//! - Auth uses `x-api-key` header + `anthropic-version`.
//! - SSE events: `content_block_delta` carries text; `message_stop` signals done.
//! - Status 529 means "overloaded" (treat as Network error).

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::ai::{
    sse::{find_line_end, map_http_error, separator_len, truncate},
    AiError, AiProvider, AiToolCall, ChatMessage, GenerateChunk, GenerateRequest,
    GenerateWithToolsResult, McpToolDefinition, TokenUsage,
};

pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// OAuth 請求必要的 beta 旗標，逗號分隔的 header 形態。
///
/// **這是全專案唯一的定義**。上游靠這些旗標判斷這是 Claude Code 的 OAuth
/// 請求，缺了就不會把 token 當 OAuth token 看待。先前這個字串散在四個檔案
/// 共八處，改動時漏掉其中幾處會讓「某些功能壞掉、某些正常」——最難查的
/// 那種症狀。
pub const OAUTH_BETA_HEADER: &str = "claude-code-20250219,oauth-2025-04-20";

/// 同一組旗標的陣列形態，給需要與客戶端旗標合併去重的 bridge 用。
/// 與 [`OAUTH_BETA_HEADER`] 的一致性由 `oauth_beta_two_forms_agree` 測試保證。
pub const OAUTH_BETA_PARTS: &[&str] = &["claude-code-20250219", "oauth-2025-04-20"];

pub struct AnthropicClient {
    token: String,
    model: String,
    base_url: String,
    client: reqwest::Client,
    is_oauth: bool,
}

impl AnthropicClient {
    pub fn new(api_key: String, model: String) -> Self {
        Self::with_base_url(api_key, model, "https://api.anthropic.com".into())
    }

    pub fn with_base_url(api_key: String, model: String, base_url: String) -> Self {
        Self { token: api_key, model, base_url, client: reqwest::Client::new(), is_oauth: false }
    }

    pub fn with_oauth(access_token: String, model: String, base_url: String) -> Self {
        Self { token: access_token, model, base_url, client: reqwest::Client::new(), is_oauth: true }
    }

    fn messages_url(&self) -> String {
        format!("{}/v1/messages", self.base_url.trim_end_matches('/'))
    }

    fn auth_request(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.is_oauth {
            builder
                .header("Authorization", format!("Bearer {}", self.token))
                .header("anthropic-beta", OAUTH_BETA_HEADER)
                .header("x-app", "cli")
        } else {
            builder.header("x-api-key", &self.token)
        }
    }
}

#[async_trait]
impl AiProvider for AnthropicClient {
    fn id(&self) -> &str { "anthropic" }
    fn display_name(&self) -> &str { "Anthropic" }

    async fn generate(
        &self,
        req: GenerateRequest,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<(), AiError> {
        let body = build_request_body(&self.model, &req, true, self.is_oauth);
        let resp = self
            .auth_request(self.client.post(self.messages_url()))
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        let status = resp.status();
        if !status.is_success() {
            // 529 = Anthropic overloaded
            if status.as_u16() == 529 {
                return Err(AiError::Network { message: "Anthropic API is overloaded".into() });
            }
            return Err(map_http_error(status, resp).await);
        }
        consume_anthropic_sse(resp, tx).await
    }

    async fn generate_with_tools(
        &self,
        req: GenerateRequest,
        tools: Vec<McpToolDefinition>,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<GenerateWithToolsResult, AiError> {
        let tool_defs: serde_json::Value = serde_json::Value::Array(
            tools.iter().map(|t| serde_json::json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema
            })).collect()
        );

        let system_text = extract_system_text(&req.messages, &req.system_prompt);
        let messages: Vec<serde_json::Value> = build_anthropic_messages(&req.messages);

        // 串流。原本沒帶 stream，帶工具的對話因此整段一次跳出來。
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 4096,
            "system": build_system_field(&system_text, self.is_oauth),
            "messages": messages,
            "tools": tool_defs,
            "stream": true
        });

        let resp = self
            .auth_request(self.client.post(self.messages_url()))
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        let status = resp.status();
        if status == 401 { return Err(AiError::AuthFailed); }
        if status.as_u16() == 529 {
            return Err(AiError::Network { message: "Anthropic API is overloaded".into() });
        }
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            // 訂閱（OAuth）憑證帶 `tools` 時，Anthropic 會把請求算到 API credits
            // 而不是訂閱額度；credits 是 0 就回這個 400。實測用三次 A/B 確認過
            // 唯一的變數是 tools 的有無（串流與 max_tokens 都無關）。
            //
            // 對這個憑證而言，這等同「無法使用原生工具呼叫」——回
            // ToolCallingUnsupported 讓上層改用「工具描述注入系統提示」的
            // fallback，那條不帶 tools，因此回到訂閱計費。
            //
            // 只限 OAuth：API key 用戶付的就是 API 額度，同一句話對他們是真的
            // 餘額不足，偽裝成「不支援工具」會蓋掉真正的問題。
            if self.is_oauth && status.as_u16() == 400 && body_text.contains("out of extra usage") {
                log::warn!(
                    "Anthropic 訂閱憑證帶工具被計入 API credits（餘額不足），改用系統提示注入的相容模式"
                );
                return Err(AiError::ToolCallingUnsupported {
                    reason: crate::ai::ToolFallbackReason::SubscriptionBilling,
                });
            }
            return Err(AiError::Network {
                message: format!("http {}: {}", status.as_u16(), truncate(&body_text, 300)),
            });
        }

        let streamed = consume_anthropic_sse_with_tools(resp, tx).await?;

        if !streamed.calls.is_empty() {
            return Ok(GenerateWithToolsResult::ToolCalls {
                calls: streamed.calls,
                raw: streamed.raw,
            });
        }
        Ok(GenerateWithToolsResult::Text(streamed.text))
    }

    async fn health_check(&self) -> Result<(), AiError> {
        // Minimal 1-token non-streaming request.
        let hc_req = health_check_request();
        let body = build_request_body(&self.model, &hc_req, false, self.is_oauth);
        let resp = self
            .auth_request(self.client.post(self.messages_url()))
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else if status.as_u16() == 529 {
            Err(AiError::Network { message: "Anthropic API is overloaded".into() })
        } else {
            Err(map_http_error(status, resp).await)
        }
    }
}

// ── Request types ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    system: serde_json::Value,
    messages: Vec<AnthropicMessage>,
    max_tokens: u32,
    stream: bool,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: serde_json::Value,
}

/// True if `m` is a `role:"system"` ChatMessage. Anthropic has no place for the
/// system role inside `messages` — it must be excluded from every message list
/// built for Anthropic and folded into the top-level `system` field instead
/// (see `extract_system_text`). Centralized here so `build_anthropic_messages`
/// and `build_request_body` can't drift on what counts as a system message.
fn is_system_message(m: &ChatMessage) -> bool {
    m.role == "system"
}

/// Extract and concatenate any `role:"system"` ChatMessage content onto `base_system_prompt`.
/// Anthropic requires the system prompt as a separate top-level field, not a message —
/// used by both the plain streaming path (`build_request_body`) and tool-calling path
/// (`generate_with_tools`).
fn extract_system_text(messages: &[ChatMessage], base_system_prompt: &str) -> String {
    let mut system_text = base_system_prompt.to_string();
    for m in messages {
        if !is_system_message(m) {
            continue;
        }
        match m.content.as_str() {
            Some(s) if !s.is_empty() => {
                system_text = if system_text.is_empty() {
                    s.to_string()
                } else {
                    format!("{}\n\n{}", system_text, s)
                };
            }
            Some(_) => {}
            None => {
                if !m.content.is_null() {
                    log::warn!("system-role ChatMessage has non-string content; content dropped when building Anthropic system prompt");
                }
            }
        }
    }
    system_text
}

/// Convert internal ChatMessage history into Anthropic's Messages API format.
/// Anthropic has no "tool" role: tool calls live inside an assistant message's
/// `content` array as `tool_use` blocks, and tool results are wrapped in a
/// `user` message's `content` array as `tool_result` blocks. Consecutive
/// `role: "tool"` ChatMessages (parallel tool calls) are coalesced into one
/// user turn, since Anthropic requires strictly alternating roles.
fn build_anthropic_messages(messages: &[ChatMessage]) -> Vec<serde_json::Value> {
    let mut result: Vec<serde_json::Value> = Vec::with_capacity(messages.len());
    let mut pending_tool_results: Vec<serde_json::Value> = Vec::new();

    for m in messages {
        if is_system_message(m) {
            continue;
        }
        if m.role == "tool" {
            let tool_use_id = m.tool_call_id.clone().unwrap_or_else(|| {
                log::warn!("tool-role ChatMessage missing tool_call_id; sending empty tool_use_id to Anthropic");
                String::new()
            });
            pending_tool_results.push(serde_json::json!({
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": m.content.clone(),
            }));
            continue;
        }

        flush_tool_results(&mut result, &mut pending_tool_results);

        if m.role == "assistant" {
            if let Some(tool_calls) = &m.tool_calls {
                result.push(serde_json::json!({
                    "role": "assistant",
                    "content": to_anthropic_content_blocks(tool_calls),
                }));
                continue;
            }
        }

        result.push(serde_json::json!({ "role": m.role, "content": m.content }));
    }

    flush_tool_results(&mut result, &mut pending_tool_results);
    result
}

fn flush_tool_results(result: &mut Vec<serde_json::Value>, pending: &mut Vec<serde_json::Value>) {
    if !pending.is_empty() {
        result.push(serde_json::json!({
            "role": "user",
            "content": std::mem::take(pending),
        }));
    }
}

/// Convert a ChatMessage's `tool_calls` value into Anthropic content blocks.
/// Handles two possible shapes: Anthropic-native (already `tool_use` blocks,
/// e.g. echoed back verbatim from a prior `raw`) and OpenAI-shaped (the
/// frontend's fallback reconstruction, `function.arguments` as a JSON string).
/// Detection: OpenAI-shaped elements always have a `"function"` key; Anthropic
/// content blocks (whether `text` or `tool_use`) never do — so checking only
/// the first element would misdetect a `[text, tool_use]` raw echo.
fn to_anthropic_content_blocks(tool_calls: &serde_json::Value) -> Vec<serde_json::Value> {
    let Some(arr) = tool_calls.as_array() else {
        return vec![];
    };

    let is_openai_shaped = arr.iter().any(|el| el.get("function").is_some());
    if !is_openai_shaped {
        return arr.clone();
    }

    arr.iter()
        // Assumes a homogeneous array (all-native or all-OpenAI-shaped, never mixed) —
        // a genuinely mixed array would silently drop the native blocks here.
        .filter(|el| el.get("function").is_some())
        .map(|el| {
            let arguments = el["function"]["arguments"]
                .as_str()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(serde_json::json!({}));
            serde_json::json!({
                "type": "tool_use",
                "id": el["id"],
                "name": el["function"]["name"],
                "input": arguments,
            })
        })
        .collect()
}

/// Anthropic gates subscription OAuth tokens (`sk-ant-oat*`) so they only serve
/// genuine Claude Code traffic. A request whose first system block is not this
/// exact sentinel is rejected with an opaque
/// `{"type":"rate_limit_error","message":"Error"}` — note that this masquerades
/// as a rate limit even on the very first request, so do NOT read that error as
/// a quota problem. API-key auth has no such requirement.
pub const CLAUDE_CODE_SENTINEL: &str = "You are Claude Code, Anthropic's official CLI for Claude.";

/// Build the top-level `system` field.
///
/// API-key auth keeps the plain-string form. OAuth auth must use the block-array
/// form with `CLAUDE_CODE_SENTINEL` first (see the const's docs). The caller's own
/// prompt follows as a second block, and is omitted entirely when empty — Anthropic
/// rejects empty text blocks with a 400.
fn build_system_field(system_text: &str, is_oauth: bool) -> serde_json::Value {
    if !is_oauth {
        return serde_json::Value::String(system_text.to_owned());
    }
    let mut blocks = vec![serde_json::json!({ "type": "text", "text": CLAUDE_CODE_SENTINEL })];
    if !system_text.is_empty() {
        blocks.push(serde_json::json!({ "type": "text", "text": system_text }));
    }
    serde_json::Value::Array(blocks)
}

fn build_request_body(
    model: &str,
    req: &GenerateRequest,
    stream: bool,
    is_oauth: bool,
) -> AnthropicRequest {
    let messages = req
        .messages
        .iter()
        .filter(|m| !is_system_message(m))
        .map(|m| AnthropicMessage { role: m.role.clone(), content: m.content.clone() })
        .collect();
    AnthropicRequest {
        model: model.to_owned(),
        system: build_system_field(
            &extract_system_text(&req.messages, &req.system_prompt),
            is_oauth,
        ),
        messages,
        max_tokens: req.max_tokens.unwrap_or(1024),
        stream,
    }
}

fn health_check_request() -> GenerateRequest {
    use crate::ai::{EnvSnapshot, QueryMode};
    use std::path::PathBuf;
    GenerateRequest {
        system_prompt: "ping".into(),
        messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!("hi"), tool_call_id: None, tool_calls: None }],
        context: EnvSnapshot {
            os: std::env::consts::OS.into(),
            shell: "sh".into(),
            cwd: PathBuf::from("."),
            ..Default::default()
        },
        mode: QueryMode::SingleCommand,
        max_tokens: Some(1),
    }
}

// ── SSE consumer（帶工具）─────────────────────────────────────────────────────

/// 串流中累積起來的 content blocks。
pub(crate) struct AnthropicStreamed {
    pub text: String,
    pub calls: Vec<AiToolCall>,
    /// 完整的 content blocks 陣列（含文字區塊），供需要回填的模型 echo 回去。
    pub raw: Option<serde_json::Value>,
}

/// 像 `consume_anthropic_sse`，但同時累積 `tool_use` 區塊。
///
/// 文字（`text_delta`）邊收邊送；工具參數（`input_json_delta`）是分片的 JSON
/// 字串，要按 content block 的 index 對位累積，收齊才 parse。
async fn consume_anthropic_sse_with_tools(
    resp: reqwest::Response,
    tx: mpsc::Sender<GenerateChunk>,
) -> Result<AnthropicStreamed, AiError> {
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Block {
        ty: String,
        id: Option<String>,
        name: Option<String>,
        text: String,
        json: String,
    }

    let mut stream = resp.bytes_stream();
    let mut buf = Vec::<u8>::new();
    // BTreeMap：輸出順序照 content block 的 index，而不是事件抵達的順序。
    let mut blocks: BTreeMap<usize, Block> = BTreeMap::new();
    let mut text = String::new();
    let mut stream_ended = false;
    let mut saw_stop = false;
    let mut saw_data_prefix = false;
    let mut raw_body = String::new();

    while !stream_ended && !saw_stop {
        match stream.next().await {
            Some(item) => {
                let bytes = item.map_err(|e| AiError::Network { message: e.to_string() })?;
                buf.extend_from_slice(&bytes);
            }
            None => {
                stream_ended = true;
                if !buf.is_empty() && buf.last() != Some(&b'\n') {
                    buf.push(b'\n');
                }
            }
        }

        loop {
            let Some(pos) = find_line_end(&buf) else { break };
            let line_bytes: Vec<u8> = buf.drain(..pos).collect();
            let sep = separator_len(&buf);
            buf.drain(..sep);
            let line = match std::str::from_utf8(&line_bytes) {
                Ok(s) => s.trim(),
                Err(_) => continue,
            };
            if !saw_data_prefix {
                raw_body.push_str(line);
            }
            if line.is_empty() { continue; }
            let Some(data) = line.strip_prefix("data:") else { continue };
            saw_data_prefix = true;
            let Ok(event) = serde_json::from_str::<AnthropicSseEvent>(data.trim()) else { continue };

            match event {
                AnthropicSseEvent::ContentBlockStart { index, content_block } => {
                    let b = blocks.entry(index).or_default();
                    b.ty = content_block.ty;
                    b.id = content_block.id;
                    b.name = content_block.name;
                }
                AnthropicSseEvent::ContentBlockDelta { index, delta } => {
                    let b = blocks.entry(index).or_default();
                    if let Some(t) = delta.text {
                        b.text.push_str(&t);
                        text.push_str(&t);
                        let _ = tx.send(GenerateChunk { delta: t, done: false, usage: None }).await;
                    }
                    if let Some(j) = delta.partial_json {
                        b.json.push_str(&j);
                    }
                }
                AnthropicSseEvent::MessageDelta { usage } => {
                    let token_usage = usage.map(|u| TokenUsage {
                        prompt: u.input_tokens,
                        completion: u.output_tokens,
                    });
                    let _ = tx.send(GenerateChunk { delta: String::new(), done: false, usage: token_usage }).await;
                }
                AnthropicSseEvent::MessageStop => {
                    let _ = tx.send(GenerateChunk { delta: String::new(), done: true, usage: None }).await;
                    saw_stop = true;
                    break;
                }
                AnthropicSseEvent::Other => {}
            }
        }
    }

    if !saw_stop {
        let _ = tx.send(GenerateChunk { delta: String::new(), done: true, usage: None }).await;
    }

    // 沒有半個 `data:` 行 = 對方把串流收合成了單一 JSON 回應（企業 LLM gateway
    // 會這樣做）。照非串流的形狀解一次，否則使用者會拿到一則空白回覆。
    if !saw_data_prefix {
        let raw = raw_body.trim();
        if raw.starts_with('{') {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) {
                if let Some(content_blocks) = v["content"].as_array() {
                    let calls: Vec<AiToolCall> = content_blocks.iter()
                        .filter(|b| b["type"].as_str() == Some("tool_use"))
                        .map(|b| AiToolCall {
                            id: b["id"].as_str().unwrap_or("").to_string(),
                            tool_name: b["name"].as_str().unwrap_or("").to_string(),
                            args: b["input"].clone(),
                            thought_signature: None,
                        })
                        .collect();
                    let joined: String = content_blocks.iter()
                        .filter_map(|b| b["text"].as_str())
                        .collect();
                    let raw_value = if calls.is_empty() {
                        None
                    } else {
                        Some(serde_json::Value::Array(content_blocks.clone()))
                    };
                    return Ok(AnthropicStreamed { text: joined, calls, raw: raw_value });
                }
            }
        }
    }

    let mut calls = Vec::new();
    let mut raw_blocks = Vec::new();
    for (_, b) in blocks {
        if b.ty == "tool_use" {
            let input: serde_json::Value = if b.json.trim().is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(&b.json).unwrap_or(serde_json::json!({}))
            };
            let id = b.id.unwrap_or_default();
            let name = b.name.unwrap_or_default();
            raw_blocks.push(serde_json::json!({
                "type": "tool_use", "id": id, "name": name, "input": input
            }));
            calls.push(AiToolCall {
                id,
                tool_name: name,
                args: input,
                thought_signature: None,
            });
        } else if !b.text.is_empty() {
            raw_blocks.push(serde_json::json!({ "type": "text", "text": b.text }));
        }
    }

    let raw = if calls.is_empty() { None } else { Some(serde_json::Value::Array(raw_blocks)) };
    Ok(AnthropicStreamed { text, calls, raw })
}

// ── SSE consumer ─────────────────────────────────────────────────────────────

async fn consume_anthropic_sse(
    resp: reqwest::Response,
    tx: mpsc::Sender<GenerateChunk>,
) -> Result<(), AiError> {
    let mut stream = resp.bytes_stream();
    let mut buf = Vec::<u8>::new();
    let mut saw_done = false;

    'outer: while let Some(item) = stream.next().await {
        let bytes = item.map_err(|e| AiError::Network { message: e.to_string() })?;
        buf.extend_from_slice(&bytes);

        loop {
            let Some(pos) = find_line_end(&buf) else { break };
            let line_bytes: Vec<u8> = buf.drain(..pos).collect();
            let sep = separator_len(&buf);
            buf.drain(..sep);
            let line = match std::str::from_utf8(&line_bytes) {
                Ok(s) => s.trim(),
                Err(_) => continue,
            };
            if line.is_empty() { continue; }

            // SSE lines: "event: ..." or "data: ..."
            if let Some(data) = line.strip_prefix("data:") {
                let data = data.trim();
                match serde_json::from_str::<AnthropicSseEvent>(data) {
                    Ok(event) => match event {
                        AnthropicSseEvent::ContentBlockStart { .. } => {}
                        AnthropicSseEvent::ContentBlockDelta { delta, .. } => {
                            if let Some(text) = delta.text {
                                let _ = tx
                                    .send(GenerateChunk { delta: text, done: false, usage: None })
                                    .await;
                            }
                        }
                        AnthropicSseEvent::MessageDelta { usage } => {
                            let token_usage = usage.map(|u| TokenUsage {
                                prompt: u.input_tokens,
                                completion: u.output_tokens,
                            });
                            let _ = tx
                                .send(GenerateChunk { delta: String::new(), done: false, usage: token_usage })
                                .await;
                        }
                        AnthropicSseEvent::MessageStop => {
                            let _ = tx
                                .send(GenerateChunk { delta: String::new(), done: true, usage: None })
                                .await;
                            saw_done = true;
                            break 'outer;
                        }
                        AnthropicSseEvent::Other => {}
                    },
                    Err(_) => continue,
                }
            }
        }
    }

    if !saw_done {
        let _ = tx.send(GenerateChunk { delta: String::new(), done: true, usage: None }).await;
    }
    Ok(())
}

// ── SSE event types ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicSseEvent {
    ContentBlockStart {
        #[serde(default)]
        index: usize,
        content_block: ContentBlockStart,
    },
    ContentBlockDelta {
        #[serde(default)]
        index: usize,
        delta: ContentDelta,
    },
    MessageDelta {
        #[serde(default)]
        usage: Option<MessageDeltaUsage>,
    },
    #[serde(rename = "message_stop")]
    MessageStop,
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct ContentDelta {
    #[serde(rename = "type")]
    _ty: Option<String>,
    #[serde(default)]
    text: Option<String>,
    /// 工具參數的片段。整個 `input` 物件是被切成好幾段 JSON 字串送來的。
    #[serde(default)]
    partial_json: Option<String>,
}

#[derive(Deserialize)]
struct ContentBlockStart {
    #[serde(rename = "type")]
    ty: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct MessageDeltaUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

#[cfg(test)]
mod tests {

    /// 兩種形態必須永遠等價 —— 收斂之後，這是唯一還可能漂移的地方。
    #[test]
    fn oauth_beta_two_forms_agree() {
        assert_eq!(OAUTH_BETA_PARTS.join(","), OAUTH_BETA_HEADER);
    }

    use super::*;
    use crate::ai::{EnvSnapshot, QueryMode};
    use std::path::PathBuf;

    fn sample_req() -> GenerateRequest {
        GenerateRequest {
            system_prompt: "You are a terminal assistant.".into(),
            messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!("list files"), tool_call_id: None, tool_calls: None }],
            context: EnvSnapshot { os: "windows".into(), shell: "pwsh".into(), cwd: PathBuf::from("C:\\"), ..Default::default() },
            mode: QueryMode::SingleCommand,
            max_tokens: Some(256),
        }
    }

    /// OAuth auth must send `system` as a block array whose FIRST block is the
    /// Claude Code sentinel, otherwise Anthropic rejects the request with an
    /// opaque `rate_limit_error` even on the first call.
    #[test]
    fn oauth_request_body_puts_claude_code_sentinel_first() {
        let req = sample_req();
        let body = build_request_body("claude-sonnet-4-5", &req, true, true);
        let json = serde_json::to_value(&body).unwrap();
        let blocks = json["system"].as_array().expect("oauth system must be a block array");
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[0]["text"], CLAUDE_CODE_SENTINEL);
        // The caller's own prompt must survive, as a later block.
        assert_eq!(blocks[1]["text"], "You are a terminal assistant.");
    }

    /// An empty caller prompt must not produce an empty text block — Anthropic
    /// 400s on those. The sentinel alone is the whole array.
    #[test]
    fn oauth_request_body_omits_empty_prompt_block() {
        let mut req = sample_req();
        req.system_prompt = String::new();
        let body = build_request_body("claude-sonnet-4-5", &req, true, true);
        let json = serde_json::to_value(&body).unwrap();
        let blocks = json["system"].as_array().unwrap();
        assert_eq!(blocks.len(), 1, "expected only the sentinel block");
        assert_eq!(blocks[0]["text"], CLAUDE_CODE_SENTINEL);
    }

    /// API-key auth has no sentinel requirement — keep the plain-string form so
    /// this path stays byte-identical to before the OAuth fix.
    #[test]
    fn api_key_request_body_keeps_plain_string_system() {
        let req = sample_req();
        let body = build_request_body("claude-sonnet-4-5", &req, true, false);
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["system"], "You are a terminal assistant.");
    }

    #[test]
    fn request_body_puts_system_at_top_level() {
        let req = sample_req();
        let body = build_request_body("claude-sonnet-4-5", &req, true, false);
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["system"], "You are a terminal assistant.");
        assert!(json["messages"][0]["role"] == "user");
        // system must NOT appear inside messages array
        for msg in json["messages"].as_array().unwrap() {
            assert_ne!(msg["role"], "system", "system must not be in messages array");
        }
        assert_eq!(json["stream"], true);
        assert_eq!(json["model"], "claude-sonnet-4-5");
    }

    #[test]
    fn request_body_extracts_system_role_message_and_excludes_it_from_messages() {
        let mut req = sample_req();
        req.messages.insert(0, ChatMessage {
            role: "system".into(),
            content: serde_json::json!("You are the orchestrator."),
            tool_call_id: None,
            tool_calls: None,
        });
        let body = build_request_body("claude-sonnet-4-5", &req, true, false);
        let json = serde_json::to_value(&body).unwrap();
        // sample_req() carries a non-empty system_prompt ("You are a terminal assistant.");
        // extract_system_text concatenates it with the system-role message rather than
        // overwriting it, matching the already-approved behavior in generate_with_tools.
        assert_eq!(json["system"], "You are a terminal assistant.\n\nYou are the orchestrator.");
        for msg in json["messages"].as_array().unwrap() {
            assert_ne!(msg["role"], "system", "system message must not appear in the messages array");
        }
    }

    #[test]
    fn sse_event_content_block_delta_parses() {
        let raw = r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}"#;
        let event: AnthropicSseEvent = serde_json::from_str(raw).unwrap();
        match event {
            AnthropicSseEvent::ContentBlockDelta { delta, index } => {
                assert_eq!(index, 0);
                assert_eq!(delta.text.unwrap(), "hello");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn sse_event_message_stop_parses() {
        let raw = r#"{"type":"message_stop"}"#;
        let event: AnthropicSseEvent = serde_json::from_str(raw).unwrap();
        assert!(matches!(event, AnthropicSseEvent::MessageStop));
    }

    #[test]
    fn sse_event_unknown_type_parses_as_other() {
        let raw = r#"{"type":"ping"}"#;
        let event: AnthropicSseEvent = serde_json::from_str(raw).unwrap();
        assert!(matches!(event, AnthropicSseEvent::Other));
    }
}
