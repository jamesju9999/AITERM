//! 「補充一句」：執行中的卡片，不開分頁、不停掉重建，直接把一句話送進它的 Claude session。
//!
//! 這裡只做純判斷（能不能送、送什麼）；真正寫進終端機在 `commands::tasks`。
//! 補充一句等於使用者介入對話：呼叫端要把它記成使用者輸入，這張自動卡才會轉成互動卡，
//! 不會在 Claude 回完補充那一輪就被當成完成而結案。

use crate::tasks::store::TaskRow;

/// 補充訊息長度上限（字元）。再長請走「接著談」或建新卡。
pub const MAX_CHARS: usize = 2000;

/// 能送就回「要打進終端機的文字（已整理）」與分頁 id，不能送就回原因（直接給使用者看）。
pub fn plan(task: &TaskRow, message: &str) -> Result<(String, String), String> {
    if task.status != "running" {
        return Err("這張卡不在執行中；已完成的卡請用「接著談」。".into());
    }
    let Some(tab_id) = task.tab_id.clone().filter(|t| !t.is_empty()) else {
        return Err("這張卡的分頁還沒開好，請稍候再試。".into());
    };
    if task.cli.as_deref().is_some_and(|c| !c.eq_ignore_ascii_case("claude")) {
        return Err("補充一句目前只支援 Claude Code 的卡片。".into());
    }
    // 整理：換行壓成空白（多行貼進 TUI 會被當成貼上、不會送出），去頭尾空白。
    let text = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return Err("沒有內容可以送。".into());
    }
    if text.chars().count() > MAX_CHARS {
        return Err(format!("太長了（上限 {MAX_CHARS} 字）。長的內容請用「接著談」或建一張新卡。"));
    }
    Ok((tab_id, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running() -> TaskRow {
        TaskRow {
            status: "running".into(),
            tab_id: Some("tab-1".into()),
            ..crate::tasks::workflow::tests::test_row("c1")
        }
    }

    #[test]
    fn a_running_card_with_a_tab_accepts_a_message_and_tidies_it() {
        let (tab, text) = plan(&running(), "  請改用\n 繁體中文  \r\n回答 ").unwrap();
        assert_eq!(tab, "tab-1");
        assert_eq!(text, "請改用 繁體中文 回答");
    }

    #[test]
    fn each_refusal_says_why() {
        let err = |t: TaskRow, m: &str| plan(&t, m).unwrap_err();
        assert!(err(TaskRow { status: "done".into(), ..running() }, "x").contains("接著談"));
        assert!(err(TaskRow { status: "queued".into(), ..running() }, "x").contains("不在執行中"));
        assert!(err(TaskRow { tab_id: None, ..running() }, "x").contains("還沒開好"));
        assert!(err(TaskRow { tab_id: Some(String::new()), ..running() }, "x").contains("還沒開好"));
        assert!(err(TaskRow { cli: Some("codex".into()), ..running() }, "x").contains("Claude"));
        assert!(err(running(), "  \n\t ").contains("沒有內容"));
        assert!(err(running(), &"字".repeat(MAX_CHARS + 1)).contains("太長"));
    }

    #[test]
    fn the_length_limit_counts_characters_not_bytes() {
        assert!(plan(&running(), &"字".repeat(MAX_CHARS)).is_ok());
    }
}
