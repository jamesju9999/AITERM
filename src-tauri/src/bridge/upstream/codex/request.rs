//! Anthropic Messages 請求 → Codex Responses API 請求。
//!
//! 形狀全部來自實測（`tests/codex_probe.rs`），不是從 Responses API 公開
//! 文件推導的 —— `chatgpt.com/backend-api/codex` 是逆向的無文件端點。

use serde_json::{json, Map, Value};

use crate::ai::codex::{content_type_for_role, map_input_role};
use crate::bridge::anthropic::request::{
    parse_content, system_text, ContentBlock, MessagesRequest,
};

pub fn build_body(req: &MessagesRequest, model: &str) -> Value {
    let mut input: Vec<Value> = Vec::new();
    for m in &req.messages {
        push_message(&mut input, &m.role, &parse_content(&m.content));
    }

    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    // 後端拒絕沒有 instructions 的請求（見 ai/codex.rs 的註解），
    // 所以即使是空字串也要送。
    body.insert("instructions".into(), json!(system_text(req.system.as_ref())));
    body.insert("input".into(), Value::Array(input));
    body.insert("stream".into(), json!(true));
    body.insert("store".into(), json!(false));

    if let Some(tools) = &req.tools {
        if !tools.is_empty() {
            // Responses API 的 tool 是扁平的：name 直接在物件上，
            // 沒有 chat.completions 那層巢狀的 function 物件。
            let defs: Vec<Value> = tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "name": t.name,
                        "description": t.description.clone().unwrap_or_default(),
                        "parameters": t.input_schema,
                    })
                })
                .collect();
            body.insert("tools".into(), Value::Array(defs));
        }
    }

    if let Some(t) = &req.thinking {
        // 實測：不設 summary 就一個 reasoning 事件都不會出現。
        body.insert(
            "reasoning".into(),
            json!({"effort": reasoning_effort(t.budget_tokens.unwrap_or(0)), "summary": "auto"}),
        );
    }

    Value::Object(body)
}

fn reasoning_effort(budget: u32) -> &'static str {
    if budget < 4096 {
        "low"
    } else if budget < 16384 {
        "medium"
    } else {
        "high"
    }
}

