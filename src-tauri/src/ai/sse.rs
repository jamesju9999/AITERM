//! Shared SSE (Server-Sent Events) streaming utilities.
//!
//! Used by `OpenAiClient` and `OpenAiCompatibleClient`.

use futures_util::StreamExt;
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::ai::{AiError, GenerateChunk, TokenUsage};

// ── Public entry point ───────────────────────────────────────────────────────

/// Consume an OpenAI-format SSE response stream and forward `GenerateChunk`s.
pub async fn consume_openai_sse(
    resp: reqwest::Response,
    tx: mpsc::Sender<GenerateChunk>,
) -> Result<(), AiError> {
    let mut stream = resp.bytes_stream();
    let mut buf = Vec::<u8>::new();
    let mut saw_done = false;
    let mut saw_data_prefix = false;
    let mut raw_response = String::new(); // Accumulate raw response for fallback

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
            
            if !saw_data_prefix {
                raw_response.push_str(line);
                raw_response.push('\n');
            }

            if line.is_empty() { continue; }

            let payload = match line.strip_prefix("data:") {
                Some(p) => {
                    saw_data_prefix = true;
                    p.trim()
                },
                None => continue,
            };
            if payload == "[DONE]" {
                saw_done = true;
                break 'outer;
            }
            match serde_json::from_str::<OpenAiSsePayload>(payload) {
                Ok(p) => {
                    let delta = p.delta_text();
                    let usage = p.usage_into();
                    let finish = p.finish_reason_present();
                    let _ = tx.send(GenerateChunk { delta, done: false, usage: usage.clone() }).await;
                    if finish {
                        let _ = tx.send(GenerateChunk { delta: String::new(), done: true, usage }).await;
                    }
                }
                Err(_) => continue,
            }
        }
    }

    if !saw_data_prefix {
        let raw = raw_response.trim();
        if !raw.is_empty() {
            if raw.starts_with('{') {
                if let Ok(p) = serde_json::from_str::<OpenAiSsePayload>(raw) {
                    let delta = p.delta_text();
                    let usage = p.usage_into();
                    if !delta.is_empty() {
                        let _ = tx.send(GenerateChunk { delta, done: false, usage: usage.clone() }).await;
                    }
                    let _ = tx.send(GenerateChunk { delta: String::new(), done: true, usage }).await;
                    return Ok(());
                }
            }
            
            // If we received an NDJSON error or something else we don't understand.
            return Err(AiError::ModelError {
                reason: "模型回傳了無法識別的格式 (非 SSE 串流也非標準 JSON)。請確認 URL 與模型是否正確。".into(),
                raw: raw.chars().take(300).collect(),
            });
        }
    }

    if !saw_done {
        let _ = tx.send(GenerateChunk { delta: String::new(), done: true, usage: None }).await;
    }
    Ok(())
}

// ── 串流 + 工具呼叫 ───────────────────────────────────────────────────────────

/// `consume_openai_sse_with_tools` 的累積結果。
pub struct StreamedToolsResult {
    pub text: String,
    pub calls: Vec<crate::ai::AiToolCall>,
    /// 原封不動的 tool_calls JSON，供需要回填的模型（如 Gemini thinking）echo 回去。
    pub raw: Option<serde_json::Value>,
}

