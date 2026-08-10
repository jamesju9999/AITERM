//! 工具呼叫的 prompt 模擬：契約序列化與封套剖析。
//!
//! ChatGPT 網頁版沒有原生 function calling。做法是在 prompt 裡給模型一份
//! 契約，要它用 `<tool>{…}</tool>` 封套回覆，再從回覆文字剖析回結構。

use crate::ai::McpToolDefinition;

/// 完整契約。放在客戶端訊息**之後**（executor 摺疊 system 訊息後會落在
/// 區塊尾端）。
///
/// 依據 OmniRoute #7679 的實測：prepend 在巨大 system 區塊開頭時，30K 字元
/// prompt 下 chatgpt-web 會回答「tool X is not in my tool set」，成功率 0/3；
/// 改成尾端 + user 訊息提醒的雙位置為 16/17。Claude Code 正是那個形狀。
pub fn build_contract(tools: &[McpToolDefinition], nonce: &str) -> String {
    if tools.is_empty() {
        return String::new();
    }
    let mut lines = vec![
        "The client application provides tools beyond your built-in ones. They are NOT in \
         your native tool registry; they are invoked via a plain-text protocol: the client \
         parses your reply and executes the tool on the user machine. Treat these client \
         tools as fully available to you; never claim they are unavailable."
            .to_string(),
        format!(
            "To invoke one, reply with a single line containing a <tool> block whose JSON \
             includes the secret binding \"_nonce\": \"{nonce}\":"
        ),
        format!(r#"<tool>{{"name": "<tool_name>", "arguments": {{ ... }}, "_nonce": "{nonce}"}}</tool>"#),
        "Only emit the <tool> block when you actually want to call a tool; otherwise answer \
         normally."
            .to_string(),
        String::new(),
        "Available tools:".to_string(),
    ];
    for t in tools {
        lines.push(format!(
            "- {}: {}\n  parameters: {}",
            t.name, t.description, t.input_schema
        ));
    }
    lines.join("\n")
}

/// 掛在最新一則 user 訊息末尾的一行提醒。刻意簡短：網頁版模型對當前 user
/// 回合的權重遠高於龐大的 system 區塊，而 ChatGPT 的注入偵測又不信任藏在
/// user 內容裡的長指令，所以完整契約留在 system 尾端，這裡只指回去並點名工具。
pub fn build_reminder(tools: &[McpToolDefinition]) -> String {
    if tools.is_empty() {
        return String::new();
    }
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    format!(
        "\n\n[Client protocol reminder: the client-tool contract in the system instructions \
         is active in this conversation. These client tools ARE available via the <tool> \
         block protocol: {}.]",
        names.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(name: &str) -> crate::ai::McpToolDefinition {
        crate::ai::McpToolDefinition {
            name: name.into(),
            description: format!("{name} 的說明"),
            input_schema: json!({"type": "object"}),
        }
    }

    #[test]
    fn contract_lists_every_tool_and_embeds_the_nonce() {
        let c = build_contract(&[tool("Read"), tool("Edit")], "abc123");
        assert!(c.contains("Read"));
        assert!(c.contains("Edit"));
        assert!(c.contains("abc123"), "nonce 要出現在契約裡");
        assert!(c.contains("_nonce"), "要明確告訴模型欄位名");
        // 契約存在的唯一目的是讓模型知道「有哪些工具、各自做什麼、參數長怎樣」。
        // 若退化成只有一串工具名稱，模型收到的是無用清單，整個工具模擬功能
        // 會靜默壞掉——但只斷言 contains("Read") 這種測試不會發現，因為名稱
        // 本身就會出現。必須分別鎖住 description 與 input_schema 有被序列化。
        assert!(
            c.contains("Read 的說明") && c.contains("Edit 的說明"),
            "每個工具的 description 要出現在契約裡，否則模型不知道工具做什麼"
        );
        assert!(
            c.contains(r#"{"type":"object"}"#),
            "每個工具的 input_schema 要被序列化進契約，否則模型不知道參數長怎樣"
        );
    }

    #[test]
    fn empty_tool_list_produces_no_contract() {
        assert_eq!(build_contract(&[], "abc123"), "");
    }

    /// 依據 OmniRoute #7679 的實測：契約 prepend 在巨大 system 區塊開頭時，
    /// 30K 字元 prompt 下模型會直接忽略它（0/3），雙位置為 16/17。
    #[test]
    fn reminder_is_short_and_names_the_tools() {
        let r = build_reminder(&[tool("Read"), tool("Edit")]);
        assert!(r.contains("Read") && r.contains("Edit"), "要點名工具");
        assert!(r.len() < 400, "只是指回 system 區塊的一行提示，不是完整契約：{}", r.len());
        assert!(!r.contains("_nonce"), "nonce 只放在完整契約，避免重複洩漏");
    }
}
