//! Anthropic Messages 請求 → Gemini 原生（Antigravity）請求。
//!
//! 形狀全部來自實測（`tests/antigravity_probe.rs`），不是從 Gemini 公開文件
//! 推導的 —— `cloudcode-pa.googleapis.com/v1internal:streamGenerateContent`
//! 是逆向的無文件私有端點。

use std::collections::HashMap;

use serde_json::{json, Map, Value};

use crate::ai::antigravity::wrap_envelope;
use crate::bridge::anthropic::request::{parse_content, system_text, ContentBlock, MessagesRequest};
use crate::bridge::tool_meta::ToolMetaCache;

/// `ai/antigravity.rs` 的 `build_request_body` 把它寫死成 16384；橋接沒收到
/// `max_tokens` 時沿用同一個預設，收到時原樣透傳 Claude Code 送來的值。
const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 16384;

pub fn build_body(
    req: &MessagesRequest,
    model: &str,
    project_id: &str,
    tool_meta: &ToolMetaCache,
) -> Value {
    // functionResponse 需要 name，但 Anthropic 的 tool_result 只帶
    // tool_use_id —— 先掃一遍歷史建立 id → name 對照，翻譯 tool_result 時查表。
    let mut call_names: HashMap<String, String> = HashMap::new();
    for m in &req.messages {
        for block in parse_content(&m.content) {
            if let ContentBlock::ToolUse { id, name, .. } = block {
                call_names.insert(id, name);
            }
        }
    }

    let contents: Vec<Value> = req
        .messages
        .iter()
        .filter_map(|m| {
            let role = if m.role == "assistant" { "model" } else { "user" };
            let parts = build_parts(&parse_content(&m.content), tool_meta, &call_names);
            // 一個 turn 的區塊全被丟棄（例如只有一個沒帶文字的 thinking
            // 區塊）就不送空 parts 的 turn——那不是合法的 Gemini turn。
            if parts.is_empty() {
                return None;
            }
            Some(json!({ "role": role, "parts": parts }))
        })
        .collect();

    let mut request = Map::new();
    request.insert("contents".into(), Value::Array(contents));
    request.insert(
        "systemInstruction".into(),
        json!({ "parts": [{ "text": system_text(req.system.as_ref()) }] }),
    );
    request.insert(
        "generationConfig".into(),
        json!({
            "topK": 40,
            "topP": 1.0,
            "maxOutputTokens": req.max_tokens.unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS),
        }),
    );

    if let Some(tools) = &req.tools {
        if !tools.is_empty() {
            let decls: Vec<Value> = tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description.clone().unwrap_or_default(),
                        "parameters": t.input_schema.clone(),
                    })
                })
                .collect();
            request.insert("tools".into(), json!([{ "functionDeclarations": decls }]));
        }
    }

    wrap_envelope(project_id, model, Value::Object(request))
}