/// 像 `consume_openai_sse`，但同時累積 `tool_calls`。
///
/// 文字照樣邊收邊往 `tx` 送（這就是串流）；工具呼叫則是**分片**來的——同一個
/// 呼叫的 `arguments` 會被切成好幾段，只能靠 `index` 對位接回完整 JSON，所以
/// 不能邊收邊送，要等收齊。
pub async fn consume_openai_sse_with_tools(
    resp: reqwest::Response,
    tx: mpsc::Sender<GenerateChunk>,
) -> Result<StreamedToolsResult, AiError> {
    use std::collections::BTreeMap;

    let mut stream = resp.bytes_stream();
    let mut buf = Vec::<u8>::new();
    let mut text = String::new();
    // BTreeMap 讓輸出順序跟著 index 走，而不是片段抵達的順序。
    let mut acc: BTreeMap<usize, (Option<String>, Option<String>, String)> = BTreeMap::new();
    let mut saw_data_prefix = false;
    let mut raw_body = String::new();

    let mut stream_ended = false;
    let mut saw_terminator = false;
    while !stream_ended && !saw_terminator {
        match stream.next().await {
            Some(item) => {
                let bytes = item.map_err(|e| AiError::Network { message: e.to_string() })?;
                buf.extend_from_slice(&bytes);
            }
            None => {
                stream_ended = true;
                // 最後一行可能沒有換行結尾——非串流的 JSON 回應就是這樣。不補一個
                // 換行的話，整包內容會卡在 buffer 裡永遠不被處理。
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
                raw_body.push('\n');
            }
            if line.is_empty() { continue; }
            let payload = match line.strip_prefix("data:") {
                Some(p) => { saw_data_prefix = true; p.trim() }
                None => continue,
            };
            if payload == "[DONE]" { saw_terminator = true; break; }
            let Ok(p) = serde_json::from_str::<OpenAiSsePayload>(payload) else { continue };

            let delta = p.delta_text();
            if !delta.is_empty() {
                text.push_str(&delta);
                let _ = tx.send(GenerateChunk { delta, done: false, usage: p.usage_into() }).await;
            }
            for tc in p.tool_call_deltas() {
                let slot = acc.entry(tc.index).or_insert((None, None, String::new()));
                if let Some(id) = &tc.id { slot.0 = Some(id.clone()); }
                if let Some(f) = &tc.function {
                    if let Some(n) = &f.name { slot.1 = Some(n.clone()); }
                    if let Some(a) = &f.arguments { slot.2.push_str(a); }
                }
            }
            if p.finish_reason_present() {
                let _ = tx.send(GenerateChunk { delta: String::new(), done: true, usage: p.usage_into() }).await;
            }
        }
    }

    // 有些本地伺服器會忽略 stream:true，直接回一包普通 JSON。沒有這段的話
    // 使用者會拿到一則空白回覆，而且完全沒有錯誤訊息。
    if !saw_data_prefix {
        let raw = raw_body.trim();
        if raw.starts_with('{') {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) {
                let msg = &v["choices"][0]["message"];
                if let Some(arr) = msg["tool_calls"].as_array() {
                    let calls = arr.iter().map(|c| crate::ai::AiToolCall {
                        id: c["id"].as_str().unwrap_or("").to_string(),
                        tool_name: c["function"]["name"].as_str().unwrap_or("").to_string(),
                        args: serde_json::from_str(c["function"]["arguments"].as_str().unwrap_or("{}"))
                            .unwrap_or(serde_json::json!({})),
                        thought_signature: None,
                    }).collect();
                    return Ok(StreamedToolsResult { text: String::new(), calls, raw: Some(msg["tool_calls"].clone()) });
                }
                let content = msg["content"].as_str().unwrap_or("").to_string();
                let _ = tx.send(GenerateChunk { delta: content.clone(), done: true, usage: None }).await;
                return Ok(StreamedToolsResult { text: content, calls: vec![], raw: None });
            }
        }
    }

    let mut calls = Vec::new();
    let mut raw_calls = Vec::new();
    let acc_len = acc.len();
    for (i, (id, name, args)) in acc {
        let Some(name) = name else {
            // 只收到 arguments 卻沒有函式名，組不出呼叫。靜靜丟掉的話，這一輪
            // 會變成一則空白回覆——使用者只看得到一個空氣泡。
            log::warn!("[sse] 丟棄沒有函式名的工具呼叫片段 index={i} args_len={}", args.len());
            continue;
        };
        let id = id.unwrap_or_else(|| format!("call_{i}"));
        let args_json: serde_json::Value = serde_json::from_str(&args).unwrap_or(serde_json::json!({}));
        raw_calls.push(serde_json::json!({
            "id": id, "type": "function",
            "function": { "name": name, "arguments": args }
        }));
        calls.push(crate::ai::AiToolCall {
            id, tool_name: name, args: args_json, thought_signature: None,
        });
    }

    // 既沒文字也沒工具呼叫 = 使用者會看到一個空白氣泡，而且沒有任何線索。
    // 實測遇過一次（工具執行失敗之後那一輪），但沒能重現，所以先留下線索。
    if text.is_empty() && calls.is_empty() {
        log::warn!(
            "[sse] 這一輪既沒有文字也沒有工具呼叫（saw_data_prefix={saw_data_prefix} 片段數={acc_len}）；\
             開頭: {}",
            raw_body.chars().take(200).collect::<String>()
        );
    }

    let raw = if raw_calls.is_empty() { None } else { Some(serde_json::Value::Array(raw_calls)) };
    Ok(StreamedToolsResult { text, calls, raw })
}

