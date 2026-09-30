//! OpenAI `chat.completions` 請求 → Anthropic 形狀的 `MessagesRequest`。
//!
//! 現有所有上游 adapter 的輸入型別都是 `MessagesRequest`（Anthropic 形狀），
//! 所以入口只需要把 OpenAI 翻成它，不用另造中間型別。同時回傳 `raw`
//! （Anthropic 形狀的 JSON）——Anthropic 家族上游的 `send_raw` 吃的是原始 JSON。
//!
//! 錯誤（`Err(String)`）是直接回給客戶端的 400 訊息，所以寫成人看得懂的句子。

use serde_json::{json, Map, Value};

use crate::bridge::anthropic::request::MessagesRequest;

/// Anthropic 的 `max_tokens` 必填，OpenAI 客戶端常常不送。
const DEFAULT_MAX_TOKENS: u64 = 4096;

pub struct Translated {
    pub request: MessagesRequest,
    pub raw: Value,
    pub stream: bool,
    pub include_usage: bool,
}

pub fn translate(body: &Value) -> Result<Translated, String> {
    let obj = body.as_object().ok_or("請求必須是 JSON 物件。")?;
    let model = obj
        .get("model")
        .and_then(Value::as_str)
        .ok_or("缺少 model 欄位。")?;
    let msgs = obj
        .get("messages")
        .and_then(Value::as_array)
        .filter(|m| !m.is_empty())
        .ok_or("messages 不可為空。")?;

    if obj.get("n").and_then(Value::as_u64).is_some_and(|n| n > 1) {
        return Err("n > 1 不支援：AITerm 一次只產生一個回應。".into());
    }

    let mut system_parts: Vec<String> = Vec::new();
    let mut messages: Vec<Value> = Vec::new();
    // 只有「我們自己為 role:tool 建立的 user 訊息」才可以繼續併入 tool_result；
    // 使用者真正的 user 訊息不能被塞進額外的區塊。
    let mut last_is_tool_group = false;

    for m in msgs {
        let role = m.get("role").and_then(Value::as_str).unwrap_or_default();
        match role {
            "system" | "developer" => {
                system_parts.push(text_of(m.get("content")));
                last_is_tool_group = false;
            }
            "user" => {
                messages.push(json!({"role": "user", "content": user_content(m.get("content"))?}));
                last_is_tool_group = false;
            }
            "assistant" => {
                if let Some(msg) = assistant_message(m) {
                    messages.push(msg);
                }
                last_is_tool_group = false;
            }
            "tool" => {
                let block = json!({
                    "type": "tool_result",
                    "tool_use_id": m.get("tool_call_id").and_then(Value::as_str).unwrap_or_default(),
                    "content": text_of(m.get("content")),
                });
                if last_is_tool_group {
                    if let Some(arr) = messages
                        .last_mut()
                        .and_then(|l| l.get_mut("content"))
                        .and_then(Value::as_array_mut)
                    {
                        arr.push(block);
                        continue;
                    }
                }
                messages.push(json!({"role": "user", "content": [block]}));
                last_is_tool_group = true;
            }
            other => return Err(format!("不支援的 role「{other}」。")),
        }
    }

    let mut raw = Map::new();
    raw.insert("model".into(), json!(model));
    raw.insert("messages".into(), Value::Array(messages));
    let system = system_parts.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n\n");
    if !system.is_empty() {
        raw.insert("system".into(), json!(system));
    }
    let max_tokens = obj
        .get("max_completion_tokens")
        .or_else(|| obj.get("max_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_MAX_TOKENS);
    raw.insert("max_tokens".into(), json!(max_tokens));
    if let Some(t) = obj.get("temperature").and_then(Value::as_f64) {
        raw.insert("temperature".into(), json!(t));
    }
    match obj.get("stop") {
        Some(Value::String(s)) => {
            raw.insert("stop_sequences".into(), json!([s]));
        }
        Some(Value::Array(a)) if !a.is_empty() => {
            raw.insert("stop_sequences".into(), Value::Array(a.clone()));
        }
        _ => {}
    }

    insert_tools(&mut raw, obj);

    let stream = obj.get("stream").and_then(Value::as_bool).unwrap_or(false);
    let include_usage = obj
        .get("stream_options")
        .and_then(|o| o.get("include_usage"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    raw.insert("stream".into(), json!(stream));

    let raw = Value::Object(raw);
    let request: MessagesRequest =
        serde_json::from_value(raw.clone()).map_err(|e| format!("無法解析請求：{e}"))?;
    Ok(Translated { request, raw, stream, include_usage })
}

/// `tools` 與 `tool_choice`。`tool_choice:"none"` 的意思是「這回合不准呼叫工具」，
/// Anthropic 沒有對應值，最忠實的作法是乾脆不送 tools。
fn insert_tools(raw: &mut Map<String, Value>, obj: &Map<String, Value>) {
    let choice = obj.get("tool_choice");
    if choice.and_then(Value::as_str) == Some("none") {
        return;
    }
    let tools: Vec<Value> = obj
        .get("tools")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|t| t.get("type").and_then(Value::as_str) == Some("function"))
                .filter_map(|t| {
                    let f = t.get("function")?;
                    let name = f.get("name")?.as_str()?;
                    let mut def = Map::new();
                    def.insert("name".into(), json!(name));
                    if let Some(d) = f.get("description").and_then(Value::as_str) {
                        def.insert("description".into(), json!(d));
                    }
                    def.insert(
                        "input_schema".into(),
                        f.get("parameters")
                            .cloned()
                            .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
                    );
                    Some(Value::Object(def))
                })
                .collect()
        })
        .unwrap_or_default();
    if tools.is_empty() {
        return;
    }
    raw.insert("tools".into(), Value::Array(tools));
    let mapped = match choice {
        Some(Value::String(s)) if s == "required" => Some(json!({"type": "any"})),
        Some(Value::String(s)) if s == "auto" => Some(json!({"type": "auto"})),
        Some(Value::Object(o)) => o
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(Value::as_str)
            .map(|n| json!({"type": "tool", "name": n})),
        _ => None,
    };
    if let Some(c) = mapped {
        raw.insert("tool_choice".into(), c);
    }
}

/// content 可能是字串、`[{"type":"text","text":..}]` 或 null，攤平成純文字。
fn text_of(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn user_content(v: Option<&Value>) -> Result<Value, String> {
    let Some(Value::Array(parts)) = v else {
        return Ok(json!(text_of(v)));
    };
    let mut blocks = Vec::new();
    for p in parts {
        match p.get("type").and_then(Value::as_str) {
            Some("text") => {
                blocks.push(json!({"type": "text", "text": p.get("text").and_then(Value::as_str).unwrap_or_default()}));
            }
            Some("image_url") => {
                let url = p
                    .get("image_url")
                    .and_then(|i| i.get("url"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                blocks.push(image_block(url)?);
            }
            // 未知的內容型別（input_audio 等）直接略過，與 Anthropic 入口對未知
            // block 的處理一致。
            _ => {}
        }
    }
    Ok(Value::Array(blocks))
}

fn image_block(url: &str) -> Result<Value, String> {
    let rest = url
        .strip_prefix("data:")
        .ok_or("image_url 只支援 data: URL（base64）。AITerm 不會替客戶端下載遠端圖片。")?;
    let (meta, data) = rest.split_once(',').ok_or("image_url 的 data: URL 格式錯誤。")?;
    let media_type = meta.strip_suffix(";base64").ok_or("image_url 的 data: URL 必須是 base64。")?;
    Ok(json!({
        "type": "image",
        "source": {"type": "base64", "media_type": media_type, "data": data},
    }))
}

fn assistant_message(m: &Value) -> Option<Value> {
    let text = text_of(m.get("content"));
    let calls = m.get("tool_calls").and_then(Value::as_array);
    let Some(calls) = calls.filter(|c| !c.is_empty()) else {
        // 空的 assistant 訊息會被 Anthropic 拒絕，直接略過。
        return (!text.is_empty()).then(|| json!({"role": "assistant", "content": text}));
    };
    let mut blocks = Vec::new();
    if !text.is_empty() {
        blocks.push(json!({"type": "text", "text": text}));
    }
    for c in calls {
        let f = c.get("function");
        let args = f.and_then(|f| f.get("arguments")).and_then(Value::as_str).unwrap_or("{}");
        blocks.push(json!({
            "type": "tool_use",
            "id": c.get("id").and_then(Value::as_str).unwrap_or_default(),
            "name": f.and_then(|f| f.get("name")).and_then(Value::as_str).unwrap_or_default(),
            // 客戶端歷史裡的 arguments 壞掉時退成空物件，總比整個請求失敗好。
            "input": serde_json::from_str::<Value>(args).unwrap_or_else(|_| json!({})),
        }));
    }
    Some(json!({"role": "assistant", "content": blocks}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(body: Value) -> Translated {
        translate(&body).unwrap()
    }

    #[test]
    fn plain_text_round_trip() {
        let r = t(json!({"model": "a", "messages": [{"role": "user", "content": "hi"}]}));
        assert_eq!(r.raw["messages"][0], json!({"role": "user", "content": "hi"}));
        assert_eq!(r.request.model, "a");
        assert!(!r.stream);
    }

    #[test]
    fn system_and_developer_messages_merge_into_system() {
        let r = t(json!({"model": "a", "messages": [
            {"role": "system", "content": "一"},
            {"role": "developer", "content": [{"type": "text", "text": "二"}]},
            {"role": "user", "content": "hi"}
        ]}));
        assert_eq!(r.raw["system"], "一\n\n二");
        assert_eq!(r.raw["messages"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn image_data_url_becomes_base64_source() {
        let r = t(json!({"model": "a", "messages": [{"role": "user", "content": [
            {"type": "text", "text": "看"},
            {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAA"}}
        ]}]}));
        assert_eq!(
            r.raw["messages"][0]["content"][1],
            json!({"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "AAA"}})
        );
    }

    #[test]
    fn remote_image_url_is_rejected() {
        let err = translate(&json!({"model": "a", "messages": [{"role": "user", "content": [
            {"type": "image_url", "image_url": {"url": "https://x/y.png"}}
        ]}]}))
        .err()
        .unwrap();
        assert!(err.contains("data:"), "{err}");
    }

    #[test]
    fn assistant_tool_calls_become_tool_use_blocks() {
        let r = t(json!({"model": "a", "messages": [
            {"role": "user", "content": "go"},
            {"role": "assistant", "content": null, "tool_calls": [
                {"id": "c1", "type": "function", "function": {"name": "Read", "arguments": "{\"p\":1}"}}
            ]}
        ]}));
        assert_eq!(
            r.raw["messages"][1]["content"][0],
            json!({"type": "tool_use", "id": "c1", "name": "Read", "input": {"p": 1}})
        );
    }

    #[test]
    fn broken_arguments_json_falls_back_to_empty_object() {
        let r = t(json!({"model": "a", "messages": [
            {"role": "assistant", "tool_calls": [
                {"id": "c1", "type": "function", "function": {"name": "R", "arguments": "{oops"}}
            ]}
        ]}));
        assert_eq!(r.raw["messages"][0]["content"][0]["input"], json!({}));
    }

    #[test]
    fn consecutive_tool_messages_merge_into_one_user_message() {
        let r = t(json!({"model": "a", "messages": [
            {"role": "user", "content": "go"},
            {"role": "assistant", "tool_calls": [
                {"id": "c1", "type": "function", "function": {"name": "A", "arguments": "{}"}},
                {"id": "c2", "type": "function", "function": {"name": "B", "arguments": "{}"}}
            ]},
            {"role": "tool", "tool_call_id": "c1", "content": "r1"},
            {"role": "tool", "tool_call_id": "c2", "content": "r2"}
        ]}));
        let msgs = r.raw["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3, "兩個 tool 訊息要併成一則 user：{msgs:?}");
        assert_eq!(msgs[2]["role"], "user");
        assert_eq!(msgs[2]["content"].as_array().unwrap().len(), 2);
        assert_eq!(msgs[2]["content"][1]["tool_use_id"], "c2");
    }

    #[test]
    fn a_real_user_message_after_tool_results_is_not_merged_into_them() {
        let r = t(json!({"model": "a", "messages": [
            {"role": "tool", "tool_call_id": "c1", "content": "r1"},
            {"role": "user", "content": "接著"},
            {"role": "tool", "tool_call_id": "c2", "content": "r2"}
        ]}));
        assert_eq!(r.raw["messages"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn tools_and_tool_choice_are_mapped() {
        let base = |tc: Value| {
            t(json!({"model": "a", "messages": [{"role": "user", "content": "x"}],
                "tools": [{"type": "function", "function": {
                    "name": "Read", "description": "d", "parameters": {"type": "object"}}}],
                "tool_choice": tc}))
            .raw
        };
        let auto = base(json!("auto"));
        assert_eq!(auto["tools"][0], json!({"name": "Read", "description": "d", "input_schema": {"type": "object"}}));
        assert_eq!(auto["tool_choice"], json!({"type": "auto"}));
        assert_eq!(base(json!("required"))["tool_choice"], json!({"type": "any"}));
        assert_eq!(
            base(json!({"type": "function", "function": {"name": "Read"}}))["tool_choice"],
            json!({"type": "tool", "name": "Read"})
        );
    }

    #[test]
    fn tool_choice_none_drops_tools_entirely() {
        let r = t(json!({"model": "a", "messages": [{"role": "user", "content": "x"}],
            "tools": [{"type": "function", "function": {"name": "Read"}}],
            "tool_choice": "none"}));
        assert!(r.raw.get("tools").is_none());
        assert!(r.raw.get("tool_choice").is_none());
    }

    #[test]
    fn tool_without_parameters_gets_an_empty_object_schema() {
        let r = t(json!({"model": "a", "messages": [{"role": "user", "content": "x"}],
            "tools": [{"type": "function", "function": {"name": "Ping"}}]}));
        assert_eq!(r.raw["tools"][0]["input_schema"], json!({"type": "object", "properties": {}}));
    }

    #[test]
    fn max_tokens_precedence_and_default() {
        let m = |extra: Value| {
            let mut b = json!({"model": "a", "messages": [{"role": "user", "content": "x"}]});
            b.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            t(b).raw["max_tokens"].clone()
        };
        assert_eq!(m(json!({})), 4096);
        assert_eq!(m(json!({"max_tokens": 10})), 10);
        assert_eq!(m(json!({"max_tokens": 10, "max_completion_tokens": 20})), 20);
    }

    #[test]
    fn stop_string_and_array_both_become_stop_sequences() {
        let s = |stop: Value| {
            t(json!({"model": "a", "messages": [{"role": "user", "content": "x"}], "stop": stop})).raw
        };
        assert_eq!(s(json!("END"))["stop_sequences"], json!(["END"]));
        assert_eq!(s(json!(["a", "b"]))["stop_sequences"], json!(["a", "b"]));
    }

    #[test]
    fn stream_flags_are_reported() {
        let r = t(json!({"model": "a", "messages": [{"role": "user", "content": "x"}],
            "stream": true, "stream_options": {"include_usage": true}}));
        assert!(r.stream);
        assert!(r.include_usage);
    }

    #[test]
    fn rejects_bad_requests() {
        assert!(translate(&json!({"model": "a", "messages": []})).is_err());
        assert!(translate(&json!({"model": "a"})).is_err());
        assert!(translate(&json!({"messages": [{"role": "user", "content": "x"}]})).is_err());
        let n = translate(&json!({"model": "a", "n": 2, "messages": [{"role": "user", "content": "x"}]}));
        assert!(n.err().unwrap().contains("n > 1"));
        let role = translate(&json!({"model": "a", "messages": [{"role": "wizard", "content": "x"}]}));
        assert!(role.err().unwrap().contains("wizard"));
    }
}
