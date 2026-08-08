//! Antigravity `v1internal:streamGenerateContent` SSE → [`UpstreamEvent`]。
//!
//! 事件形狀全部來自實測（`tests/antigravity_probe.rs`），跟 `ai/antigravity.rs`
//! 的既有邏輯共用同一份端點知識：
//!
//! 1. 每個 `data:` 行外層可能包一層 `{"response":{…}}` 信封，也可能是裸的
//!    chunk。信封要**先試**，因為所有欄位都是 `serde(default)`，順序反了
//!    會把信封靜默解析成空的 chunk（見 `ai/antigravity.rs:186` 附近的說明）。
//! 2. `args` 一次到齊，沒有觀察到跨 chunk 分片，所以一個 `functionCall`
//!    part 直接發完 `ToolUseStart` + `ToolInputDelta` + `ToolUseEnd`。
//! 3. `thoughtSignature` 掛在 part 上跟 `functionCall` 平行，不是每個並行
//!    呼叫都有——有的話才以 `functionCall.id` 為鍵存進 `tool_meta`。

use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;

use crate::ai::AiError;
use crate::bridge::tool_meta::ToolMetaCache;
use crate::bridge::upstream::{StopReason, UpstreamEvent, Usage};

pub struct StreamParser {
    tool_meta: Arc<ToolMetaCache>,
    saw_tool: bool,
    usage: Usage,
    done_sent: bool,
}

impl StreamParser {
    pub fn new(tool_meta: Arc<ToolMetaCache>) -> Self {
        Self { tool_meta, saw_tool: false, usage: Usage::default(), done_sent: false }
    }

    pub fn feed_line(&mut self, line: &str) -> Vec<UpstreamEvent> {
        let Some(chunk) = parse_chunk(line) else { return Vec::new() };
        let mut out = Vec::new();

        let candidate = chunk.candidates.into_iter().next();
        if let Some(content) = candidate.as_ref().and_then(|c| c.content.as_ref()) {
            for part in &content.parts {
                if let Some(text) = part.text.as_ref().filter(|t| !t.is_empty()) {
                    out.push(UpstreamEvent::TextDelta(text.clone()));
                }
                if let Some(call) = &part.function_call {
                    if let Some(sig) = &part.thought_signature {
                        self.tool_meta.insert(call.id.clone(), sig.clone());
                    }
                    self.saw_tool = true;
                    out.push(UpstreamEvent::ToolUseStart {
                        id: call.id.clone(),
                        name: call.name.clone(),
                    });
                    let args = serde_json::to_string(&call.args).unwrap_or_else(|_| "{}".into());
                    out.push(UpstreamEvent::ToolInputDelta(args));
                    out.push(UpstreamEvent::ToolUseEnd);
                }
            }
        }

        if let Some(u) = chunk.usage_metadata {
            self.usage = Usage {
                input_tokens: u.prompt_token_count,
                output_tokens: u.candidates_token_count,
            };
        }

        if let Some(reason) = candidate.and_then(|c| c.finish_reason) {
            let stop_reason = match reason.as_str() {
                "MAX_TOKENS" => StopReason::MaxTokens,
                _ if self.saw_tool => StopReason::ToolUse,
                _ => StopReason::EndTurn,
            };
            out.push(self.done(stop_reason));
        }

        out
    }

    /// 若這一行帶 `promptFeedback.blockReason`（請求被擋下），回傳對應的錯誤。
    pub fn take_error(&mut self, line: &str) -> Option<AiError> {
        let chunk = parse_chunk(line)?;
        if !chunk.candidates.is_empty() {
            return None;
        }
        let reason = chunk.prompt_feedback?.block_reason?;
        Some(AiError::ModelError { reason, raw: line.chars().take(300).collect() })
    }

    /// 串流結束時呼叫。冪等。
    pub fn finish(&mut self) -> Vec<UpstreamEvent> {
        if self.done_sent {
            return Vec::new();
        }
        let stop_reason = if self.saw_tool { StopReason::ToolUse } else { StopReason::EndTurn };
        vec![self.done(stop_reason)]
    }

    fn done(&mut self, stop_reason: StopReason) -> UpstreamEvent {
        self.done_sent = true;
        UpstreamEvent::Done { stop_reason, usage: self.usage }
    }
}