/// 把一個 Anthropic turn 攤成一或多個 Responses API 的 input item。
fn push_message(out: &mut Vec<Value>, role: &str, blocks: &[ContentBlock]) {
    let mut text_parts: Vec<String> = Vec::new();

    for b in blocks {
        match b {
            ContentBlock::Text(t) => text_parts.push(t.clone()),
            // 我們產生的 thinking 區塊沒有 Codex 的 encrypted_content，
            // 送回去只是雜訊。實測整個不回送也回 200。
            ContentBlock::Thinking(_) => {}
            // 圖片：Responses API 有 input_image，但探勘沒有驗證過這個端點
            // 接不接受，所以先丟棄而不是送一個可能被拒的欄位。
            // 見 spec 的「已知限制」。
            ContentBlock::Image { .. } => {}
            ContentBlock::ToolUse { id, name, input } => out.push(json!({
                "type": "function_call",
                // 往返用 call_id（實測：fc_ 開頭的 id 不需要送）。
                "call_id": id,
                "name": name,
                "arguments": serde_json::to_string(input).unwrap_or_else(|_| "{}".into()),
            })),
            ContentBlock::ToolResult { tool_use_id, content, .. } => {
                let text: Vec<String> = content
                    .iter()
                    .filter_map(|inner| match inner {
                        ContentBlock::Text(t) => Some(t.clone()),
                        _ => None,
                    })
                    .collect();
                out.push(json!({
                    "type": "function_call_output",
                    "call_id": tool_use_id,
                    "output": text.join("\n"),
                }));
            }
        }
    }

    if !text_parts.is_empty() {
        // 角色映射（system → developer）與 content type 判斷
        // （assistant → output_text，其餘 → input_text）都是同一份端點知識，
        // 共用 ai/codex.rs 的 map_input_role / content_type_for_role，
        // 不在這裡另外複製一份。
        let mapped_role = map_input_role(role);
        let content_type = content_type_for_role(mapped_role);
        out.push(json!({
            "type": "message",
            "role": mapped_role,
            "content": [{"type": content_type, "text": text_parts.join("\n")}],
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn req(v: serde_json::Value) -> MessagesRequest {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn system_becomes_top_level_instructions() {
        // Codex 後端必填 instructions（見 ai/codex.rs:81 的註解）。
        let body = build_body(
            &req(json!({
                "model": "aiterm:sonnet", "system": "你是助手",
                "messages": [{"role": "user", "content": "hi"}]
            })),
            "gpt-5.6",
        );
        assert_eq!(body["instructions"], "你是助手");
        assert_eq!(body["model"], "gpt-5.6");
        assert_eq!(body["stream"], true);
        assert_eq!(body["store"], false);
    }

    #[test]
    fn empty_system_still_sends_instructions() {
        // 後端拒絕沒有 instructions 的請求，所以空字串也要送。
        let body = build_body(
            &req(json!({"model": "m", "messages": [{"role":"user","content":"hi"}]})),
            "m",
        );
        assert!(body.get("instructions").is_some());
    }

    #[test]
    fn user_text_becomes_input_text_message() {
        let body = build_body(
            &req(json!({"model":"m","messages":[{"role":"user","content":"hi"}]})),
            "m",
        );
        let item = &body["input"][0];
        assert_eq!(item["type"], "message");
        assert_eq!(item["role"], "user");
        assert_eq!(item["content"][0]["type"], "input_text");
        assert_eq!(item["content"][0]["text"], "hi");
    }

    #[test]
    fn system_role_messages_are_remapped_to_developer() {
        // Codex 後端拒絕 system 角色（{"detail":"System messages are not
        // allowed"}）。Claude Code 確實會在 messages 裡送 system 角色，
        // 這是實測撞到的 400。
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"system","content":"你很簡潔"},
                {"role":"user","content":"hi"}
            ]})),
            "m",
        );
        assert_eq!(body["input"][0]["role"], "developer");
        assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(body["input"][1]["role"], "user");
    }

    #[test]
    fn assistant_text_uses_output_text() {
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"user","content":"a"},
                {"role":"assistant","content":"b"}
            ]})),
            "m",
        );
        assert_eq!(body["input"][1]["content"][0]["type"], "output_text");
    }

    #[test]
    fn tool_use_becomes_a_function_call_item() {
        // 實測：往返只需要 type/call_id/name/arguments，不需要 fc_ 開頭的 id。
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"assistant","content":[
                    {"type":"tool_use","id":"call_abc","name":"Read","input":{"p":1}}
                ]}
            ]})),
            "m",
        );
        let item = &body["input"][0];
        assert_eq!(item["type"], "function_call");
        assert_eq!(item["call_id"], "call_abc");
        assert_eq!(item["name"], "Read");
        // arguments 是 JSON 字串，不是物件。
        assert_eq!(item["arguments"], "{\"p\":1}");
        assert!(item.get("id").is_none(), "不該送 fc_ 開頭的 id");
    }

    #[test]
    fn tool_result_becomes_function_call_output() {
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"user","content":[
                    {"type":"tool_result","tool_use_id":"call_abc",
                     "content":[{"type":"text","text":"檔案內容"}]}
                ]}
            ]})),
            "m",
        );
        let item = &body["input"][0];
        assert_eq!(item["type"], "function_call_output");
        assert_eq!(item["call_id"], "call_abc");
        assert_eq!(item["output"], "檔案內容");
    }

    #[test]
    fn a_turn_with_two_tool_results_produces_two_items() {
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"user","content":[
                    {"type":"tool_result","tool_use_id":"c1","content":"a"},
                    {"type":"tool_result","tool_use_id":"c2","content":"b"}
                ]}
            ]})),
            "m",
        );
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 2);
        assert_eq!(input[0]["call_id"], "c1");
        assert_eq!(input[1]["call_id"], "c2");
    }

    #[test]
    fn tools_are_flat_not_nested() {
        // Responses API 的 tool 格式跟 chat.completions 不同：name 直接在
        // 物件上，沒有巢狀的 function 物件。
        let body = build_body(
            &req(json!({
                "model":"m","messages":[{"role":"user","content":"x"}],
                "tools":[{"name":"Read","description":"讀檔",
                          "input_schema":{"type":"object","properties":{}}}]
            })),
            "m",
        );
        let t = &body["tools"][0];
        assert_eq!(t["type"], "function");
        assert_eq!(t["name"], "Read");
        assert_eq!(t["description"], "讀檔");
        assert_eq!(t["parameters"]["type"], "object");
        assert!(t.get("function").is_none(), "不該有巢狀的 function 物件");
    }

    #[test]
    fn no_tools_means_no_tools_field() {
        let body = build_body(
            &req(json!({"model":"m","messages":[{"role":"user","content":"x"}]})),
            "m",
        );
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn thinking_request_enables_reasoning_summary() {
        // 實測：不設 summary 就完全沒有 reasoning 事件（第一輪探勘的結果）。
        let body = build_body(
            &req(json!({
                "model":"m","messages":[{"role":"user","content":"x"}],
                "thinking":{"type":"enabled","budget_tokens":8000}
            })),
            "m",
        );
        assert_eq!(body["reasoning"]["summary"], "auto");
        assert_eq!(body["reasoning"]["effort"], "medium");
    }

    #[test]
    fn no_thinking_means_no_reasoning_field() {
        let body = build_body(
            &req(json!({"model":"m","messages":[{"role":"user","content":"x"}]})),
            "m",
        );
        assert!(body.get("reasoning").is_none());
    }

    #[test]
    fn thinking_blocks_in_history_are_dropped() {
        // 實測：reasoning item 連同 encrypted_content 整個不回送也回 200，
        // 而我們產生的 thinking 區塊沒有 encrypted_content，送回去只是雜訊。
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"assistant","content":[
                    {"type":"thinking","thinking":"嗯"},
                    {"type":"text","text":"答案"}
                ]}
            ]})),
            "m",
        );
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["content"][0]["text"], "答案");
    }
}
