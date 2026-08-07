//! Anthropic Messages 請求 → OpenAI chat.completions 請求。

use serde_json::{json, Map, Value};

use crate::bridge::anthropic::request::{
    parse_content, system_text, ContentBlock, MessagesRequest,
};
use crate::bridge::tool_meta::ToolMetaCache;

pub fn build_body(req: &MessagesRequest, model: &str, tool_meta: &ToolMetaCache) -> Value {
    let mut messages: Vec<Value> = Vec::new();

    let system = system_text(req.system.as_ref());
    if !system.is_empty() {
        messages.push(json!({"role": "system", "content": system}));
    }
    for m in &req.messages {
        push_message(&mut messages, &m.role, &parse_content(&m.content), tool_meta);
    }

    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    body.insert("messages".into(), Value::Array(messages));
    body.insert("stream".into(), json!(true));
    // 沒有這個旗標，多數 OpenAI 相容端點的串流回應不會帶 usage，
    // Claude Code 的 token 計數就會一直是 0。
    body.insert("stream_options".into(), json!({"include_usage": true}));

    if let Some(v) = req.max_tokens {
        body.insert("max_tokens".into(), json!(v));
    }
    if let Some(v) = req.temperature {
        body.insert("temperature".into(), json!(v));
    }
    if let Some(v) = &req.stop_sequences {
        if !v.is_empty() {
            body.insert("stop".into(), json!(v));
        }
    }
    if let Some(t) = &req.thinking {
        if let Some(budget) = t.budget_tokens {
            body.insert("reasoning_effort".into(), json!(reasoning_effort(budget)));
        }
    }
    if let Some(tools) = &req.tools {
        if !tools.is_empty() {
            let defs: Vec<Value> = tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description.clone().unwrap_or_default(),
                            "parameters": t.input_schema,
                        }
                    })
                })
                .collect();
            body.insert("tools".into(), Value::Array(defs));
            body.insert("tool_choice".into(), map_tool_choice(req.tool_choice.as_ref()));
        }
    }

    Value::Object(body)
}

/// Anthropic 的 `tool_choice` 是物件，OpenAI 大多是字串（指定工具時才是物件）。
///
/// 不能寫死 `"auto"`：Claude Code 在某些子代理流程會送 `{"type":"none"}` 來
/// 禁止工具呼叫，寫死會讓模型在不該用工具的回合硬呼叫工具。
fn map_tool_choice(tc: Option<&Value>) -> Value {
    let Some(tc) = tc else { return json!("auto") };
    match tc.get("type").and_then(Value::as_str) {
        Some("any") => json!("required"),
        Some("none") => json!("none"),
        Some("tool") => match tc.get("name").and_then(Value::as_str) {
            Some(name) => json!({"type": "function", "function": {"name": name}}),
            None => json!("auto"),
        },
        _ => json!("auto"),
    }
}

/// Anthropic 的 thinking budget 是 token 數，OpenAI 是三段式。粗略對應即可
/// ——這個欄位只影響推理深度，不影響正確性。
fn reasoning_effort(budget: u32) -> &'static str {
    if budget < 4096 {
        "low"
    } else if budget < 16384 {
        "medium"
    } else {
        "high"
    }
}