/// 解析一個 SSE `data:` 行成 [`GeminiStreamChunk`]。先試 Antigravity 的
/// `{"response":{…}}` 信封，再退回裸 chunk——順序見本模組文件註解。
/// 壞掉的一行不該終止整個串流，回 `None` 直接跳過。
fn parse_chunk(line: &str) -> Option<GeminiStreamChunk> {
    let data = line.trim().strip_prefix("data:")?.trim();
    if data.is_empty() {
        return None;
    }
    if let Ok(env) = serde_json::from_str::<StreamEnvelope>(data) {
        if let Some(chunk) = env.response {
            return Some(chunk);
        }
    }
    serde_json::from_str::<GeminiStreamChunk>(data).ok()
}

#[derive(Deserialize)]
struct StreamEnvelope {
    #[serde(default)]
    response: Option<GeminiStreamChunk>,
}

#[derive(Deserialize, Default)]
struct GeminiStreamChunk {
    #[serde(default)]
    candidates: Vec<GeminiCandidate>,
    #[serde(default, rename = "usageMetadata")]
    usage_metadata: Option<GeminiUsageMetadata>,
    #[serde(default, rename = "promptFeedback")]
    prompt_feedback: Option<GeminiPromptFeedback>,
}

#[derive(Deserialize)]
struct GeminiPromptFeedback {
    #[serde(default, rename = "blockReason")]
    block_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct GeminiCandidate {
    #[serde(default)]
    content: Option<GeminiContent>,
    #[serde(default, rename = "finishReason")]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct GeminiContent {
    #[serde(default)]
    parts: Vec<GeminiPart>,
}

#[derive(Deserialize, Default)]
struct GeminiPart {
    #[serde(default)]
    text: Option<String>,
    #[serde(default, rename = "functionCall")]
    function_call: Option<GeminiFunctionCall>,
    #[serde(default, rename = "thoughtSignature")]
    thought_signature: Option<Value>,
}

#[derive(Deserialize)]
struct GeminiFunctionCall {
    name: String,
    #[serde(default)]
    args: Value,
    id: String,
}

#[derive(Deserialize, Default)]
struct GeminiUsageMetadata {
    #[serde(default, rename = "promptTokenCount")]
    prompt_token_count: u32,
    #[serde(default, rename = "candidatesTokenCount")]
    candidates_token_count: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn parser() -> (StreamParser, Arc<ToolMetaCache>) {
        let cache = Arc::new(ToolMetaCache::new(512));
        (StreamParser::new(cache.clone()), cache)
    }

    fn run(lines: &[&str]) -> Vec<UpstreamEvent> {
        let (mut p, _) = parser();
        let mut out = Vec::new();
        for l in lines {
            out.extend(p.feed_line(l));
        }
        out.extend(p.finish());
        out
    }

    #[test]
    fn text_parts_become_text_deltas() {
        let ev = run(&[
            r#"data: {"response":{"candidates":[{"content":{"parts":[{"text":"你好"}]}}]}}"#,
        ]);
        assert_eq!(ev[0], UpstreamEvent::TextDelta("你好".into()));
    }

