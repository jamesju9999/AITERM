//! Codex Responses API SSE → [`UpstreamEvent`]。
//!
//! 事件名稱與欄位位置全部來自實測（`tests/codex_probe.rs`）。三個跟公開
//! 文件不同、會讓「照文件寫」出錯的地方：
//!
//! 1. `response.completed.response.output` 是**空陣列**，即使呼叫了工具。
//!    權威記錄只在串流過程的 `response.output_item.done`。
//! 2. `call_id` 與 `name` 在 `item` 物件上，不在 delta 事件裡。
//! 3. arguments 分片與否跟長度有關 —— 短內容一次到齊，**不能假設一定
//!    收得到 delta**，所以 `.done` 要能補位。

use std::collections::BTreeMap;

use serde_json::Value;

use crate::ai::AiError;
use crate::bridge::upstream::{StopReason, UpstreamEvent, Usage};

#[derive(Debug, Default)]
struct Slot {
    // 目前只在 on_item_added 寫入，finish() 收尾時不需要按 call 區分。
    // 保留是為了在未來需要（例如逐 call 記錄診斷資訊）時不必重新接線。
    #[allow(dead_code)]
    call_id: String,
    /// 是否已經收過至少一個 arguments delta。沒收過的話 `.done` 要補位。
    saw_delta: bool,
    closed: bool,
}

#[derive(Default)]
pub struct StreamParser {
    /// 以 `item_id` 分桶。實測時並行呼叫沒有交錯，但探勘明確標註
    /// 「沒觀察到 ≠ 不會發生」，分桶在兩種情況下都正確。
    slots: BTreeMap<String, Slot>,
    saw_tool: bool,
    usage: Usage,
    done_sent: bool,
}

impl StreamParser {
    pub fn feed_line(&mut self, line: &str) -> Vec<UpstreamEvent> {
        let Some(v) = parse_data_line(line) else { return Vec::new() };
        let Some(ty) = v.get("type").and_then(Value::as_str) else { return Vec::new() };

        match ty {
            "response.output_text.delta" => {
                str_field(&v, "delta").map(|t| vec![UpstreamEvent::TextDelta(t)]).unwrap_or_default()
            }
            "response.reasoning_summary_text.delta" => str_field(&v, "delta")
                .map(|t| vec![UpstreamEvent::ThinkingDelta(t)])
                .unwrap_or_default(),
            "response.output_item.added" => self.on_item_added(&v),
            "response.function_call_arguments.delta" => self.on_args_delta(&v),
            "response.function_call_arguments.done" => self.on_args_done(&v),
            "response.completed" => {
                if let Some(u) = v.get("response").and_then(|r| r.get("usage")) {
                    self.usage = Usage {
                        input_tokens: u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
                        output_tokens: u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
                    };
                }
                self.finish()
            }
            // response.created / in_progress / content_part.* / output_item.done /
            // reasoning_summary_part.* 等：不影響輸出，忽略。
            _ => Vec::new(),
        }
    }

    /// 若這一行是 `response.failed`，回傳對應的錯誤。
    pub fn take_error(&mut self, line: &str) -> Option<AiError> {
        let v = parse_data_line(line)?;
        if v.get("type").and_then(Value::as_str) != Some("response.failed") {
            return None;
        }
        let raw = v.get("response").map(|r| r.to_string()).unwrap_or_default();
        let reason = v
            .get("response")
            .and_then(|r| r.get("error"))
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("Codex 回報請求失敗")
            .to_string();
        Some(AiError::ModelError { reason, raw })
    }

    /// 串流結束時呼叫。冪等。
    pub fn finish(&mut self) -> Vec<UpstreamEvent> {
        if self.done_sent {
            return Vec::new();
        }
        self.done_sent = true;
        let mut out = Vec::new();
        // 關掉還開著的工具區塊，避免客戶端等一個永遠不來的收尾。
        let open: Vec<String> = self
            .slots
            .iter()
            .filter(|(_, s)| !s.closed)
            .map(|(k, _)| k.clone())
            .collect();
        for k in open {
            if let Some(s) = self.slots.get_mut(&k) {
                s.closed = true;
            }
            out.push(UpstreamEvent::ToolUseEnd);
        }
        let stop_reason = if self.saw_tool { StopReason::ToolUse } else { StopReason::EndTurn };
        out.push(UpstreamEvent::Done { stop_reason, usage: self.usage });
        out
    }

