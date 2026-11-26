//! Agent 的「資料庫」技能：設定格式、權限判定與提示詞。桌面版與 server 共用。
//! 見 `docs/superpowers/specs/2026-10-19-agent-db-skill-design.md`。

use serde::{Deserialize, Serialize};

use crate::sql_guard;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DbMode {
    /// 只能跑唯讀查詢。
    Read,
    /// 寫入要使用者每次核准。
    Approve,
    /// 寫入直接放行。
    Write,
}

/// 存在 `agents.db_skill` 的 JSON。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbSkill {
    pub connections: Vec<String>,
    pub mode: DbMode,
}

/// `agents.db_skill` 欄位的值；沒裝、壞掉或沒有任何連線都是 `None`。
pub fn parse(raw: Option<&str>) -> Option<DbSkill> {
    raw.and_then(|r| serde_json::from_str::<DbSkill>(r).ok()).filter(|s| !s.connections.is_empty())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// 要使用者核准才執行。
    NeedApproval,
    Deny(String),
}

/// 這位 Agent 能不能在這個連線跑這句 SQL。
pub fn authorize(skill: &DbSkill, connection_id: &str, sql: &str) -> Decision {
    if !skill.connections.iter().any(|c| c == connection_id) {
        return Decision::Deny("這個連線沒有開放給你，先用 db_list_connections 看可以用哪些。".into());
    }
    if sql_guard::is_read_only(sql) {
        return Decision::Allow;
    }
    match skill.mode {
        DbMode::Read => Decision::Deny(
            "這個連線只開放唯讀查詢（SELECT／WITH／SHOW／DESCRIBE／EXPLAIN，一次一句）。需要寫入請在待確認事項說明。".into(),
        ),
        DbMode::Approve => Decision::NeedApproval,
        DbMode::Write => Decision::Allow,
    }
}

/// 卡片提示詞後面附的一段：告訴 Agent 有哪些連線可以用、權限是什麼。
/// `names`：連線 id → 名稱（找不到名稱就顯示 id）。
pub fn prompt_note(skill: &DbSkill, names: &[(String, String)]) -> String {
    let list: Vec<String> = skill
        .connections
        .iter()
        .map(|id| {
            let name = names.iter().find(|(i, _)| i == id).map_or(id.as_str(), |(_, n)| n.as_str());
            format!("{name}（id：{id}）")
        })
        .collect();
    let rule = match skill.mode {
        DbMode::Read => "只能執行唯讀查詢",
        DbMode::Approve => "唯讀查詢直接執行；寫入要等使用者核准，被拒絕就不要硬做",
        DbMode::Write => "可以讀寫，寫入前先確認會影響哪些資料",
    };
    format!(
        "\n\n資料庫：你可以用 db_list_connections、db_list_tables、db_table_schema、db_query 工具存取這些連線：{}。{rule}。\
         帳密由 AITerm 管理，不要在程式碼或檔案裡寫死連線資訊。",
        list.join("、")
    )
}


#[cfg(test)]
mod tests {
    use super::*;

    fn skill(mode: DbMode) -> DbSkill {
        DbSkill { connections: vec!["dev".into()], mode }
    }

    #[test]
    fn parse_ignores_missing_broken_and_empty() {
        assert_eq!(parse(None), None);
        assert_eq!(parse(Some("not json")), None);
        assert_eq!(parse(Some(r#"{"connections":[],"mode":"read"}"#)), None);
        assert_eq!(parse(Some(r#"{"connections":["dev"],"mode":"approve"}"#)), Some(skill(DbMode::Approve)));
    }

    #[test]
    fn authorization_by_connection_and_mode() {
        let read = "SELECT * FROM t";
        let write = "DELETE FROM t";
        for mode in [DbMode::Read, DbMode::Approve, DbMode::Write] {
            assert_eq!(authorize(&skill(mode), "dev", read), Decision::Allow, "{mode:?} 唯讀一律放行");
            assert!(matches!(authorize(&skill(mode), "prod", read), Decision::Deny(_)), "{mode:?} 沒開放的連線");
        }
        assert!(matches!(authorize(&skill(DbMode::Read), "dev", write), Decision::Deny(m) if m.contains("唯讀")));
        assert_eq!(authorize(&skill(DbMode::Approve), "dev", write), Decision::NeedApproval);
        assert_eq!(authorize(&skill(DbMode::Write), "dev", write), Decision::Allow);
    }

    #[test]
    fn the_prompt_note_names_connections_and_the_rule() {
        let note = prompt_note(&DbSkill { connections: vec!["dev".into(), "x".into()], mode: DbMode::Read }, &[("dev".into(), "HTBKTW-DEV".into())]);
        assert!(note.contains("HTBKTW-DEV（id：dev）") && note.contains("x（id：x）"), "{note}");
        assert!(note.contains("唯讀") && note.contains("db_query"), "{note}");
        assert!(prompt_note(&skill(DbMode::Approve), &[]).contains("核准"));
    }
}
