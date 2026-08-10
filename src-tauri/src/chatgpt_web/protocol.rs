//! ChatGPT 網頁版的協定轉換：歷史攤平與 SSE 解析。

/// 攤平前的一個回合。
///
/// 網頁版沒有 `role: "tool"` 這種結構化角色，工具回合只能以文字回填——
/// 這是此傳輸路徑的先天限制，不是實作取捨。見 spec 的「工具結果的回填」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlatTurn {
    User(String),
    Assistant(String),
    ToolCall { id: String, name: String, args: String },
    ToolResult { id: String, content: String },
}

/// 把 system prompt 與整段歷史攤平成單一 user turn 的內容。
///
/// 界定符 `[[tool_call:…]]` / `[[tool_result:…]]` 刻意與模型要輸出的
/// `<tool>` 封套不同：若共用同一組標籤，`tools::parse_tool_calls` 可能把
/// 歷史裡我們自己寫進去的回合誤判成模型發出的新呼叫。nonce 檢查雖然也會
/// 擋下（歷史回合沒有 `_nonce`），但不該把正確性建立在第二道防線上。
pub fn flatten_history(system_prompt: &str, turns: &[FlatTurn]) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !system_prompt.trim().is_empty() {
        parts.push(system_prompt.to_string());
    }
    for turn in turns {
        parts.push(match turn {
            FlatTurn::User(text) => format!("User: {text}"),
            FlatTurn::Assistant(text) => format!("Assistant: {text}"),
            FlatTurn::ToolCall { id, name, args } => {
                format!("[[tool_call:{name}#{id}]]\n{args}\n[[/tool_call]]")
            }
            FlatTurn::ToolResult { id, content } => {
                format!("[[tool_result:{id}]]\n{content}\n[[/tool_result]]")
            }
        });
    }
    parts.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_system_and_turns_in_order() {
        let out = flatten_history(
            "你是助理",
            &[
                FlatTurn::User("第一問".into()),
                FlatTurn::Assistant("第一答".into()),
                FlatTurn::User("第二問".into()),
            ],
        );
        assert!(out.starts_with("你是助理"), "system 要在最前面，實際：{out}");
        let first = out.find("第一問").unwrap();
        let second = out.find("第二問").unwrap();
        assert!(first < second, "順序要保留");
        assert!(out.contains("第一答"));
    }

    #[test]
    fn system_prompt_omitted_when_empty() {
        let out = flatten_history("", &[FlatTurn::User("只有這句".into())]);
        assert!(!out.starts_with('\n'), "空的 system 不該留下空行，實際：{out:?}");
        assert!(out.contains("只有這句"));
    }

    /// 界定符刻意與模型要輸出的 `<tool>` 封套不同——共用會讓剖析器把歷史
    /// 誤判成模型發出的新呼叫。
    #[test]
    fn tool_turns_use_distinct_delimiters() {
        let out = flatten_history(
            "",
            &[
                FlatTurn::ToolCall { id: "call_1".into(), name: "Read".into(),
                                     args: r#"{"path":"a.txt"}"#.into() },
                FlatTurn::ToolResult { id: "call_1".into(), content: "檔案內容".into() },
            ],
        );
        assert!(out.contains("[[tool_call:Read#call_1]]"), "實際：{out}");
        assert!(out.contains(r#"{"path":"a.txt"}"#));
        assert!(out.contains("[[/tool_call]]"));
        assert!(out.contains("[[tool_result:call_1]]"));
        assert!(out.contains("檔案內容"));
        assert!(out.contains("[[/tool_result]]"));
        assert!(!out.contains("<tool>"), "不可使用模型輸出用的封套標籤");
        assert!(!out.contains("<tool_call"), "不可使用模型輸出用的封套標籤");
    }
}