    #[test]
    fn bare_chunk_without_envelope_also_parses() {
        // ai/antigravity.rs 兩種都處理，因為所有欄位都是 serde default。
        let ev = run(&[r#"data: {"candidates":[{"content":{"parts":[{"text":"hi"}]}}]}"#]);
        assert_eq!(ev[0], UpstreamEvent::TextDelta("hi".into()));
    }

    #[test]
    fn function_call_emits_start_input_and_end_in_one_go() {
        // 實測：args 一次到齊，沒有分片。
        let ev = run(&[
            r#"data: {"response":{"candidates":[{"content":{"parts":[{"functionCall":{"name":"Read","args":{"p":1},"id":"c1"}}]}}]}}"#,
        ]);
        assert_eq!(ev[0], UpstreamEvent::ToolUseStart { id: "c1".into(), name: "Read".into() });
        assert_eq!(ev[1], UpstreamEvent::ToolInputDelta("{\"p\":1}".into()));
        assert_eq!(ev[2], UpstreamEvent::ToolUseEnd);
    }

    #[test]
    fn thought_signature_is_cached_keyed_by_call_id() {
        // 實測：回送時拿掉它會 400。
        let (mut p, cache) = parser();
        p.feed_line(
            r#"data: {"response":{"candidates":[{"content":{"parts":[{"thoughtSignature":"SIG","functionCall":{"name":"Read","args":{},"id":"c1"}}]}}]}}"#,
        );
        assert_eq!(cache.get("c1"), Some(serde_json::json!("SIG")));
    }

    #[test]
    fn a_call_without_a_signature_caches_nothing() {
        // 實測：三個並行呼叫只有第 1 個帶簽章，其餘欄位不存在。
        let (mut p, cache) = parser();
        p.feed_line(
            r#"data: {"response":{"candidates":[{"content":{"parts":[{"functionCall":{"name":"Read","args":{},"id":"c2"}}]}}]}}"#,
        );
        assert_eq!(cache.get("c2"), None);
    }

    #[test]
    fn parallel_calls_arrive_in_separate_chunks() {
        // 實測：每個 functionCall 各自獨立成一個 data: 事件。
        let ev = run(&[
            r#"data: {"response":{"candidates":[{"content":{"parts":[{"thoughtSignature":"S1","functionCall":{"name":"A","args":{},"id":"c1"}}]}}]}}"#,
            r#"data: {"response":{"candidates":[{"content":{"parts":[{"functionCall":{"name":"B","args":{},"id":"c2"}}]}}]}}"#,
        ]);
        let starts: Vec<_> = ev.iter().filter_map(|e| match e {
            UpstreamEvent::ToolUseStart { id, name } => Some((id.clone(), name.clone())),
            _ => None,
        }).collect();
        assert_eq!(starts, vec![("c1".into(), "A".into()), ("c2".into(), "B".into())]);
    }

    #[test]
    fn finish_reason_ends_the_stream_with_usage() {
        let ev = run(&[
            r#"data: {"response":{"candidates":[{"content":{"parts":[{"text":"hi"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":7,"candidatesTokenCount":3}}}"#,
        ]);
        match ev.last().unwrap() {
            UpstreamEvent::Done { stop_reason, usage } => {
                assert_eq!(*stop_reason, StopReason::EndTurn);
                assert_eq!(usage.input_tokens, 7);
                assert_eq!(usage.output_tokens, 3);
            }
            other => panic!("預期 Done，實際 {other:?}"),
        }
    }

    #[test]
    fn stop_reason_is_tool_use_when_a_tool_was_called() {
        let ev = run(&[
            r#"data: {"response":{"candidates":[{"content":{"parts":[{"functionCall":{"name":"A","args":{},"id":"c1"}}]}}]}}"#,
            r#"data: {"response":{"candidates":[{"finishReason":"STOP"}],"usageMetadata":{}}}"#,
        ]);
        assert!(matches!(
            ev.last().unwrap(),
            UpstreamEvent::Done { stop_reason: StopReason::ToolUse, .. }
        ));
    }

    #[test]
    fn max_tokens_finish_reason_maps_to_max_tokens() {
        let ev = run(&[
            r#"data: {"response":{"candidates":[{"finishReason":"MAX_TOKENS"}],"usageMetadata":{}}}"#,
        ]);
        assert!(matches!(
            ev.last().unwrap(),
            UpstreamEvent::Done { stop_reason: StopReason::MaxTokens, .. }
        ));
    }

    #[test]
    fn empty_text_parts_emit_nothing() {
        // 實測的 dump 尾端有一個 {"text":""}。
        let ev = run(&[
            r#"data: {"response":{"candidates":[{"content":{"parts":[{"text":""}]}}]}}"#,
        ]);
        assert!(!ev.iter().any(|e| matches!(e, UpstreamEvent::TextDelta(_))));
    }

    #[test]
    fn prompt_feedback_block_surfaces_an_error() {
        let (mut p, _) = parser();
        let err = p.take_error(
            r#"data: {"response":{"candidates":[],"promptFeedback":{"blockReason":"SAFETY"}}}"#,
        );
        assert!(err.is_some(), "被擋下的請求必須產生錯誤");
    }

    #[test]
    fn malformed_json_is_skipped_not_fatal() {
        let ev = run(&[
            "data: {not json",
            r#"data: {"response":{"candidates":[{"content":{"parts":[{"text":"ok"}]}}]}}"#,
        ]);
        assert_eq!(ev[0], UpstreamEvent::TextDelta("ok".into()));
    }

    #[test]
    fn stream_ending_without_finish_reason_still_emits_done() {
        let ev = run(&[
            r#"data: {"response":{"candidates":[{"content":{"parts":[{"text":"a"}]}}]}}"#,
        ]);
        assert!(matches!(ev.last(), Some(UpstreamEvent::Done { .. })));
    }
}
