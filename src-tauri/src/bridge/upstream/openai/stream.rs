//! OpenAI chat.completions SSE → [`UpstreamEvent`]。
//!
//! 為什麼不用 `ai::sse::consume_openai_sse`：那支的輸出型別是
//! `GenerateChunk { delta: String }`，工具呼叫在型別上就表達不出來。行切分
//! 的工具函式仍然共用。

use serde_json::Value;

use super::tool_calls::ToolCallAccumulator;
use crate::bridge::upstream::{StopReason, UpstreamEvent, Usage};

/// 逐行餵入的 SSE 解析器。呼叫端負責把 byte 串切成行
/// （見 `ai::sse::find_line_end`）。
#[derive(Default)]
pub struct StreamParser {
    tools: ToolCallAccumulator,
    /// 收到 finish_reason 後暫存，等 usage 那一片到了才發 Done。
    pending_stop: Option<StopReason>,
    usage: Usage,
    done_sent: bool,
}

impl StreamParser {
    pub fn feed_line(&mut self, line: &str) -> Vec<UpstreamEvent> {
        let line = line.trim();
        let Some(payload) = line.strip_prefix("data:") else {
            return Vec::new();
        };
        let payload = payload.trim();
        if payload == "[DONE]" {
            return self.finish();
        }
        let Ok(v) = serde_json::from_str::<Value>(payload) else {
            // 壞掉的一行不該終止整個串流：部分端點會夾雜心跳或非標準行。
            return Vec::new();
        };

        let mut out = Vec::new();
        if let Some(u) = v.get("usage") {
            self.usage = Usage {
                input_tokens: u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
                output_tokens: u.get("completion_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
            };
        }

        let Some(choice) = v.get("choices").and_then(Value::as_array).and_then(|c| c.first()) else {
            return out;
        };
        if let Some(delta) = choice.get("delta") {
            if let Some(t) = delta.get("content").and_then(Value::as_str) {
                if !t.is_empty() {
                    out.push(UpstreamEvent::TextDelta(t.to_string()));
                }
            }
            // DeepSeek 用 reasoning_content，部分相容端點用 reasoning。
            for key in ["reasoning_content", "reasoning"] {
                if let Some(t) = delta.get(key).and_then(Value::as_str) {
                    if !t.is_empty() {
                        out.push(UpstreamEvent::ThinkingDelta(t.to_string()));
                    }
                }
            }
            if let Some(tc) = delta.get("tool_calls").and_then(Value::as_array) {
                out.extend(self.tools.push(tc));
            }
        }
        if let Some(fr) = choice.get("finish_reason").and_then(Value::as_str) {
            self.pending_stop = Some(map_stop_reason(fr));
        }
        out
    }

    /// 串流結束（收到 `[DONE]` 或連線關閉）時呼叫。冪等。
    pub fn finish(&mut self) -> Vec<UpstreamEvent> {
        if self.done_sent {
            return Vec::new();
        }
        self.done_sent = true;
        let mut out = self.tools.finish();
        // 上游若沒給 finish_reason，用工具呼叫的有無來推斷。
        let stop_reason = self.pending_stop.unwrap_or(if self.tools.saw_any() {
            StopReason::ToolUse
        } else {
            StopReason::EndTurn
        });
        out.push(UpstreamEvent::Done { stop_reason, usage: self.usage });
        out
    }
}

fn map_stop_reason(fr: &str) -> StopReason {
    match fr {
        "length" => StopReason::MaxTokens,
        "tool_calls" | "function_call" => StopReason::ToolUse,
        "stop_sequence" => StopReason::StopSequence,
        // content_filter 等其餘一律當正常結束 —— Anthropic 沒有對應的
        // stop_reason，回未知值會讓 Claude Code 解析失敗。
        _ => StopReason::EndTurn,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 把幾個 SSE 資料行餵進解析器，收集全部事件。
    fn run(lines: &[&str]) -> Vec<UpstreamEvent> {
        let mut p = StreamParser::default();
        let mut out = Vec::new();
        for l in lines {
            out.extend(p.feed_line(l));
        }
        out.extend(p.finish());
        out
    }

    #[test]
    fn text_deltas_are_forwarded() {
        let ev = run(&[
            r#"data: {"choices":[{"delta":{"content":"你"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"好"}}]}"#,
            "data: [DONE]",
        ]);
        assert_eq!(ev[0], UpstreamEvent::TextDelta("你".into()));
        assert_eq!(ev[1], UpstreamEvent::TextDelta("好".into()));
    }

    #[test]
    fn non_data_lines_are_ignored() {
        let ev = run(&["", ": comment", "event: whatever", "data: [DONE]"]);
        assert!(matches!(ev.as_slice(), [UpstreamEvent::Done { .. }]));
    }

    #[test]
    fn malformed_json_is_skipped_not_fatal() {
        let ev = run(&[
            "data: {not json",
            r#"data: {"choices":[{"delta":{"content":"ok"}}]}"#,
            "data: [DONE]",
        ]);
        assert_eq!(ev[0], UpstreamEvent::TextDelta("ok".into()));
    }

    #[test]
    fn reasoning_content_becomes_thinking_delta() {
        let ev = run(&[
            r#"data: {"choices":[{"delta":{"reasoning_content":"嗯"}}]}"#,
            "data: [DONE]",
        ]);
        assert_eq!(ev[0], UpstreamEvent::ThinkingDelta("嗯".into()));
    }

    #[test]
    fn tool_calls_produce_tool_events() {
        let ev = run(&[
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"Read","arguments":"{}"}}]}}]}"#,
            r#"data: {"choices":[{"finish_reason":"tool_calls"}]}"#,
            "data: [DONE]",
        ]);
        assert_eq!(ev[0], UpstreamEvent::ToolUseStart { id: "c1".into(), name: "Read".into() });
        assert_eq!(ev[1], UpstreamEvent::ToolInputDelta("{}".into()));
        assert_eq!(ev[2], UpstreamEvent::ToolUseEnd);
        assert!(matches!(
            ev[3],
            UpstreamEvent::Done { stop_reason: StopReason::ToolUse, .. }
        ));
    }

    #[test]
    fn finish_reason_maps_to_stop_reason() {
        let mk = |fr: &str| {
            let line = format!(r#"data: {{"choices":[{{"finish_reason":"{fr}"}}]}}"#);
            match run(&[&line, "data: [DONE]"]).into_iter().next().unwrap() {
                UpstreamEvent::Done { stop_reason, .. } => stop_reason,
                other => panic!("預期 Done，實際 {other:?}"),
            }
        };
        assert_eq!(mk("stop"), StopReason::EndTurn);
        assert_eq!(mk("length"), StopReason::MaxTokens);
        assert_eq!(mk("tool_calls"), StopReason::ToolUse);
        assert_eq!(mk("content_filter"), StopReason::EndTurn);
    }

    #[test]
    fn usage_is_captured_even_when_it_arrives_after_finish_reason() {
        // stream_options.include_usage 會讓 usage 出現在最後一個 chunk，
        // 也就是 finish_reason 之後。
        let ev = run(&[
            r#"data: {"choices":[{"finish_reason":"stop"}]}"#,
            r#"data: {"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":34}}"#,
            "data: [DONE]",
        ]);
        let done = ev.iter().find(|e| matches!(e, UpstreamEvent::Done { .. })).unwrap();
        match done {
            UpstreamEvent::Done { usage, .. } => {
                assert_eq!(usage.input_tokens, 12);
                assert_eq!(usage.output_tokens, 34);
            }
            _ => unreachable!(),
        }
        assert_eq!(
            ev.iter().filter(|e| matches!(e, UpstreamEvent::Done { .. })).count(),
            1,
            "Done 只能發一次"
        );
    }

    #[test]
    fn stream_ending_without_done_still_emits_done() {
        let ev = run(&[r#"data: {"choices":[{"delta":{"content":"a"}}]}"#]);
        assert!(matches!(ev.last(), Some(UpstreamEvent::Done { .. })));
    }
}
