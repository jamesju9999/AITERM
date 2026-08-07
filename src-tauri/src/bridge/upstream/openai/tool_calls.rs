//! OpenAI 串流工具呼叫的累積器。
//!
//! OpenAI 把一個工具呼叫拆成多個 `delta.tool_calls[]` 片段：`id` 與
//! `function.name` 通常先到，`function.arguments` 分很多片後到。Anthropic 的
//! `content_block_start` 卻要求一開始就帶完整的 name，所以必須緩衝到名稱
//! 確定才發第一個事件。
//!
//! 「名稱確定」的判準：收到第一個非空的 arguments 片段、串流結束，或是
//! 在 id 已經確定之後，單獨到達的名稱片段（此時不會再有新的 id 更新，
//! 這片段就是名稱的最後一塊，可以直接發 `ToolUseStart`）。

use std::collections::BTreeMap;

use serde_json::Value;

use crate::bridge::upstream::UpstreamEvent;

#[derive(Debug, Default)]
struct Slot {
    id: String,
    name: String,
    started: bool,
}

#[derive(Debug, Default)]
pub struct ToolCallAccumulator {
    slots: BTreeMap<u64, Slot>,
    /// 目前已發出 `ToolUseStart` 但還沒 `ToolUseEnd` 的槽位。
    active: Option<u64>,
    saw_any: bool,
}

impl ToolCallAccumulator {
    /// 餵進一個 SSE chunk 的 `delta.tool_calls` 陣列，回傳要發出的事件。
    pub fn push(&mut self, tool_calls: &[Value]) -> Vec<UpstreamEvent> {
        let mut out = Vec::new();
        for tc in tool_calls {
            let index = tc.get("index").and_then(Value::as_u64).unwrap_or(0);
            self.saw_any = true;

            let id = tc.get("id").and_then(Value::as_str).filter(|s| !s.is_empty());
            let name = tc.get("function").and_then(|f| f.get("name")).and_then(Value::as_str);

            let slot = self.slots.entry(index).or_default();
            if let Some(id) = id {
                slot.id = id.to_string();
            }
            if let Some(name) = name {
                slot.name.push_str(name);
            }

            let args = tc
                .get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(Value::as_str)
                .unwrap_or_default();

            if !args.is_empty() {
                // 有參數進來就代表名稱已經完結，可以開區塊了。
                out.extend(self.activate(index));
                out.push(UpstreamEvent::ToolInputDelta(args.to_string()));
            } else if id.is_none() && name.is_some() {
                // 這個片段沒帶 id（id 在更早的片段就確定過了），卻補上了
                // 名稱——代表名稱片段已經送完，即使還沒看到參數也視為確定。
                out.extend(self.activate(index));
            }
        }
        out
    }

    /// 串流結束時呼叫：把還沒開的槽位補開、關掉開著的區塊。
    pub fn finish(&mut self) -> Vec<UpstreamEvent> {
        let mut out = Vec::new();
        let pending: Vec<u64> = self
            .slots
            .iter()
            .filter(|(_, s)| !s.started)
            .map(|(i, _)| *i)
            .collect();
        for index in pending {
            out.extend(self.activate(index));
        }
        out.extend(self.close_active());
        out
    }

    pub fn saw_any(&self) -> bool {
        self.saw_any
    }

    /// 讓 `index` 成為開著的區塊：先關掉別的，需要時發 `ToolUseStart`。
    fn activate(&mut self, index: u64) -> Vec<UpstreamEvent> {
        if self.active == Some(index) {
            return Vec::new();
        }
        let mut out = self.close_active();
        let Some(slot) = self.slots.get_mut(&index) else {
            return out;
        };
        if !slot.started {
            slot.started = true;
            out.push(UpstreamEvent::ToolUseStart {
                id: slot.id.clone(),
                name: slot.name.clone(),
            });
        }
        self.active = Some(index);
        out
    }

