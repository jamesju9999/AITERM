//! ai_complete：給「側欄功能」用的純文字補全。它**不能**套用終端機助手的系統提示詞
//! （那段會叫模型把指令包進 `<cmd>…</cmd>`，結果「提示詞助手」改寫出來的提示詞就被 `<cmd>` 包住了）。
//! ai_complete 本身是 #[tauri::command]（要 AppHandle/State），無法在整合測試建構，
//! 所以測它的純建構塊：build_plain_request。

use aiterm_lib::ai::{ChatMessage, QueryMode};
use aiterm_lib::commands::ai::{build_chat_prompt, build_plain_request};
use aiterm_lib::ai::context::snapshot_from_remote_ctx;
use aiterm_lib::ai::Locale;

fn user(text: &str) -> ChatMessage {
    ChatMessage { role: "user".into(), content: serde_json::json!(text), tool_call_id: None, tool_calls: None }
}

#[test]
fn uses_exactly_the_callers_system_prompt() {
    let req = build_plain_request(vec![user("hi")], "只回傳 JSON".into());
    assert_eq!(req.system_prompt, "只回傳 JSON");
}

#[test]
fn does_not_carry_the_terminal_assistant_rules() {
    let req = build_plain_request(vec![user("hi")], "plain".into());
    for needle in ["<cmd>", "terminal assistant", "Environment:", "Cwd:"] {
        assert!(!req.system_prompt.contains(needle), "{needle} leaked into: {}", req.system_prompt);
    }
    // 對照組：原本的聊天提示詞確實含有 <cmd> 規則——這就是被借來用時出問題的來源。
    let chat = build_chat_prompt(&snapshot_from_remote_ctx("linux", None, None, None), Locale::En, false);
    assert!(chat.contains("<cmd>"), "control: the chat prompt is expected to contain the <cmd> rule");
}

#[test]
fn keeps_the_messages_untouched_and_uses_chat_mode() {
    let msgs = vec![user("第一句"), user("第二句")];
    let req = build_plain_request(msgs.clone(), "p".into());
    assert_eq!(req.messages.len(), 2);
    assert_eq!(req.messages[1].content, msgs[1].content);
    assert!(matches!(req.mode, QueryMode::Chat));
    assert!(req.max_tokens.is_none());
}

#[test]
fn carries_no_terminal_context() {
    let req = build_plain_request(vec![user("hi")], "p".into());
    assert!(req.context.recent_output.is_none());
    assert!(req.context.dir_listing.is_none());
}