    fn on_item_added(&mut self, v: &Value) -> Vec<UpstreamEvent> {
        let Some(item) = v.get("item") else { return Vec::new() };
        // output_item.added 也用於 reasoning，必須靠 item.type 分辨。
        if item.get("type").and_then(Value::as_str) != Some("function_call") {
            return Vec::new();
        }
        let Some(item_id) = item.get("id").and_then(Value::as_str) else { return Vec::new() };
        let call_id = item.get("call_id").and_then(Value::as_str).unwrap_or_default().to_string();
        let name = item.get("name").and_then(Value::as_str).unwrap_or_default().to_string();

        self.saw_tool = true;
        self.slots.insert(
            item_id.to_string(),
            Slot { call_id: call_id.clone(), saw_delta: false, closed: false },
        );
        // added 就同時帶了 call_id 與 name，可以立刻開區塊 —— 不需要
        // OpenAI 路徑那種延後邏輯。往返用 call_id 不是 item id。
        vec![UpstreamEvent::ToolUseStart { id: call_id, name }]
    }

    fn on_args_delta(&mut self, v: &Value) -> Vec<UpstreamEvent> {
        let Some(item_id) = v.get("item_id").and_then(Value::as_str) else { return Vec::new() };
        let Some(delta) = str_field(v, "delta") else { return Vec::new() };
        if let Some(s) = self.slots.get_mut(item_id) {
            s.saw_delta = true;
        }
        vec![UpstreamEvent::ToolInputDelta(delta)]
    }

    fn on_args_done(&mut self, v: &Value) -> Vec<UpstreamEvent> {
        let Some(item_id) = v.get("item_id").and_then(Value::as_str) else { return Vec::new() };
        let Some(s) = self.slots.get_mut(item_id) else { return Vec::new() };
        if s.closed {
            return Vec::new();
        }
        let mut out = Vec::new();
        // 短參數可能完全沒有 delta，這時 done 的完整值就是唯一來源。
        if !s.saw_delta {
            if let Some(args) = str_field(v, "arguments") {
                out.push(UpstreamEvent::ToolInputDelta(args));
            }
        }
        s.closed = true;
        out.push(UpstreamEvent::ToolUseEnd);
        out
    }
}