/// 把一個 Anthropic turn 攤成一或多個 OpenAI 訊息。
///
/// 一個 turn 可能含多個 `tool_result`，OpenAI 要求每個結果各自一則 `tool`
/// 訊息；而 `tool` 訊息不能帶圖片，所以圖片會被提到後面新增的 user turn。
fn push_message(out: &mut Vec<Value>, role: &str, blocks: &[ContentBlock], tool_meta: &ToolMetaCache) {
    let mut text_parts: Vec<String> = Vec::new();
    let mut images: Vec<Value> = Vec::new();
    let mut tool_calls: Vec<Value> = Vec::new();
    let mut lifted_images: Vec<Value> = Vec::new();

    for b in blocks {
        match b {
            ContentBlock::Text(t) => text_parts.push(t.clone()),
            // thinking 區塊不回送給上游：它是模型自己的產出，重送只是浪費 token。
            ContentBlock::Thinking(_) => {}
            ContentBlock::Image { media_type, data } => images.push(image_part(media_type, data)),
            ContentBlock::ToolUse { id, name, input } => {
                let mut call = Map::new();
                call.insert("id".into(), json!(id));
                call.insert("type".into(), json!("function"));
                call.insert(
                    "function".into(),
                    json!({
                        "name": name,
                        // OpenAI 的 arguments 是 JSON 字串，不是物件。
                        "arguments": serde_json::to_string(input).unwrap_or_else(|_| "{}".into()),
                    }),
                );
                // Gemini 等供應商要求第二輪把工具呼叫附帶的 extra_content
                // （如 thought_signature）原樣回送，否則 400——見
                // `tool_meta.rs`。快取沒命中時完全不加這個欄位，不能塞
                // `null`，那對某些上游一樣算「缺少」。
                if let Some(extra) = tool_meta.get(id) {
                    call.insert("extra_content".into(), extra);
                }
                tool_calls.push(Value::Object(call));
            }
            ContentBlock::ToolResult { tool_use_id, content, .. } => {
                let mut result_text: Vec<String> = Vec::new();
                for inner in content {
                    match inner {
                        ContentBlock::Text(t) => result_text.push(t.clone()),
                        ContentBlock::Image { media_type, data } => {
                            lifted_images.push(image_part(media_type, data))
                        }
                        _ => {}
                    }
                }
                out.push(json!({
                    "role": "tool",
                    "tool_call_id": tool_use_id,
                    "content": result_text.join("\n"),
                }));
            }
        }
    }

    let has_own_content = !text_parts.is_empty() || !images.is_empty() || !tool_calls.is_empty();
    if has_own_content {
        let mut msg = Map::new();
        msg.insert("role".into(), json!(role));
        msg.insert("content".into(), content_value(&text_parts, &images));
        if !tool_calls.is_empty() {
            msg.insert("tool_calls".into(), Value::Array(tool_calls));
        }
        out.push(Value::Object(msg));
    }

    if !lifted_images.is_empty() {
        out.push(json!({"role": "user", "content": Value::Array(lifted_images)}));
    }
}

/// 沒有圖片時回純字串 —— 部分 OpenAI 相容 server 不接受 content 陣列。
fn content_value(text_parts: &[String], images: &[Value]) -> Value {
    if images.is_empty() {
        return json!(text_parts.join("\n"));
    }
    let mut parts: Vec<Value> = Vec::new();
    let joined = text_parts.join("\n");
    if !joined.is_empty() {
        parts.push(json!({"type": "text", "text": joined}));
    }
    parts.extend(images.iter().cloned());
    Value::Array(parts)
}