/// 把單一 turn 的 content block 翻成 Gemini 的 `parts`。
fn build_parts(
    blocks: &[ContentBlock],
    tool_meta: &ToolMetaCache,
    call_names: &HashMap<String, String>,
) -> Vec<Value> {
    let mut parts = Vec::new();
    let mut text_parts: Vec<String> = Vec::new();

    for b in blocks {
        match b {
            ContentBlock::Text(t) => text_parts.push(t.clone()),
            // 兩輪探勘都沒觀察到可讀的 thought part，我們也產生不出合法的
            // thoughtSignature，送回去只是雜訊（見本模組文件註解）。
            ContentBlock::Thinking(_) => {}
            // 圖片的 inlineData 兩輪探勘都沒驗證過，先丟棄而不是送一個可能
            // 被拒的欄位。
            ContentBlock::Image { .. } => {}
            ContentBlock::ToolUse { id, name, input } => {
                let mut part = Map::new();
                // 伺服器給了簽章的那個 part 才原樣回送，沒有的直接不帶
                // ——不要幫沒有的合成一個，也不要塞 null（實測：三個並行
                // 呼叫只有第 1 個帶 thoughtSignature，其餘欄位根本不存在）。
                if let Some(sig) = tool_meta.get(id) {
                    part.insert("thoughtSignature".into(), sig);
                }
                part.insert(
                    "functionCall".into(),
                    json!({ "name": name, "args": input, "id": id }),
                );
                parts.push(Value::Object(part));
            }
            ContentBlock::ToolResult { tool_use_id, content, .. } => {
                let name = call_names.get(tool_use_id).cloned().unwrap_or_else(|| {
                    log::warn!(
                        "Antigravity 橋接：tool_result 的 tool_use_id「{tool_use_id}」在請求歷史裡查不到對應的工具名稱，改用空字串"
                    );
                    String::new()
                });
                let text: Vec<String> = content
                    .iter()
                    .filter_map(|inner| match inner {
                        ContentBlock::Text(t) => Some(t.clone()),
                        _ => None,
                    })
                    .collect();
                parts.push(json!({
                    "functionResponse": { "name": name, "response": { "result": text.join("\n") } }
                }));
            }
        }
    }

    if !text_parts.is_empty() {
        parts.push(json!({ "text": text_parts.join("\n") }));
    }

    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn req(v: serde_json::Value) -> MessagesRequest {
        serde_json::from_value(v).unwrap()
    }

    fn empty_cache() -> ToolMetaCache {
        ToolMetaCache::new(512)
    }

    /// 取出信封裡的 `request` 物件，測試大多只關心它。
    fn inner(v: &serde_json::Value) -> &serde_json::Value {
        &v["request"]
    }

    #[test]
    fn envelope_carries_project_and_model() {
        let body = build_body(
            &req(json!({"model":"aiterm:sonnet","messages":[{"role":"user","content":"hi"}]})),
            "gemini-2.5-flash",
            "proj-1",
            &empty_cache(),
        );
        assert_eq!(body["project"], "proj-1");
        assert_eq!(body["model"], "gemini-2.5-flash");
        assert_eq!(body["userAgent"], "antigravity");
        assert_eq!(body["requestType"], "agent");
        assert!(body["requestId"].as_str().unwrap().starts_with("agent/"));
    }

    #[test]
    fn system_becomes_system_instruction() {
        let body = build_body(
            &req(json!({"model":"m","system":"你是助手","messages":[{"role":"user","content":"hi"}]})),
            "m", "p", &empty_cache(),
        );
        assert_eq!(inner(&body)["systemInstruction"]["parts"][0]["text"], "你是助手");
    }

    #[test]
    fn user_and_assistant_roles_map_to_user_and_model() {
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"user","content":"a"},
                {"role":"assistant","content":"b"}
            ]})),
            "m", "p", &empty_cache(),
        );
        let c = inner(&body)["contents"].as_array().unwrap();
        assert_eq!(c[0]["role"], "user");
        assert_eq!(c[0]["parts"][0]["text"], "a");
        assert_eq!(c[1]["role"], "model");
    }

    #[test]
    fn max_tokens_is_passed_through() {
        // ai/antigravity.rs 把 maxOutputTokens 寫死成 16384；橋接要透傳
        // Claude Code 送來的值。
        let body = build_body(
            &req(json!({"model":"m","max_tokens":4096,"messages":[{"role":"user","content":"x"}]})),
            "m", "p", &empty_cache(),
        );
        assert_eq!(inner(&body)["generationConfig"]["maxOutputTokens"], 4096);
    }

    #[test]
    fn tools_become_function_declarations() {
        let body = build_body(
            &req(json!({
                "model":"m","messages":[{"role":"user","content":"x"}],
                "tools":[{"name":"Read","description":"讀檔",
                          "input_schema":{"type":"object","properties":{}}}]
            })),
            "m", "p", &empty_cache(),
        );
        let d = &inner(&body)["tools"][0]["functionDeclarations"][0];
        assert_eq!(d["name"], "Read");
        assert_eq!(d["description"], "讀檔");
        assert_eq!(d["parameters"]["type"], "object");
    }

    #[test]
    fn no_tools_means_no_tools_field() {
        let body = build_body(
            &req(json!({"model":"m","messages":[{"role":"user","content":"x"}]})),
            "m", "p", &empty_cache(),
        );
        assert!(inner(&body).get("tools").is_none());
    }

    #[test]
    fn tool_use_becomes_a_function_call_part() {
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"assistant","content":[
                    {"type":"tool_use","id":"MrBcwQsI","name":"Read","input":{"p":1}}
                ]}
            ]})),
            "m", "p", &empty_cache(),
        );
        let part = &inner(&body)["contents"][0]["parts"][0];
        assert_eq!(part["functionCall"]["name"], "Read");
        assert_eq!(part["functionCall"]["id"], "MrBcwQsI");
        // args 是物件，不是 JSON 字串（跟 OpenAI 不同）。
        assert_eq!(part["functionCall"]["args"]["p"], 1);
    }

    #[test]
    fn cached_thought_signature_is_reattached() {
        // 實測：拿掉伺服器給的簽章 → 400。
        let cache = empty_cache();
        cache.insert("MrBcwQsI".into(), json!("EjQKMgER-sig"));
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"assistant","content":[
                    {"type":"tool_use","id":"MrBcwQsI","name":"Read","input":{}}
                ]}
            ]})),
            "m", "p", &cache,
        );
        let part = &inner(&body)["contents"][0]["parts"][0];
        assert_eq!(part["thoughtSignature"], "EjQKMgER-sig");
    }

    #[test]
    fn missing_signature_is_simply_omitted() {
        // 實測：三個並行呼叫只有第 1 個帶簽章，其餘欄位根本不存在。
        // 沒有的就不要帶，也不要塞 null。
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"assistant","content":[
                    {"type":"tool_use","id":"NoSig","name":"Read","input":{}}
                ]}
            ]})),
            "m", "p", &empty_cache(),
        );
        let part = &inner(&body)["contents"][0]["parts"][0];
        assert!(part.get("thoughtSignature").is_none());
    }

    #[test]
    fn tool_result_becomes_function_response_with_the_name_from_history() {
        // functionResponse 需要 name，但 Anthropic 的 tool_result 只有
        // tool_use_id —— 要從同一個請求的歷史裡查出名稱。
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"assistant","content":[
                    {"type":"tool_use","id":"c1","name":"get_weather","input":{}}
                ]},
                {"role":"user","content":[
                    {"type":"tool_result","tool_use_id":"c1",
                     "content":[{"type":"text","text":"31°C"}]}
                ]}
            ]})),
            "m", "p", &empty_cache(),
        );
        let c = inner(&body)["contents"].as_array().unwrap();
        let fr = &c[1]["parts"][0]["functionResponse"];
        assert_eq!(fr["name"], "get_weather");
        assert_eq!(c[1]["role"], "user");
    }

    #[test]
    fn tool_result_with_unknown_id_still_produces_a_part() {
        // 理論上不該發生，但不能讓整個請求失敗。
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"user","content":[
                    {"type":"tool_result","tool_use_id":"ghost","content":"x"}
                ]}
            ]})),
            "m", "p", &empty_cache(),
        );
        let fr = &inner(&body)["contents"][0]["parts"][0]["functionResponse"];
        assert_eq!(fr["name"], "");
    }

    #[test]
    fn two_parallel_tool_calls_keep_their_own_ids() {
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"assistant","content":[
                    {"type":"tool_use","id":"c1","name":"A","input":{}},
                    {"type":"tool_use","id":"c2","name":"B","input":{}}
                ]}
            ]})),
            "m", "p", &empty_cache(),
        );
        let parts = inner(&body)["contents"][0]["parts"].as_array().unwrap();
        assert_eq!(parts[0]["functionCall"]["id"], "c1");
        assert_eq!(parts[1]["functionCall"]["id"], "c2");
    }

    #[test]
    fn thinking_blocks_are_dropped() {
        // 兩輪探勘都沒觀察到可讀的 thought part，我們也產生不出合法的
        // thoughtSignature，送回去只是雜訊。
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"assistant","content":[
                    {"type":"thinking","thinking":"嗯"},
                    {"type":"text","text":"答案"}
                ]}
            ]})),
            "m", "p", &empty_cache(),
        );
        let parts = inner(&body)["contents"][0]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0]["text"], "答案");
    }
}