fn parse_data_line(line: &str) -> Option<Value> {
    let payload = line.trim().strip_prefix("data:")?.trim();
    if payload.is_empty() || payload == "[DONE]" {
        return None;
    }
    // 壞掉的一行不該終止整個串流。
    serde_json::from_str(payload).ok()
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
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

    fn added_fn(item_id: &str, call_id: &str, name: &str) -> String {
        format!(
            r#"data: {{"type":"response.output_item.added","output_index":0,"item":{{"id":"{item_id}","type":"function_call","status":"in_progress","arguments":"","call_id":"{call_id}","name":"{name}"}}}}"#
        )
    }

    #[test]
    fn text_deltas_are_forwarded() {
        let ev = run(&[
            r#"data: {"type":"response.output_text.delta","delta":"你"}"#,
            r#"data: {"type":"response.output_text.delta","delta":"好"}"#,
            r#"data: {"type":"response.completed","response":{"usage":{"input_tokens":5,"output_tokens":2}}}"#,
        ]);
        assert_eq!(ev[0], UpstreamEvent::TextDelta("你".into()));
        assert_eq!(ev[1], UpstreamEvent::TextDelta("好".into()));
    }

    #[test]
    fn reasoning_summary_becomes_thinking_delta() {
        // 實測的事件名稱，不是從文件推的。
        let ev = run(&[
            r#"data: {"type":"response.reasoning_summary_text.delta","delta":"嗯","item_id":"rs_1"}"#,
            r#"data: {"type":"response.completed","response":{"usage":{}}}"#,
        ]);
        assert_eq!(ev[0], UpstreamEvent::ThinkingDelta("嗯".into()));
    }

    #[test]
    fn tool_use_starts_immediately_on_output_item_added() {
        // 比 OpenAI 路徑簡單的地方：added 就同時帶了 call_id 與 name，
        // 不需要緩衝到參數到達才發 ToolUseStart。
        let ev = run(&[&added_fn("fc_1", "call_1", "Read")]);
        assert_eq!(
            ev[0],
            UpstreamEvent::ToolUseStart { id: "call_1".into(), name: "Read".into() }
        );
    }

    #[test]
    fn tool_use_id_is_the_call_id_not_the_item_id() {
        // 往返用 call_id；fc_ 開頭的 id 是內部識別碼。
        let ev = run(&[&added_fn("fc_1", "call_1", "Read")]);
        match &ev[0] {
            UpstreamEvent::ToolUseStart { id, .. } => assert_eq!(id, "call_1"),
            other => panic!("預期 ToolUseStart，實際 {other:?}"),
        }
    }

    #[test]
    fn argument_fragments_are_forwarded_verbatim() {
        let ev = run(&[
            &added_fn("fc_1", "call_1", "Read"),
            r#"data: {"type":"response.function_call_arguments.delta","delta":"{\"a\"","item_id":"fc_1","output_index":0}"#,
            r#"data: {"type":"response.function_call_arguments.delta","delta":":1}","item_id":"fc_1","output_index":0}"#,
            r#"data: {"type":"response.function_call_arguments.done","arguments":"{\"a\":1}","item_id":"fc_1","output_index":0}"#,
        ]);
        assert_eq!(ev[1], UpstreamEvent::ToolInputDelta("{\"a\"".into()));
        assert_eq!(ev[2], UpstreamEvent::ToolInputDelta(":1}".into()));
        assert_eq!(ev[3], UpstreamEvent::ToolUseEnd);
    }

    #[test]
    fn done_supplies_arguments_when_no_delta_arrived() {
        // 實測：arguments 分片與否跟長度有關，短內容可能一次到齊。
        // 若完全沒有 delta，必須用 done 的完整值補上，否則工具呼叫會沒有參數。
        let ev = run(&[
            &added_fn("fc_1", "call_1", "Read"),
            r#"data: {"type":"response.function_call_arguments.done","arguments":"{\"a\":1}","item_id":"fc_1","output_index":0}"#,
        ]);
        assert_eq!(ev[1], UpstreamEvent::ToolInputDelta("{\"a\":1}".into()));
        assert_eq!(ev[2], UpstreamEvent::ToolUseEnd);
    }

    #[test]
    fn done_does_not_duplicate_already_streamed_arguments() {
        let ev = run(&[
            &added_fn("fc_1", "call_1", "Read"),
            r#"data: {"type":"response.function_call_arguments.delta","delta":"{}","item_id":"fc_1","output_index":0}"#,
            r#"data: {"type":"response.function_call_arguments.done","arguments":"{}","item_id":"fc_1","output_index":0}"#,
        ]);
        let deltas = ev.iter().filter(|e| matches!(e, UpstreamEvent::ToolInputDelta(_))).count();
        assert_eq!(deltas, 1, "done 不能重複送一次參數：{ev:?}");
    }

    #[test]
    fn two_parallel_tool_calls_are_bucketed_by_item_id() {
        // 實測時兩個呼叫沒有交錯，但探勘明確標註「沒觀察到 ≠ 不會發生」，
        // 所以用 item_id 分桶，交錯與否都正確。
        let ev = run(&[
            &added_fn("fc_1", "call_1", "A"),
            &added_fn("fc_2", "call_2", "B"),
            r#"data: {"type":"response.function_call_arguments.delta","delta":"{\"x\":1}","item_id":"fc_2","output_index":1}"#,
            r#"data: {"type":"response.function_call_arguments.delta","delta":"{\"y\":2}","item_id":"fc_1","output_index":0}"#,
            r#"data: {"type":"response.function_call_arguments.done","arguments":"{\"y\":2}","item_id":"fc_1","output_index":0}"#,
            r#"data: {"type":"response.function_call_arguments.done","arguments":"{\"x\":1}","item_id":"fc_2","output_index":1}"#,
        ]);
        // 兩個工具各自開一個區塊，參數不能串到對方身上。
        let starts: Vec<_> = ev.iter().filter_map(|e| match e {
            UpstreamEvent::ToolUseStart { id, name } => Some((id.clone(), name.clone())),
            _ => None,
        }).collect();
        assert_eq!(starts.len(), 2);
        assert!(starts.contains(&("call_1".to_string(), "A".to_string())));
        assert!(starts.contains(&("call_2".to_string(), "B".to_string())));
    }

    #[test]
    fn reasoning_output_item_does_not_start_a_tool() {
        // output_item.added 也用於 reasoning，必須靠 item.type 分辨。
        let ev = run(&[
            r#"data: {"type":"response.output_item.added","output_index":0,"item":{"id":"rs_1","type":"reasoning","summary":[]}}"#,
        ]);
        assert!(
            !ev.iter().any(|e| matches!(e, UpstreamEvent::ToolUseStart { .. })),
            "reasoning item 不該被當成工具呼叫：{ev:?}"
        );
    }

    #[test]
    fn stop_reason_is_tool_use_when_a_tool_was_called() {
        let ev = run(&[
            &added_fn("fc_1", "call_1", "Read"),
            r#"data: {"type":"response.function_call_arguments.done","arguments":"{}","item_id":"fc_1","output_index":0}"#,
            r#"data: {"type":"response.completed","response":{"usage":{"input_tokens":5,"output_tokens":2}}}"#,
        ]);
        match ev.last().unwrap() {
            UpstreamEvent::Done { stop_reason, usage } => {
                assert_eq!(*stop_reason, StopReason::ToolUse);
                assert_eq!(usage.input_tokens, 5);
                assert_eq!(usage.output_tokens, 2);
            }
            other => panic!("預期 Done，實際 {other:?}"),
        }
    }

    #[test]
    fn stop_reason_is_end_turn_for_plain_text() {
        let ev = run(&[
            r#"data: {"type":"response.output_text.delta","delta":"hi"}"#,
            r#"data: {"type":"response.completed","response":{"usage":{}}}"#,
        ]);
        assert!(matches!(
            ev.last().unwrap(),
            UpstreamEvent::Done { stop_reason: StopReason::EndTurn, .. }
        ));
    }

    #[test]
    fn response_failed_surfaces_an_error() {
        let mut p = StreamParser::default();
        let err = p.take_error(
            r#"data: {"type":"response.failed","response":{"error":{"message":"boom"}}}"#,
        );
        assert!(err.is_some(), "response.failed 必須產生錯誤");
    }

    #[test]
    fn unknown_events_are_ignored() {
        let ev = run(&[
            r#"data: {"type":"response.created"}"#,
            r#"data: {"type":"response.in_progress"}"#,
            r#"data: {"type":"response.content_part.added"}"#,
            r#"data: {"type":"response.output_text.delta","delta":"ok"}"#,
            r#"data: {"type":"response.completed","response":{"usage":{}}}"#,
        ]);
        assert_eq!(ev[0], UpstreamEvent::TextDelta("ok".into()));
    }

    #[test]
    fn malformed_json_is_skipped_not_fatal() {
        let ev = run(&[
            "data: {not json",
            r#"data: {"type":"response.output_text.delta","delta":"ok"}"#,
            r#"data: {"type":"response.completed","response":{"usage":{}}}"#,
        ]);
        assert_eq!(ev[0], UpstreamEvent::TextDelta("ok".into()));
    }

    #[test]
    fn stream_ending_without_completed_still_emits_done() {
        // 實測沒遇到，但串流中斷是常態，不能讓客戶端永遠等下去。
        let ev = run(&[r#"data: {"type":"response.output_text.delta","delta":"a"}"#]);
        assert!(matches!(ev.last(), Some(UpstreamEvent::Done { .. })));
    }
}
