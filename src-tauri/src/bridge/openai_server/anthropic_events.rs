//! Anthropic Messages SSE → [`UpstreamEvent`]。
//!
//! Claude Code 橋接對 Anthropic 家族上游走 passthrough（原樣 pipe），不需要
//! 解析。OpenAI 入口的輸出格式不同，所以這裡必須真的解析成中立事件，
//! 才能交給 OpenAI 序列化器。

use std::collections::{HashSet, VecDeque};

use futures_util::{Stream, StreamExt};
use serde_json::Value;

use crate::ai::sse::{find_line_end, separator_len};
use crate::ai::AiError;
use crate::bridge::upstream::{StopReason, UpstreamEvent, Usage};

type Item = Result<UpstreamEvent, AiError>;

#[derive(Default)]
pub struct AnthropicSseParser {
    /// 目前開著的 tool_use block 的 index；stop 時才知道要發 `ToolUseEnd`。
    tool_blocks: HashSet<u64>,
    stop_reason: Option<StopReason>,
    usage: Usage,
    done_sent: bool,
}

impl AnthropicSseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// 只處理 `data:` 行；`event:` 行沒有額外資訊（`type` 已在 JSON 裡）。
    pub fn feed_line(&mut self, line: &str) -> Vec<Item> {
        let Some(payload) = line.trim().strip_prefix("data:") else {
            return Vec::new();
        };
        // 壞掉的一行不該終止整個串流。
        let Ok(v) = serde_json::from_str::<Value>(payload.trim()) else {
            return Vec::new();
        };
        let index = v.get("index").and_then(Value::as_u64).unwrap_or(0);

        match v.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                if let Some(n) = v.pointer("/message/usage/input_tokens").and_then(Value::as_u64) {
                    self.usage.input_tokens = n as u32;
                }
                Vec::new()
            }
            Some("content_block_start") => {
                let block = &v["content_block"];
                if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                    self.tool_blocks.insert(index);
                    vec![Ok(UpstreamEvent::ToolUseStart {
                        id: block.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
                        name: block.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
                    })]
                } else {
                    Vec::new()
                }
            }
            Some("content_block_delta") => {
                let d = &v["delta"];
                match d.get("type").and_then(Value::as_str) {
                    Some("text_delta") => text(d, "text").map(UpstreamEvent::TextDelta),
                    Some("thinking_delta") => text(d, "thinking").map(UpstreamEvent::ThinkingDelta),
                    Some("input_json_delta") => text(d, "partial_json").map(UpstreamEvent::ToolInputDelta),
                    _ => None,
                }
                .map(|e| vec![Ok(e)])
                .unwrap_or_default()
            }
            Some("content_block_stop") => {
                if self.tool_blocks.remove(&index) {
                    vec![Ok(UpstreamEvent::ToolUseEnd)]
                } else {
                    Vec::new()
                }
            }
            Some("message_delta") => {
                if let Some(r) = v.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.stop_reason = Some(stop_reason(r));
                }
                if let Some(n) = v.pointer("/usage/output_tokens").and_then(Value::as_u64) {
                    self.usage.output_tokens = n as u32;
                }
                if let Some(n) = v.pointer("/usage/input_tokens").and_then(Value::as_u64) {
                    self.usage.input_tokens = n as u32;
                }
                Vec::new()
            }
            Some("message_stop") => self.finish(),
            Some("error") => {
                self.done_sent = true;
                let message = v.pointer("/error/message").and_then(Value::as_str).unwrap_or("上游回報錯誤");
                vec![Err(AiError::ModelError { reason: message.to_string(), raw: payload.trim().to_string() })]
            }
            // ping 與未來新增的事件型別：忽略。
            _ => Vec::new(),
        }
    }

    /// 串流結束時呼叫。已送過 `Done`（或錯誤）就回空，可安全無條件呼叫。
    pub fn finish(&mut self) -> Vec<Item> {
        if self.done_sent {
            return Vec::new();
        }
        self.done_sent = true;
        vec![Ok(UpstreamEvent::Done {
            stop_reason: self.stop_reason.unwrap_or(StopReason::EndTurn),
            usage: self.usage,
        })]
    }
}

fn text(d: &Value, key: &str) -> Option<String> {
    d.get(key).and_then(Value::as_str).map(str::to_string)
}

fn stop_reason(s: &str) -> StopReason {
    match s {
        "max_tokens" => StopReason::MaxTokens,
        "tool_use" => StopReason::ToolUse,
        "stop_sequence" => StopReason::StopSequence,
        _ => StopReason::EndTurn,
    }
}

