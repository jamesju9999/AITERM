//! 把 [`UpstreamEvent`] 序列化成 Anthropic SSE frame。
//!
//! 這是整條翻譯管線唯一的輸出端 —— 每條上游路徑都收斂到 `UpstreamEvent`，
//! 事件序列的正確性只需要在這裡驗證一次。

use serde_json::{json, Value};

use crate::bridge::upstream::{StopReason, UpstreamEvent, Usage};

/// 一個完整的 SSE frame：`event: X\ndata: {...}\n\n`。
fn frame(event: &str, data: Value) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

/// Claude Code 在等上游第一個 byte 期間遇到靜默會斷線，且 SSE 註解
/// （`: ping`）不算資料。必須送完整的 event frame。
pub fn ping_frame() -> String {
    frame("ping", json!({"type": "ping"}))
}

pub fn error_frame(kind: &str, message: &str) -> String {
    frame(
        "error",
        json!({"type": "error", "error": {"type": kind, "message": message}}),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenBlock {
    Text,
    Thinking,
    ToolUse,
}

pub struct SseEncoder {
    message_id: String,
    model: String,
    input_tokens: u32,
    next_index: usize,
    open: Option<OpenBlock>,
    done_sent: bool,
}

impl SseEncoder {
    pub fn new(message_id: String, model: String, input_tokens: u32) -> Self {
        Self {
            message_id,
            model,
            input_tokens,
            next_index: 0,
            open: None,
            done_sent: false,
        }
    }

    pub fn start(&mut self) -> Vec<String> {
        vec![frame(
            "message_start",
            json!({
                "type": "message_start",
                "message": {
                    "id": self.message_id,
                    "type": "message",
                    "role": "assistant",
                    "model": self.model,
                    "content": [],
                    "stop_reason": Value::Null,
                    "stop_sequence": Value::Null,
                    "usage": {
                        "input_tokens": self.input_tokens,
                        "cache_creation_input_tokens": 0,
                        "cache_read_input_tokens": 0,
                        "output_tokens": 0,
                    },
                }
            }),
        )]
    }

    pub fn push(&mut self, ev: UpstreamEvent) -> Vec<String> {
        match ev {
            UpstreamEvent::TextDelta(t) => {
                let mut out = self.ensure_block(OpenBlock::Text, json!({"type": "text", "text": ""}));
                out.push(self.delta(json!({"type": "text_delta", "text": t})));
                out
            }
            UpstreamEvent::ThinkingDelta(t) => {
                let mut out = self.ensure_block(
                    OpenBlock::Thinking,
                    json!({"type": "thinking", "thinking": ""}),
                );
                out.push(self.delta(json!({"type": "thinking_delta", "thinking": t})));
                out
            }
            UpstreamEvent::ToolUseStart { id, name } => {
                // 工具區塊一律開新的：同一個回合可能連續呼叫多個工具。
                let mut out = self.close_open();
                out.push(self.open_block(
                    OpenBlock::ToolUse,
                    json!({"type": "tool_use", "id": id, "name": name, "input": {}}),
                ));
                out
            }
            UpstreamEvent::ToolInputDelta(partial) => {
                // 沒有開啟中的工具區塊時（未開過、或目前開的是文字/思考區塊），
                // input_json_delta 沒有合法的區塊可以掛，直接丟棄並記警告。
                if self.open != Some(OpenBlock::ToolUse) {
                    log::warn!(
                        "bridge::anthropic::response: 丟棄 ToolInputDelta，目前沒有開啟中的工具區塊（open={:?}）",
                        self.open
                    );
                    return Vec::new();
                }
                vec![self.delta(json!({"type": "input_json_delta", "partial_json": partial}))]
            }
            UpstreamEvent::ToolUseEnd => self.close_open(),
            UpstreamEvent::Done { stop_reason, usage } => {
                self.done_sent = true;
                self.tail(stop_reason, usage)
            }
        }
    }

    /// 上游串流在沒送 `Done` 的情況下結束時呼叫，補完收尾的 frame。
    /// 已經收過 `Done` 時回空 vec，可安全重複呼叫。
    pub fn finish(&mut self) -> Vec<String> {
        if self.done_sent {
            return Vec::new();
        }
        self.done_sent = true;
        self.tail(StopReason::EndTurn, Usage::default())
    }

    /// `Done` 與 `finish()` 共用的收尾序列：關掉開啟中的區塊，
    /// 接著 message_delta、message_stop。
    fn tail(&mut self, stop_reason: StopReason, usage: Usage) -> Vec<String> {
        let mut out = self.close_open();
        out.push(self.message_delta(stop_reason, usage));
        out.push(frame("message_stop", json!({"type": "message_stop"})));
        out
    }

    /// 目前開著的若已是同型別區塊就沿用，否則關掉舊的再開新的。
    fn ensure_block(&mut self, kind: OpenBlock, body: Value) -> Vec<String> {
        if self.open == Some(kind) {
            return Vec::new();
        }
        let mut out = self.close_open();
        out.push(self.open_block(kind, body));
        out
    }

    fn open_block(&mut self, kind: OpenBlock, body: Value) -> String {
        let index = self.next_index;
        self.next_index += 1;
        self.open = Some(kind);
        frame(
            "content_block_start",
            json!({"type": "content_block_start", "index": index, "content_block": body}),
        )
    }

    fn close_open(&mut self) -> Vec<String> {
        if self.open.take().is_none() {
            return Vec::new();
        }
        vec![frame(
            "content_block_stop",
            json!({"type": "content_block_stop", "index": self.current_index()}),
        )]
    }

    fn delta(&self, body: Value) -> String {
        frame(
            "content_block_delta",
            json!({"type": "content_block_delta", "index": self.current_index(), "delta": body}),
        )
    }

    /// 目前（或剛關閉的）區塊索引。`next_index` 永遠指向下一個。
    fn current_index(&self) -> usize {
        self.next_index.saturating_sub(1)
    }

    fn message_delta(&self, stop_reason: StopReason, usage: Usage) -> String {
        frame(
            "message_delta",
            json!({
                "type": "message_delta",
                "delta": {"stop_reason": stop_reason.as_str(), "stop_sequence": Value::Null},
                "usage": {
                    "input_tokens": usage.input_tokens,
                    "output_tokens": usage.output_tokens,
                }
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::upstream::{StopReason, Usage};

    fn encoder() -> SseEncoder {
        SseEncoder::new("msg_test".into(), "aiterm:sonnet".into(), 0)
    }

    #[test]
    fn message_start_reports_the_input_token_count() {
        let mut e = SseEncoder::new("msg_1".into(), "aiterm:sonnet".into(), 1234);
        let frames = e.start();
        assert!(frames[0].contains("\"input_tokens\":1234"), "實際：{}", frames[0]);
    }

    #[test]
    fn message_start_includes_cache_usage_fields() {
        // SDK 把這兩個欄位宣告成 number|null 而非 optional，缺席會讓 JS 端
        // 的加總變成 NaN。
        let mut e = SseEncoder::new("m".into(), "aiterm:sonnet".into(), 0);
        let f = &e.start()[0];
        assert!(f.contains("\"cache_creation_input_tokens\":0"), "實際：{f}");
        assert!(f.contains("\"cache_read_input_tokens\":0"), "實際：{f}");
    }

    #[test]
    fn message_start_echoes_the_model_string_it_was_given() {
        // 必須回 Claude Code 送來的哨兵字串，不是上游真實模型名。
        let mut e = SseEncoder::new("m".into(), "aiterm:opus".into(), 0);
        assert!(e.start()[0].contains("\"model\":\"aiterm:opus\""));
    }

    /// 從一串 frame 裡抽出 `event:` 行的名稱，方便斷言事件序列。
    fn names(frames: &[String]) -> Vec<String> {
        frames
            .iter()
            .filter_map(|f| f.lines().next())
            .map(|l| l.trim_start_matches("event: ").to_string())
            .collect()
    }

    #[test]
    fn ping_frame_is_a_real_event_not_a_comment() {
        // Claude Code 在等上游時遇到 SSE 靜默會斷線，而 SSE 註解（": ping"）
        // 無效，必須是完整的 event frame。
        let f = ping_frame();
        assert!(f.starts_with("event: ping\n"), "實際：{f}");
        assert!(f.contains("\"type\":\"ping\""));
        assert!(f.ends_with("\n\n"));
    }

    #[test]
    fn start_emits_message_start_only() {
        let mut e = encoder();
        assert_eq!(names(&e.start()), vec!["message_start"]);
    }

    #[test]
    fn text_delta_opens_a_block_then_reuses_it() {
        let mut e = encoder();
        e.start();
        let first = e.push(UpstreamEvent::TextDelta("你".into()));
        assert_eq!(names(&first), vec!["content_block_start", "content_block_delta"]);
        let second = e.push(UpstreamEvent::TextDelta("好".into()));
        assert_eq!(names(&second), vec!["content_block_delta"]);
        assert!(second[0].contains("\"text_delta\""));
    }

    #[test]
    fn switching_from_text_to_tool_closes_the_text_block() {
        let mut e = encoder();
        e.start();
        e.push(UpstreamEvent::TextDelta("先講話".into()));
        let frames = e.push(UpstreamEvent::ToolUseStart {
            id: "toolu_1".into(),
            name: "Read".into(),
        });
        assert_eq!(names(&frames), vec!["content_block_stop", "content_block_start"]);
        assert!(frames[1].contains("\"tool_use\""));
        assert!(frames[1].contains("\"Read\""));
    }

    #[test]
    fn tool_input_delta_uses_input_json_delta() {
        let mut e = encoder();
        e.start();
        e.push(UpstreamEvent::ToolUseStart { id: "t1".into(), name: "Read".into() });
        let frames = e.push(UpstreamEvent::ToolInputDelta("{\"a\":".into()));
        assert_eq!(names(&frames), vec!["content_block_delta"]);
        assert!(frames[0].contains("\"input_json_delta\""));
    }

    #[test]
    fn tool_input_delta_without_an_open_tool_block_is_dropped() {
        let mut e = encoder();
        e.start();
        assert_eq!(e.push(UpstreamEvent::ToolInputDelta("{}".into())), Vec::<String>::new());
    }

    #[test]
    fn tool_input_delta_is_dropped_while_a_text_block_is_open() {
        // 文字區塊吃不下 input_json_delta，掛上去會讓客戶端解析失敗。
        let mut e = encoder();
        e.start();
        e.push(UpstreamEvent::TextDelta("hi".into()));
        assert_eq!(e.push(UpstreamEvent::ToolInputDelta("{}".into())), Vec::<String>::new());
    }

    #[test]
    fn thinking_delta_uses_thinking_block() {
        let mut e = encoder();
        e.start();
        let frames = e.push(UpstreamEvent::ThinkingDelta("嗯".into()));
        assert!(frames[0].contains("\"thinking\""));
        assert!(frames[1].contains("\"thinking_delta\""));
    }

    #[test]
    fn done_closes_open_block_then_emits_delta_and_stop() {
        let mut e = encoder();
        e.start();
        e.push(UpstreamEvent::TextDelta("hi".into()));
        let frames = e.push(UpstreamEvent::Done {
            stop_reason: StopReason::ToolUse,
            usage: Usage { input_tokens: 10, output_tokens: 3 },
        });
        assert_eq!(
            names(&frames),
            vec!["content_block_stop", "message_delta", "message_stop"]
        );
        assert!(frames[1].contains("\"tool_use\""));
        assert!(frames[1].contains("\"output_tokens\":3"));
    }

    #[test]
    fn done_without_any_block_skips_the_stop() {
        let mut e = encoder();
        e.start();
        let frames = e.push(UpstreamEvent::Done {
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
        });
        assert_eq!(names(&frames), vec!["message_delta", "message_stop"]);
    }

    #[test]
    fn block_indices_increment() {
        let mut e = encoder();
        e.start();
        e.push(UpstreamEvent::TextDelta("a".into()));
        let frames = e.push(UpstreamEvent::ToolUseStart { id: "t".into(), name: "N".into() });
        assert!(frames[1].contains("\"index\":1"), "第二個區塊的 index 應為 1：{}", frames[1]);
    }

    #[test]
    fn finish_closes_a_dangling_stream() {
        let mut e = encoder();
        e.start();
        e.push(UpstreamEvent::TextDelta("被截斷".into()));
        assert_eq!(
            names(&e.finish()),
            vec!["content_block_stop", "message_delta", "message_stop"]
        );
    }

    #[test]
    fn finish_is_a_noop_after_done() {
        let mut e = encoder();
        e.start();
        e.push(UpstreamEvent::Done {
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
        });
        assert_eq!(e.finish(), Vec::<String>::new());
    }

    #[test]
    fn finish_is_idempotent() {
        let mut e = encoder();
        e.start();
        assert_eq!(names(&e.finish()), vec!["message_delta", "message_stop"]);
        assert_eq!(e.finish(), Vec::<String>::new());
    }

    #[test]
    fn error_frame_matches_anthropic_shape() {
        let f = error_frame("invalid_request_error", "壞掉了");
        assert!(f.starts_with("event: error\n"));
        assert!(f.contains("\"invalid_request_error\""));
        assert!(f.contains("壞掉了"));
    }
}
