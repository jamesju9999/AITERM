//! ChatGPT 網頁版上游（Claude Code 橋接用）。
//!
//! 與 `ai/chatgpt_web.rs` 的差別只在輸入形狀：那邊吃 `GenerateRequest`，
//! 這邊吃 Anthropic 的 `MessagesRequest`（含 tool_use / tool_result 區塊）。
//! 契約與提醒的文字兩邊共用 `chatgpt_web::tools`，不要各寫一份。

use async_trait::async_trait;
use futures_util::stream;

use crate::ai::{AiError, McpToolDefinition};
use crate::bridge::anthropic::request::{
    parse_content, system_text, ContentBlock, MessagesRequest, ToolDef,
};
use crate::bridge::upstream::{BridgeUpstream, StopReason, UpstreamEvent, UpstreamResponse, Usage};
use crate::chatgpt_web::protocol::{flatten_history, FlatTurn, SseOut, SseParser};
use crate::chatgpt_web::{session, tools};

pub struct ChatgptWebUpstream;

/// Anthropic 的 `ToolDef` → `McpToolDefinition`。
///
/// 存在的理由是**不要寫第二份契約文字**：橋接路徑與 `AiProvider` 路徑轉成同一
/// 個型別後共用 `build_contract` / `build_reminder`。兩份文字會漸漸漂移，而契約
/// 措辭正是 OmniRoute #7679 花力氣調出來的東西。
///
/// 轉換寫在這裡而不是 `chatgpt_web::tools`，是為了不讓那個模組反過來依賴
/// `bridge`——它目前對整個 crate 零依賴（除了 `ai` 的兩個資料型別）。
fn tool_defs_from(tools: Option<&Vec<ToolDef>>) -> Vec<McpToolDefinition> {
    tools
        .map(|ts| {
            ts.iter()
                .map(|t| McpToolDefinition {
                    name: t.name.clone(),
                    description: t.description.clone().unwrap_or_default(),
                    input_schema: t.input_schema.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 把 Anthropic Messages 請求攤平成注入腳本要的 payload。
pub fn build_payload(req: &MessagesRequest, model: &str, nonce: &str) -> serde_json::Value {
    let mut turns: Vec<FlatTurn> = Vec::new();
    for m in &req.messages {
        for block in parse_content(&m.content) {
            match block {
                ContentBlock::Text(t) | ContentBlock::Thinking(t) => {
                    if m.role == "assistant" {
                        turns.push(FlatTurn::Assistant(t));
                    } else {
                        turns.push(FlatTurn::User(t));
                    }
                }
                ContentBlock::ToolUse { id, name, input } => {
                    turns.push(FlatTurn::ToolCall { id, name, args: input.to_string() });
                }
                ContentBlock::ToolResult { tool_use_id, content, .. } => {
                    let text = content
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::Text(t) => Some(t.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    turns.push(FlatTurn::ToolResult { id: tool_use_id, content: text });
                }
                ContentBlock::Image { .. } => {
                    turns.push(FlatTurn::User("[圖片：此供應商不支援]".into()));
                }
            }
        }
    }

    let tool_defs = tool_defs_from(req.tools.as_ref());
    let reminder = tools::build_reminder(&tool_defs);
    if !reminder.is_empty() {
        // 掛在「最後一個回合」而非「最後一個 user 回合」：agent loop 裡最後一則
        // 訊息通常只含 tool_result 區塊，攤出來一個 FlatTurn::User 都沒有。往回
        // 找 User 會把提醒掛到很前面的回合、甚至掛不上去——而雙位置注入的依據
        // （OmniRoute #7679）正是「貼近當前回合」才有效。多輪工具迴圈是這個
        // provider 最需要提醒生效的場景，不能剛好是它失準的場景。
        match turns.last_mut() {
            Some(FlatTurn::User(text)) => text.push_str(&reminder),
            _ => turns.push(FlatTurn::User(reminder.trim_start().to_string())),
        }
    }

    let mut text = flatten_history(&system_text(req.system.as_ref()), &turns);
    let contract = tools::build_contract(&tool_defs, nonce);
    if !contract.is_empty() {
        text.push_str("\n\n");
        text.push_str(&contract);
    }
    serde_json::json!({ "text": text, "model": model })
}

#[async_trait]
impl BridgeUpstream for ChatgptWebUpstream {
    async fn send(&self, req: &MessagesRequest, model: &str) -> Result<UpstreamResponse, AiError> {
        let s = session::get().ok_or(AiError::NotConfigured)?;
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let payload = build_payload(req, model, &nonce).to_string();
        // `_guard` 必須綁具名變數：綁成 `_` 會當場 drop，sink 立刻被移除，
        // 接下來一個 chunk 都收不到。
        let (mut rx, _guard) = s
            .request(payload)
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        // 工具封套可能跨 chunk，必須收滿整段才剖析——所以這裡不是逐 chunk
        // 轉發，而是收完後一次產生事件序列。使用者端的體感差異由 Claude Code
        // 自己的等待指示吸收。
        let mut full = String::new();
        let mut parser = SseParser::default();
        while let Some(raw) = rx.recv().await {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
                    let status = v.get("status").and_then(|s| s.as_u64());
                    return Err(crate::ai::chatgpt_web::map_upstream_error(err, status));
                }
            }
            // 餵原始 chunk，不要自己 `raw.lines()`——HTTP chunk 不保證切在行
            // 邊界上，行緩衝在 SseParser 裡。
            let mut finished = false;
            for out in parser.feed_str(&raw) {
                match out {
                    SseOut::Text(d) => full.push_str(&d),
                    // 一定要主動結束，不能等通道關閉。
                    SseOut::Done => finished = true,
                }
            }
            if finished {
                break;
            }
        }

        let (content, calls) = tools::parse_tool_calls(&full, &nonce);
        let mut events: Vec<Result<UpstreamEvent, AiError>> = Vec::new();
        if !content.trim().is_empty() {
            events.push(Ok(UpstreamEvent::TextDelta(content)));
        }
        let stop = match &calls {
            Some(calls) => {
                for c in calls {
                    events.push(Ok(UpstreamEvent::ToolUseStart {
                        id: c.id.clone(),
                        name: c.tool_name.clone(),
                    }));
                    events.push(Ok(UpstreamEvent::ToolInputDelta(c.args.to_string())));
                    events.push(Ok(UpstreamEvent::ToolUseEnd));
                }
                StopReason::ToolUse
            }
            None => StopReason::EndTurn,
        };
        events.push(Ok(UpstreamEvent::Done {
            stop_reason: stop,
            usage: Usage { input_tokens: 0, output_tokens: 0 },
        }));

        Ok(UpstreamResponse::Events(Box::pin(stream::iter(events))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(json: serde_json::Value) -> MessagesRequest {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn tool_result_blocks_become_delimited_turns() {
        let r = req(serde_json::json!({
            "model": "aiterm:opus",
            "messages": [
                { "role": "user", "content": "讀檔" },
                { "role": "assistant", "content": [
                    { "type": "tool_use", "id": "tu_1", "name": "Read", "input": {"path": "a"} }
                ]},
                { "role": "user", "content": [
                    { "type": "tool_result", "tool_use_id": "tu_1", "content": "內容" }
                ]}
            ]
        }));
        let p = build_payload(&r, "gpt-5-5", "n1");
        let text = p["text"].as_str().unwrap();
        assert!(text.contains("[[tool_call:Read#tu_1]]"), "實際：{text}");
        assert!(text.contains("[[tool_result:tu_1]]"));
        assert!(text.contains("內容"));
    }

    #[test]
    fn system_prompt_is_included() {
        let r = req(serde_json::json!({
            "model": "aiterm:opus",
            "system": "你是助理",
            "messages": [{ "role": "user", "content": "哈囉" }]
        }));
        let text = build_payload(&r, "gpt-5-5", "n1")["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(text.contains("你是助理"));
    }

    /// agent loop 的常態：最後一則訊息只含 tool_result，攤出來一個 FlatTurn::User
    /// 都沒有。提醒必須落在整段文字的尾端（契約之前），不能被丟回很前面的
    /// user 回合——多輪工具迴圈正是最需要提醒生效的場景。
    #[test]
    fn reminder_stays_at_the_end_when_last_turn_is_a_tool_result() {
        let r = req(serde_json::json!({
            "model": "aiterm:opus",
            "tools": [{ "name": "Read", "description": "讀檔",
                        "input_schema": {"type": "object"} }],
            "messages": [
                { "role": "user", "content": "讀檔" },
                { "role": "assistant", "content": [
                    { "type": "tool_use", "id": "tu_1", "name": "Read", "input": {"path": "a"} }
                ]},
                { "role": "user", "content": [
                    { "type": "tool_result", "tool_use_id": "tu_1", "content": "內容" }
                ]}
            ]
        }));
        let p = build_payload(&r, "gpt-5-5", "n1");
        let text = p["text"].as_str().unwrap();
        let first_user_at = text.find("讀檔").unwrap();
        let result_at = text.find("[[tool_result:tu_1]]").unwrap();
        let reminder_at = text
            .find("Client protocol reminder")
            .unwrap_or_else(|| panic!("提醒完全沒出現，實際：{text}"));
        let contract_at = text.find("Available tools").unwrap();
        assert!(
            result_at < reminder_at,
            "提醒被掛到工具結果之前了（掉回舊的 user 回合），實際：{text}"
        );
        assert!(first_user_at < reminder_at);
        assert!(reminder_at < contract_at, "完整契約仍要在最後");
    }

    /// 沒帶工具時不該出現契約或提醒——Claude Code 有些請求（例如產生 commit
    /// 訊息）是不帶工具的。
    #[test]
    fn no_tools_means_no_contract_or_reminder() {
        let r = req(serde_json::json!({
            "model": "aiterm:opus",
            "messages": [{ "role": "user", "content": "哈囉" }]
        }));
        let text = build_payload(&r, "gpt-5-5", "n1")["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(!text.contains("Available tools"));
        assert!(!text.contains("Client protocol reminder"));
    }
}