// ── OpenAI SSE payload types ──────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct OpenAiSsePayload {
    #[serde(default)]
    choices: Vec<OpenAiSseChoice>,
    #[serde(default)]
    usage: Option<OpenAiSseUsage>,
}

#[derive(Deserialize)]
struct OpenAiSseChoice {
    #[serde(default)]
    delta: OpenAiSseDelta,
    #[serde(default)]
    message: Option<OpenAiSseDelta>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct OpenAiSseDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCallDelta>>,
}

/// 工具呼叫的一個片段。同一個呼叫會被切成多則，靠 `index` 對位。
#[derive(Deserialize, Default)]
pub(crate) struct ToolCallDelta {
    #[serde(default)]
    pub index: usize,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<FunctionDelta>,
}

#[derive(Deserialize, Default)]
pub(crate) struct FunctionDelta {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct OpenAiSseUsage {
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
}

impl OpenAiSsePayload {
    pub fn delta_text(&self) -> String {
        self.choices.first().and_then(|c| {
            if let Some(ref m) = c.message {
                if let Some(ref content) = m.content {
                    return Some(content.clone());
                }
            }
            c.delta.content.clone()
        }).unwrap_or_default()
    }
    pub fn tool_call_deltas(&self) -> &[ToolCallDelta] {
        self.choices
            .first()
            .and_then(|c| c.delta.tool_calls.as_deref())
            .unwrap_or(&[])
    }
    pub fn finish_reason_present(&self) -> bool {
        self.choices.first().and_then(|c| c.finish_reason.as_ref()).is_some()
    }
    pub fn usage_into(&self) -> Option<TokenUsage> {
        self.usage.as_ref().map(|u| TokenUsage {
            prompt: u.prompt_tokens,
            completion: u.completion_tokens,
        })
    }
}

// ── Byte helpers ──────────────────────────────────────────────────────────────

pub fn find_line_end(buf: &[u8]) -> Option<usize> {
    for (i, w) in buf.windows(2).enumerate() {
        if w == b"\r\n" { return Some(i); }
    }
    buf.iter().position(|&b| b == b'\n' || b == b'\r')
}

pub fn separator_len(buf: &[u8]) -> usize {
    match buf.first() {
        Some(&b'\r') if buf.get(1) == Some(&b'\n') => 2,
        Some(&b'\r') | Some(&b'\n') => 1,
        _ => 0,
    }
}

pub fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Map a non-2xx HTTP response to an `AiError`.
pub async fn map_http_error(status: reqwest::StatusCode, resp: reqwest::Response) -> AiError {
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return AiError::AuthFailed;
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let body = resp.text().await.unwrap_or_default();
        log::warn!("Rate limit (429): retry_after={retry_after:?} body={}", truncate(&body, 300));
        return AiError::RateLimit { retry_after, body: Some(body) };
    }
    let body = resp.text().await.unwrap_or_default();
    AiError::Network { message: format!("http {}: {}", status.as_u16(), truncate(&body, 200)) }
}
