//! 卡片之間的廣播：執行中的一位 Agent 把一句話送進另一位正在執行的 Agent 的對話，
//! 讓平行工作的人能當場協調（例如「我把共用函式改名了」），像同一個 session 裡的人
//! 隨時插話。送達的管道跟使用者「補充一句」一樣（見 `nudge`）。

use crate::tasks::agents::AgentRow;

/// 一句話的字數上限；要短，才不會變成貼一大段把對方的對話洗掉。
pub const MAX_CHARS: usize = 500;
/// 同一張卡兩次送訊息的最短間隔，擋住兩位 Agent 互相回覆停不下來。
pub const MIN_INTERVAL_SECS: u64 = 5;

/// 壓成一行；空白與過長擋下並說明原因。
pub fn clean(message: &str) -> Result<String, String> {
    let one_line = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.is_empty() {
        return Err("訊息不能是空的".into());
    }
    if one_line.chars().count() > MAX_CHARS {
        return Err(format!("訊息太長（上限 {MAX_CHARS} 字），請濃縮成一兩句"));
    }
    Ok(one_line)
}

/// 依名稱找收件的 Agent（不分大小寫）。不能是自己；只支援 Claude 的卡（Codex 的對話不吃這種插話）。
pub fn resolve_target<'a>(agents: &'a [AgentRow], name: &str, sender_id: Option<&str>) -> Result<&'a AgentRow, String> {
    let name = name.trim();
    let target = agents.iter().find(|a| a.name.trim().eq_ignore_ascii_case(name)).ok_or_else(|| {
        let others: Vec<&str> = agents.iter().filter(|a| Some(a.id.as_str()) != sender_id).map(|a| a.name.as_str()).collect();
        format!("找不到叫「{name}」的 Agent。現有的：{}", if others.is_empty() { "（沒有其他人）".to_string() } else { others.join("、") })
    })?;
    if Some(target.id.as_str()) == sender_id {
        return Err("不能傳給自己。".into());
    }
    if target.cli != "claude" {
        return Err(format!("「{}」不是 Claude Code，無法即時傳訊；請改用 remember 記進專案記事本。", target.name));
    }
    Ok(target)
}

/// 實際打進對方對話的那一行：標明來源，並說明不必特地回覆（免得兩邊一來一往）。
pub fn wire_text(sender_name: &str, message: &str) -> String {
    let from = if sender_name.trim().is_empty() { "另一位 Agent" } else { sender_name.trim() };
    format!("【來自 {from} 的訊息，供你參考，手邊這步照做，不用特地回覆】{message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str, name: &str, cli: &str) -> AgentRow {
        AgentRow {
            id: id.into(), kind: "coder".into(), name: name.into(), role: String::new(), prompt_prefix: String::new(),
            project_dir: "/r".into(), canvas_x: 0.0, canvas_y: 0.0, worktree_path: None, branch: None,
            paused_task_id: None, created_at: String::new(), last_merge_signature: None, produces_spec: false,
            model: None, cli: cli.into(), db_skill: None, custom_skills: None,
        }
    }

    #[test]
    fn messages_become_one_line_and_blank_or_huge_are_refused() {
        assert_eq!(clean("  共用函式\n改名成 fmt  ").unwrap(), "共用函式 改名成 fmt");
        assert!(clean(" \n").is_err());
        assert!(clean(&"字".repeat(MAX_CHARS + 1)).unwrap_err().contains("太長"));
        assert!(clean(&"字".repeat(MAX_CHARS)).is_ok());
    }

    #[test]
    fn the_target_is_found_by_name_ignoring_case_and_never_the_sender() {
        let agents = vec![agent("a1", "Coder1", "claude"), agent("a2", "工程師2", "claude")];
        assert_eq!(resolve_target(&agents, " coder1 ", Some("a2")).unwrap().id, "a1");
        assert_eq!(resolve_target(&agents, "工程師2", Some("a1")).unwrap().id, "a2");
        assert!(resolve_target(&agents, "工程師2", Some("a2")).unwrap_err().contains("自己"));
    }

    #[test]
    fn an_unknown_name_lists_who_exists_without_the_sender() {
        let agents = vec![agent("a1", "工程師1", "claude"), agent("a2", "工程師2", "claude")];
        let e = resolve_target(&agents, "老王", Some("a1")).unwrap_err();
        assert!(e.contains("老王") && e.contains("工程師2") && !e.contains("工程師1"), "{e}");
    }

    #[test]
    fn a_codex_agent_cannot_be_messaged_and_the_reason_points_to_notes() {
        let agents = vec![agent("a1", "工程師1", "claude"), agent("a2", "工程師2", "codex")];
        let e = resolve_target(&agents, "工程師2", Some("a1")).unwrap_err();
        assert!(e.contains("remember"), "{e}");
    }

    #[test]
    fn the_wire_text_names_the_sender_and_says_no_reply_is_needed() {
        let s = wire_text("工程師1", "formatTime 改名成 fmt");
        assert!(s.contains("工程師1") && s.contains("formatTime 改名成 fmt") && s.contains("不用特地回覆"));
        assert!(wire_text("", "x").contains("另一位 Agent"));
        assert!(!s.contains('\n'), "要是一行，才會被當成一則訊息送出");
    }
}