/// HTTP 回應 → 事件串流。寫法照 `upstream/openai/client.rs::into_events`。
pub fn into_events(resp: reqwest::Response) -> impl Stream<Item = Item> {
    struct State {
        bytes: std::pin::Pin<Box<dyn Stream<Item = reqwest::Result<bytes::Bytes>> + Send>>,
        buf: Vec<u8>,
        parser: AnthropicSseParser,
        queued: VecDeque<Item>,
        ended: bool,
    }
    let state = State {
        bytes: Box::pin(resp.bytes_stream()),
        buf: Vec::new(),
        parser: AnthropicSseParser::new(),
        queued: VecDeque::new(),
        ended: false,
    };
    futures_util::stream::unfold(state, |mut s| async move {
        loop {
            if let Some(item) = s.queued.pop_front() {
                return Some((item, s));
            }
            if s.ended {
                return None;
            }
            if let Some(pos) = find_line_end(&s.buf) {
                let line_bytes: Vec<u8> = s.buf.drain(..pos).collect();
                let sep = separator_len(&s.buf);
                s.buf.drain(..sep);
                if let Ok(line) = std::str::from_utf8(&line_bytes) {
                    s.queued.extend(s.parser.feed_line(line));
                }
                continue;
            }
            match s.bytes.next().await {
                Some(Ok(chunk)) => s.buf.extend_from_slice(&chunk),
                Some(Err(e)) => {
                    s.ended = true;
                    return Some((Err(AiError::Network { message: e.to_string() }), s));
                }
                None => {
                    s.ended = true;
                    s.queued.extend(s.parser.finish());
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(lines: &[&str]) -> Vec<Item> {
        let mut p = AnthropicSseParser::new();
        let mut out: Vec<Item> = lines.iter().flat_map(|l| p.feed_line(l)).collect();
        out.extend(p.finish());
        out
    }

    fn ok(items: Vec<Item>) -> Vec<UpstreamEvent> {
        items.into_iter().map(|i| i.unwrap()).collect()
    }

    #[test]
    fn text_response() {
        let ev = ok(run(&[
            r#"event: message_start"#,
            r#"data: {"type":"message_start","message":{"usage":{"input_tokens":12,"output_tokens":1}}}"#,
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"ping"}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"你"}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"好"}}"#,
            r#"data: {"type":"content_block_stop","index":0}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":7}}"#,
            r#"data: {"type":"message_stop"}"#,
        ]));
        assert_eq!(
            ev,
            vec![
                UpstreamEvent::TextDelta("你".into()),
                UpstreamEvent::TextDelta("好".into()),
                UpstreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                    usage: Usage { input_tokens: 12, output_tokens: 7 },
                },
            ]
        );
    }

    #[test]
    fn tool_use_with_fragmented_json() {
        let ev = ok(run(&[
            r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"Read","input":{}}}"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"pa"}}"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"th\":1}"}}"#,
            r#"data: {"type":"content_block_stop","index":1}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":3}}"#,
            r#"data: {"type":"message_stop"}"#,
        ]));
        assert_eq!(ev[0], UpstreamEvent::ToolUseStart { id: "toolu_1".into(), name: "Read".into() });
        assert_eq!(ev[1], UpstreamEvent::ToolInputDelta("{\"pa".into()));
        assert_eq!(ev[2], UpstreamEvent::ToolInputDelta("th\":1}".into()));
        assert_eq!(ev[3], UpstreamEvent::ToolUseEnd);
        assert!(matches!(ev[4], UpstreamEvent::Done { stop_reason: StopReason::ToolUse, .. }));
    }

    #[test]
    fn text_block_stop_does_not_emit_tool_use_end() {
        let ev = ok(run(&[
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"content_block_stop","index":0}"#,
        ]));
        assert!(!ev.contains(&UpstreamEvent::ToolUseEnd), "{ev:?}");
    }

    #[test]
    fn thinking_delta_is_forwarded() {
        let ev = ok(run(&[
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"嗯"}}"#,
        ]));
        assert_eq!(ev[0], UpstreamEvent::ThinkingDelta("嗯".into()));
    }

    #[test]
    fn truncated_stream_still_gets_a_done() {
        let ev = ok(run(&[
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"a"}}"#,
        ]));
        assert!(matches!(ev.last(), Some(UpstreamEvent::Done { stop_reason: StopReason::EndTurn, .. })));
    }

    #[test]
    fn message_stop_then_finish_sends_done_only_once() {
        let ev = ok(run(&[r#"data: {"type":"message_stop"}"#]));
        assert_eq!(ev.iter().filter(|e| matches!(e, UpstreamEvent::Done { .. })).count(), 1);
    }

    #[test]
    fn error_event_becomes_an_err_and_suppresses_done() {
        let items = run(&[r#"data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#]);
        assert_eq!(items.len(), 1, "{items:?}");
        match &items[0] {
            Err(AiError::ModelError { reason, .. }) => assert_eq!(reason, "Overloaded"),
            other => panic!("預期 ModelError，實際 {other:?}"),
        }
    }

    #[test]
    fn garbage_lines_do_not_terminate_the_stream() {
        let ev = ok(run(&[
            "data: {not json",
            ": comment",
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"ok"}}"#,
        ]));
        assert_eq!(ev[0], UpstreamEvent::TextDelta("ok".into()));
    }
}