fn image_part(media_type: &str, data: &str) -> Value {
    json!({
        "type": "image_url",
        "image_url": {"url": format!("data:{media_type};base64,{data}")}
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn req(v: serde_json::Value) -> MessagesRequest {
        serde_json::from_value(v).unwrap()
    }

    /// 大多數測試不關心 extra_content 快取，給一個空的即可。
    fn empty_cache() -> ToolMetaCache {
        ToolMetaCache::new(512)
    }

    #[test]
    fn system_becomes_first_message() {
        let body = build_body(
            &req(json!({
                "model": "aiterm:sonnet", "system": "你是助手",
                "messages": [{"role": "user", "content": "hi"}]
            })),
            "qwen",
            &empty_cache(),
        );
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "你是助手");
        assert_eq!(msgs[1]["role"], "user");
    }

    #[test]
    fn empty_system_is_omitted() {
        let body = build_body(
            &req(json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]})),
            "qwen",
            &empty_cache(),
        );
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn model_and_stream_are_set() {
        let body = build_body(
            &req(json!({"model": "aiterm:opus", "messages": [{"role":"user","content":"x"}]})),
            "gpt-4o",
            &empty_cache(),
        );
        assert_eq!(body["model"], "gpt-4o", "要用映射後的模型名，不是哨兵字串");
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
    }

    #[test]
    fn image_block_becomes_data_uri() {
        let body = build_body(
            &req(json!({
                "model": "m",
                "messages": [{"role": "user", "content": [
                    {"type": "text", "text": "看"},
                    {"type": "image", "source": {"type":"base64","media_type":"image/png","data":"AAAA"}}
                ]}]
            })),
            "gpt-4o",
            &empty_cache(),
        );
        let parts = body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[1]["image_url"]["url"], "data:image/png;base64,AAAA");
    }

    #[test]
    fn plain_text_content_stays_a_string() {
        // 沒有圖片時不要包成陣列，某些相容 server 只吃字串。
        let body = build_body(
            &req(json!({"model":"m","messages":[{"role":"user","content":[{"type":"text","text":"hi"}]}]})),
            "m",
            &empty_cache(),
        );
        assert_eq!(body["messages"][0]["content"], "hi");
    }

    #[test]
    fn tool_use_becomes_assistant_tool_calls() {
        let body = build_body(
            &req(json!({
                "model": "m",
                "messages": [{"role": "assistant", "content": [
                    {"type": "tool_use", "id": "toolu_1", "name": "Read", "input": {"p": 1}}
                ]}]
            })),
            "m",
            &empty_cache(),
        );
        let msg = &body["messages"][0];
        assert_eq!(msg["role"], "assistant");
        assert_eq!(msg["tool_calls"][0]["id"], "toolu_1");
        assert_eq!(msg["tool_calls"][0]["function"]["name"], "Read");
        // arguments 必須是 JSON 字串，不是物件。
        assert_eq!(msg["tool_calls"][0]["function"]["arguments"], "{\"p\":1}");
    }

    #[test]
    fn tool_use_with_cached_extra_content_includes_it() {
        // 這是本次功能的核心：快取命中時，翻譯出的 tool_calls[0] 要帶著
        // 原封不動的 extra_content。
        let cache = empty_cache();
        let extra = json!({"google": {"thought_signature": "Eq8CCqwC…"}});
        cache.insert("toolu_1".into(), extra.clone());

        let body = build_body(
            &req(json!({
                "model": "m",
                "messages": [{"role": "assistant", "content": [
                    {"type": "tool_use", "id": "toolu_1", "name": "Read", "input": {"p": 1}}
                ]}]
            })),
            "m",
            &cache,
        );
        assert_eq!(body["messages"][0]["tool_calls"][0]["extra_content"], extra);
    }

    #[test]
    fn tool_use_without_cached_extra_content_omits_the_field() {
        // 快取沒命中時不能生出 "extra_content": null——那對某些上游一樣
        // 算「有這個欄位但缺內容」，跟完全不送不是同一件事。
        let body = build_body(
            &req(json!({
                "model": "m",
                "messages": [{"role": "assistant", "content": [
                    {"type": "tool_use", "id": "toolu_1", "name": "Read", "input": {"p": 1}}
                ]}]
            })),
            "m",
            &empty_cache(),
        );
        assert!(body["messages"][0]["tool_calls"][0].get("extra_content").is_none());
    }

    #[test]
    fn tool_result_becomes_a_tool_message() {
        let body = build_body(
            &req(json!({
                "model": "m",
                "messages": [{"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_1",
                     "content": [{"type": "text", "text": "檔案內容"}]}
                ]}]
            })),
            "m",
            &empty_cache(),
        );
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "tool");
        assert_eq!(msgs[0]["tool_call_id"], "toolu_1");
        assert_eq!(msgs[0]["content"], "檔案內容");
    }

    #[test]
    fn image_inside_tool_result_is_lifted_to_a_following_user_turn() {
        // OpenAI 的 tool 訊息不能帶圖片，圖片必須提到後面的 user turn。
        let body = build_body(
            &req(json!({
                "model": "m",
                "messages": [{"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t1", "content": [
                        {"type": "text", "text": "截圖如下"},
                        {"type": "image", "source": {"type":"base64","media_type":"image/png","data":"BBBB"}}
                    ]}
                ]}]
            })),
            "m",
            &empty_cache(),
        );
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2, "應該多出一個 user turn 承載圖片");
        assert_eq!(msgs[0]["role"], "tool");
        assert_eq!(msgs[0]["content"], "截圖如下");
        assert_eq!(msgs[1]["role"], "user");
        assert_eq!(msgs[1]["content"][0]["image_url"]["url"], "data:image/png;base64,BBBB");
    }

    #[test]
    fn multiple_tool_results_in_one_turn_become_separate_messages() {
        let body = build_body(
            &req(json!({
                "model": "m",
                "messages": [{"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t1", "content": "a"},
                    {"type": "tool_result", "tool_use_id": "t2", "content": "b"}
                ]}]
            })),
            "m",
            &empty_cache(),
        );
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["tool_call_id"], "t1");
        assert_eq!(msgs[1]["tool_call_id"], "t2");
    }

    #[test]
    fn tools_are_translated_to_function_defs() {
        let body = build_body(
            &req(json!({
                "model": "m",
                "messages": [{"role":"user","content":"x"}],
                "tools": [{"name": "Read", "description": "讀檔",
                           "input_schema": {"type":"object","properties":{}}}]
            })),
            "m",
            &empty_cache(),
        );
        let t = &body["tools"][0];
        assert_eq!(t["type"], "function");
        assert_eq!(t["function"]["name"], "Read");
        assert_eq!(t["function"]["description"], "讀檔");
        assert_eq!(t["function"]["parameters"]["type"], "object");
        assert_eq!(body["tool_choice"], "auto", "沒指定時預設 auto");
    }

    #[test]
    fn tool_choice_is_translated_not_hardcoded() {
        let mk = |tc: serde_json::Value| {
            build_body(
                &req(json!({
                    "model": "m", "messages": [{"role":"user","content":"x"}],
                    "tools": [{"name":"Read","input_schema":{}}],
                    "tool_choice": tc
                })),
                "m",
                &empty_cache(),
            )["tool_choice"]
                .clone()
        };
        assert_eq!(mk(json!({"type": "auto"})), json!("auto"));
        assert_eq!(mk(json!({"type": "any"})), json!("required"));
        assert_eq!(mk(json!({"type": "none"})), json!("none"));
        assert_eq!(
            mk(json!({"type": "tool", "name": "Read"})),
            json!({"type": "function", "function": {"name": "Read"}})
        );
    }

    #[test]
    fn no_tools_means_no_tools_field() {
        let body = build_body(
            &req(json!({"model":"m","messages":[{"role":"user","content":"x"}]})),
            "m",
            &empty_cache(),
        );
        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());
    }

    #[test]
    fn sampling_params_pass_through() {
        let body = build_body(
            &req(json!({
                "model":"m","messages":[{"role":"user","content":"x"}],
                "max_tokens": 4096, "temperature": 0.3, "stop_sequences": ["END"]
            })),
            "m",
            &empty_cache(),
        );
        assert_eq!(body["max_tokens"], 4096);
        assert_eq!(body["temperature"], 0.3);
        assert_eq!(body["stop"][0], "END");
    }

    #[test]
    fn thinking_budget_maps_to_reasoning_effort() {
        let mk = |budget: u32| {
            build_body(
                &req(json!({
                    "model":"m","messages":[{"role":"user","content":"x"}],
                    "thinking": {"type":"enabled","budget_tokens": budget}
                })),
                "m",
                &empty_cache(),
            )["reasoning_effort"]
                .clone()
        };
        assert_eq!(mk(1000), "low");
        assert_eq!(mk(8000), "medium");
        assert_eq!(mk(30000), "high");
    }
}
