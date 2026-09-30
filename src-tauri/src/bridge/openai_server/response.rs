//! 中立事件 → OpenAI `chat.completion` 輸出（串流與非串流各一份，吃同一種事件序列）。

use serde_json::{json, Value};

use crate::bridge::upstream::{StopReason, UpstreamEvent, Usage};

pub fn finish_reason(s: StopReason) -> &'static str {
    match s {
        StopReason::EndTurn | StopReason::StopSequence => "stop",
        StopReason::MaxTokens => "length",
        StopReason::ToolUse => "tool_calls",
    }
}

/// OpenAI 的錯誤 body：`{"error":{"message","type","param","code"}}`。
pub fn error_body(kind: &str, code: Option<&str>, message: &str) -> Value {
    json!({"error": {"message": message, "type": kind, "param": null, "code": code}})
}

/// 串流已經開始（HTTP 200 已送出）後只能用一個 SSE frame 回報錯誤。
pub fn error_chunk(kind: &str, message: &str) -> String {
    format!("data: {}\n\n", error_body(kind, None, message))
}

pub const DONE_FRAME: &str = "data: [DONE]\n\n";

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn usage_json(u: Usage) -> Value {
    json!({
        "prompt_tokens": u.input_tokens,
        "completion_tokens": u.output_tokens,
        "total_tokens": u.input_tokens + u.output_tokens,
    })
}

pub struct ChunkEncoder {
    id: String,
    model: String,
    created: u64,
    include_usage: bool,
    role_sent: bool,
    tool_count: usize,
    done: bool,
}

impl ChunkEncoder {
    pub fn new(id: String, model: String, include_usage: bool) -> Self {
        Self { id, model, created: now_secs(), include_usage, role_sent: false, tool_count: 0, done: false }
    }

    fn frame(&self, choices: Value, usage: Option<Value>) -> String {
        let mut v = json!({
            "id": self.id,
            "object": "chat.completion.chunk",
            "created": self.created,
            "model": self.model,
            "choices": choices,
        });
        if let Some(u) = usage {
            v["usage"] = u;
        }
        format!("data: {v}\n\n")
    }

    fn delta_frame(&self, delta: Value, finish: Option<&str>) -> String {
        self.frame(json!([{"index": 0, "delta": delta, "finish_reason": finish}]), None)
    }

    fn ensure_role(&mut self, out: &mut Vec<String>) {
        if !self.role_sent {
            self.role_sent = true;
            out.push(self.delta_frame(json!({"role": "assistant", "content": ""}), None));
        }
    }

    /// 每個元素都是完整的 `data: {...}\n\n`。
    pub fn push(&mut self, ev: UpstreamEvent) -> Vec<String> {
        if self.done {
            return Vec::new();
        }
        let mut out = Vec::new();
        match ev {
            UpstreamEvent::TextDelta(t) => {
                self.ensure_role(&mut out);
                out.push(self.delta_frame(json!({"content": t}), None));
            }
            // OpenAI 標準沒有對應欄位；塞進 content 會污染客戶端看到的答案。
            UpstreamEvent::ThinkingDelta(_) | UpstreamEvent::ToolUseEnd => {}
            UpstreamEvent::ToolUseStart { id, name } => {
                self.ensure_role(&mut out);
                let index = self.tool_count;
                self.tool_count += 1;
                out.push(self.delta_frame(
                    json!({"tool_calls": [{
                        "index": index, "id": id, "type": "function",
                        "function": {"name": name, "arguments": ""}
                    }]}),
                    None,
                ));
            }
            UpstreamEvent::ToolInputDelta(s) => {
                if self.tool_count > 0 {
                    out.push(self.delta_frame(
                        json!({"tool_calls": [{
                            "index": self.tool_count - 1,
                            "function": {"arguments": s}
                        }]}),
                        None,
                    ));
                }
            }
            UpstreamEvent::Done { stop_reason, usage } => {
                self.ensure_role(&mut out);
                out.push(self.delta_frame(json!({}), Some(finish_reason(stop_reason))));
                if self.include_usage {
                    out.push(self.frame(json!([]), Some(usage_json(usage))));
                }
                out.push(DONE_FRAME.to_string());
                self.done = true;
            }
        }
        out
    }

    /// 上游沒送 `Done` 就結束時補收尾，否則客戶端會一直等 `[DONE]`。
    /// 已收過 `Done` 時回空，可安全無條件呼叫。
    pub fn finish(&mut self) -> Vec<String> {
        self.push(UpstreamEvent::Done { stop_reason: StopReason::EndTurn, usage: Usage::default() })
    }
}

pub struct CompletionAggregator {
    id: String,
    model: String,
    text: String,
    /// (id, name, arguments)
    tools: Vec<(String, String, String)>,
    stop: StopReason,
    usage: Usage,
}