    fn close_active(&mut self) -> Vec<UpstreamEvent> {
        if self.active.take().is_some() {
            vec![UpstreamEvent::ToolUseEnd]
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn acc() -> ToolCallAccumulator {
        ToolCallAccumulator::default()
    }

    #[test]
    fn name_and_args_in_one_chunk() {
        let mut a = acc();
        let ev = a.push(&[json!({
            "index": 0, "id": "call_1",
            "function": {"name": "Read", "arguments": "{\"p\":1}"}
        })]);
        assert_eq!(
            ev,
            vec![
                UpstreamEvent::ToolUseStart { id: "call_1".into(), name: "Read".into() },
                UpstreamEvent::ToolInputDelta("{\"p\":1}".into()),
            ]
        );
    }

    #[test]
    fn start_is_deferred_until_the_name_arrives() {
        // 有些 server 先送 id、名稱下一片才到。此時不能發 ToolUseStart，
        // 因為 Anthropic 的 content_block_start 就要帶 name。
        let mut a = acc();
        assert_eq!(a.push(&[json!({"index": 0, "id": "call_1"})]), vec![]);
        assert_eq!(
            a.push(&[json!({"index": 0, "function": {"name": "Read"}})]),
            vec![UpstreamEvent::ToolUseStart { id: "call_1".into(), name: "Read".into() }]
        );
    }

    #[test]
    fn name_split_across_chunks_is_concatenated() {
        let mut a = acc();
        a.push(&[json!({"index": 0, "id": "c1", "function": {"name": "Re"}})]);
        // 名稱補完之前不該發事件；補完後用完整名稱。
        let ev = a.push(&[json!({"index": 0, "function": {"arguments": "{}"}})]);
        assert_eq!(
            ev,
            vec![
                UpstreamEvent::ToolUseStart { id: "c1".into(), name: "Re".into() },
                UpstreamEvent::ToolInputDelta("{}".into()),
            ],
            "arguments 一到就代表名稱已完結，此時才 flush"
        );
    }

    #[test]
    fn argument_fragments_are_forwarded_verbatim() {
        let mut a = acc();
        a.push(&[json!({"index": 0, "id": "c1", "function": {"name": "N", "arguments": "{\"a\""}})]);
        let ev = a.push(&[json!({"index": 0, "function": {"arguments": ":1}"}})]);
        assert_eq!(ev, vec![UpstreamEvent::ToolInputDelta(":1}".into())]);
    }

    #[test]
    fn empty_argument_fragment_emits_nothing() {
        let mut a = acc();
        a.push(&[json!({"index": 0, "id": "c1", "function": {"name": "N", "arguments": ""}})]);
        assert_eq!(a.push(&[json!({"index": 0, "function": {"arguments": ""}})]), vec![]);
    }

    #[test]
    fn switching_index_closes_the_previous_tool() {
        let mut a = acc();
        a.push(&[json!({"index": 0, "id": "c1", "function": {"name": "A", "arguments": "{}"}})]);
        let ev = a.push(&[json!({"index": 1, "id": "c2", "function": {"name": "B", "arguments": "{}"}})]);
        assert_eq!(
            ev,
            vec![
                UpstreamEvent::ToolUseEnd,
                UpstreamEvent::ToolUseStart { id: "c2".into(), name: "B".into() },
                UpstreamEvent::ToolInputDelta("{}".into()),
            ]
        );
    }

    #[test]
    fn two_tools_in_one_chunk_are_sequenced() {
        let mut a = acc();
        let ev = a.push(&[
            json!({"index": 0, "id": "c1", "function": {"name": "A", "arguments": "{}"}}),
            json!({"index": 1, "id": "c2", "function": {"name": "B", "arguments": "{}"}}),
        ]);
        assert_eq!(
            ev,
            vec![
                UpstreamEvent::ToolUseStart { id: "c1".into(), name: "A".into() },
                UpstreamEvent::ToolInputDelta("{}".into()),
                UpstreamEvent::ToolUseEnd,
                UpstreamEvent::ToolUseStart { id: "c2".into(), name: "B".into() },
                UpstreamEvent::ToolInputDelta("{}".into()),
            ]
        );
    }

    #[test]
    fn finish_closes_the_open_tool() {
        let mut a = acc();
        a.push(&[json!({"index": 0, "id": "c1", "function": {"name": "A", "arguments": "{}"}})]);
        assert_eq!(a.finish(), vec![UpstreamEvent::ToolUseEnd]);
        assert_eq!(a.finish(), vec![], "重複呼叫不應再發事件");
    }

    #[test]
    fn finish_flushes_a_tool_that_never_got_arguments() {
        let mut a = acc();
        a.push(&[json!({"index": 0, "id": "c1", "function": {"name": "NoArgs"}})]);
        assert_eq!(
            a.finish(),
            vec![
                UpstreamEvent::ToolUseStart { id: "c1".into(), name: "NoArgs".into() },
                UpstreamEvent::ToolUseEnd,
            ],
            "無參數的工具呼叫不能被吞掉"
        );
    }

    #[test]
    fn missing_index_defaults_to_zero() {
        // 少數相容端點不送 index。
        let mut a = acc();
        let ev = a.push(&[json!({"id": "c1", "function": {"name": "A", "arguments": "{}"}})]);
        assert_eq!(ev.len(), 2);
    }

    #[test]
    fn reports_whether_any_tool_was_seen() {
        let mut a = acc();
        assert!(!a.saw_any());
        a.push(&[json!({"index": 0, "id": "c1", "function": {"name": "A"}})]);
        assert!(a.saw_any());
    }
}