impl CompletionAggregator {
    pub fn new(id: String, model: String) -> Self {
        Self { id, model, text: String::new(), tools: Vec::new(), stop: StopReason::EndTurn, usage: Usage::default() }
    }

    pub fn push(&mut self, ev: UpstreamEvent) {
        match ev {
            UpstreamEvent::TextDelta(t) => self.text.push_str(&t),
            UpstreamEvent::ThinkingDelta(_) | UpstreamEvent::ToolUseEnd => {}
            UpstreamEvent::ToolUseStart { id, name } => self.tools.push((id, name, String::new())),
            UpstreamEvent::ToolInputDelta(s) => {
                if let Some(t) = self.tools.last_mut() {
                    t.2.push_str(&s);
                }
            }
            UpstreamEvent::Done { stop_reason, usage } => {
                self.stop = stop_reason;
                self.usage = usage;
            }
        }
    }

    pub fn finish(self) -> Value {
        let mut message = json!({
            "role": "assistant",
            "content": if self.text.is_empty() && !self.tools.is_empty() { Value::Null } else { json!(self.text) },
        });
        if !self.tools.is_empty() {
            message["tool_calls"] = self
                .tools
                .iter()
                .map(|(id, name, args)| {
                    // 工具沒有參數時上游可能一個 delta 都沒送，OpenAI 客戶端會拿
                    // arguments 去 JSON.parse，空字串會炸。
                    let args = if args.is_empty() { "{}" } else { args.as_str() };
                    json!({"id": id, "type": "function", "function": {"name": name, "arguments": args}})
                })
                .collect();
        }
        json!({
            "id": self.id,
            "object": "chat.completion",
            "created": now_secs(),
            "model": self.model,
            "choices": [{"index": 0, "message": message, "finish_reason": finish_reason(self.stop)}],
            "usage": usage_json(self.usage),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(include_usage: bool) -> ChunkEncoder {
        ChunkEncoder::new("chatcmpl-1".into(), "gpt-4o".into(), include_usage)
    }

    /// 把 `data: {...}\n\n` 解回 JSON；`[DONE]` 以字串表示。
    fn parse(frames: &[String]) -> Vec<Value> {
        frames
            .iter()
            .map(|f| {
                let p = f.strip_prefix("data: ").unwrap().trim_end();
                if p == "[DONE]" { json!("[DONE]") } else { serde_json::from_str(p).unwrap() }
            })
            .collect()
    }

    fn done(reason: StopReason) -> UpstreamEvent {
        UpstreamEvent::Done { stop_reason: reason, usage: Usage { input_tokens: 3, output_tokens: 4 } }
    }

    #[test]
    fn text_stream_frame_sequence() {
        let mut e = enc(false);
        let mut frames = e.push(UpstreamEvent::TextDelta("你".into()));
        frames.extend(e.push(UpstreamEvent::TextDelta("好".into())));
        frames.extend(e.push(done(StopReason::EndTurn)));
        frames.extend(e.finish());
        let v = parse(&frames);
        assert_eq!(v[0]["choices"][0]["delta"], json!({"role": "assistant", "content": ""}));
        assert_eq!(v[1]["choices"][0]["delta"], json!({"content": "你"}));
        assert_eq!(v[2]["choices"][0]["delta"], json!({"content": "好"}));
        assert_eq!(v[3]["choices"][0]["finish_reason"], "stop");
        assert_eq!(v[4], json!("[DONE]"));
        assert_eq!(v.len(), 5, "finish() 在已收 Done 後不可再送任何東西：{v:?}");
        assert_eq!(v[0]["object"], "chat.completion.chunk");
        assert_eq!(v[0]["model"], "gpt-4o");
    }

    #[test]
    fn two_consecutive_tool_calls_get_distinct_indexes() {
        let mut e = enc(false);
        let mut frames = e.push(UpstreamEvent::ToolUseStart { id: "c1".into(), name: "A".into() });
        frames.extend(e.push(UpstreamEvent::ToolInputDelta("{\"x\":".into())));
        frames.extend(e.push(UpstreamEvent::ToolInputDelta("1}".into())));
        frames.extend(e.push(UpstreamEvent::ToolUseEnd));
        frames.extend(e.push(UpstreamEvent::ToolUseStart { id: "c2".into(), name: "B".into() }));
        frames.extend(e.push(UpstreamEvent::ToolInputDelta("{}".into())));
        frames.extend(e.push(done(StopReason::ToolUse)));
        let v = parse(&frames);
        let call = |i: usize| v[i]["choices"][0]["delta"]["tool_calls"][0].clone();
        assert_eq!(call(1), json!({"index": 0, "id": "c1", "type": "function", "function": {"name": "A", "arguments": ""}}));
        assert_eq!(call(2)["function"]["arguments"], "{\"x\":");
        assert_eq!(call(3)["function"]["arguments"], "1}");
        assert_eq!(call(4)["index"], 1);
        assert_eq!(call(4)["id"], "c2");
        assert_eq!(call(5)["index"], 1);
        assert_eq!(v[6]["choices"][0]["finish_reason"], "tool_calls");
    }

    #[test]
    fn usage_chunk_only_when_requested() {
        let count = |include: bool| {
            let mut e = enc(include);
            let frames = e.push(done(StopReason::EndTurn));
            parse(&frames).into_iter().filter(|f| f.get("usage").is_some()).count()
        };
        assert_eq!(count(false), 0);
        assert_eq!(count(true), 1);

        let mut e = enc(true);
        let v = parse(&e.push(done(StopReason::EndTurn)));
        let u = v.iter().find(|f| f.get("usage").is_some()).unwrap();
        assert_eq!(u["choices"], json!([]));
        assert_eq!(u["usage"], json!({"prompt_tokens": 3, "completion_tokens": 4, "total_tokens": 7}));
    }

    #[test]
    fn finish_reason_mapping() {
        assert_eq!(finish_reason(StopReason::EndTurn), "stop");
        assert_eq!(finish_reason(StopReason::StopSequence), "stop");
        assert_eq!(finish_reason(StopReason::MaxTokens), "length");
        assert_eq!(finish_reason(StopReason::ToolUse), "tool_calls");
    }

    #[test]
    fn thinking_never_leaks_into_content() {
        let mut e = enc(false);
        assert!(e.push(UpstreamEvent::ThinkingDelta("秘密推理".into())).is_empty());
    }

    #[test]
    fn finish_without_done_still_terminates_the_stream() {
        let mut e = enc(false);
        e.push(UpstreamEvent::TextDelta("a".into()));
        let v = parse(&e.finish());
        assert_eq!(v.last().unwrap(), &json!("[DONE]"));
        assert!(e.finish().is_empty(), "只能收尾一次");
    }

    #[test]
    fn events_after_done_are_ignored() {
        let mut e = enc(false);
        e.push(done(StopReason::EndTurn));
        assert!(e.push(UpstreamEvent::TextDelta("late".into())).is_empty());
    }

    #[test]
    fn aggregator_text_response() {
        let mut a = CompletionAggregator::new("id".into(), "m".into());
        a.push(UpstreamEvent::TextDelta("你".into()));
        a.push(UpstreamEvent::TextDelta("好".into()));
        a.push(done(StopReason::MaxTokens));
        let v = a.finish();
        assert_eq!(v["object"], "chat.completion");
        assert_eq!(v["choices"][0]["message"]["content"], "你好");
        assert_eq!(v["choices"][0]["finish_reason"], "length");
        assert_eq!(v["usage"]["total_tokens"], 7);
        assert!(v["choices"][0]["message"].get("tool_calls").is_none());
    }

    #[test]
    fn aggregator_tool_calls_with_null_content() {
        let mut a = CompletionAggregator::new("id".into(), "m".into());
        a.push(UpstreamEvent::ToolUseStart { id: "c1".into(), name: "A".into() });
        a.push(UpstreamEvent::ToolInputDelta("{\"x\":".into()));
        a.push(UpstreamEvent::ToolInputDelta("1}".into()));
        a.push(UpstreamEvent::ToolUseEnd);
        a.push(UpstreamEvent::ToolUseStart { id: "c2".into(), name: "B".into() });
        a.push(done(StopReason::ToolUse));
        let v = a.finish();
        let m = &v["choices"][0]["message"];
        assert!(m["content"].is_null());
        assert_eq!(m["tool_calls"][0]["function"]["arguments"], "{\"x\":1}");
        assert_eq!(m["tool_calls"][1]["function"]["name"], "B");
        assert_eq!(m["tool_calls"][1]["function"]["arguments"], "{}", "無參數的工具要補 {{}}");
        assert_eq!(v["choices"][0]["finish_reason"], "tool_calls");
    }

    #[test]
    fn aggregator_mixed_text_and_tools_keeps_text() {
        let mut a = CompletionAggregator::new("id".into(), "m".into());
        a.push(UpstreamEvent::TextDelta("先讀檔".into()));
        a.push(UpstreamEvent::ToolUseStart { id: "c1".into(), name: "Read".into() });
        a.push(done(StopReason::ToolUse));
        assert_eq!(a.finish()["choices"][0]["message"]["content"], "先讀檔");
    }

    #[test]
    fn error_body_shape() {
        assert_eq!(
            error_body("invalid_request_error", Some("model_not_found"), "x"),
            json!({"error": {"message": "x", "type": "invalid_request_error", "param": null, "code": "model_not_found"}})
        );
        assert!(error_chunk("api_error", "boom").starts_with("data: {\"error\""));
    }
}
