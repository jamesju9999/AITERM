//! Agent 協作的「系統連線」技能：每位 Agent 各自勾選能用哪些系統。
//! 派工時把 `sys_*` 工具（限定 token，只含被勾選的系統）掛給那張卡，並在提示裡說明規矩。

use serde_json::{json, Value};
use sqlx::SqlitePool;

use super::endpoint::MCP_SERVER_ID;
use super::types::Mode;

/// `--allowedTools` 裡代表整個系統連線伺服器的工具。
pub const ALLOWED_TOOLS: &str = "mcp__aiterm-systems";

/// `agents.systems` 欄位：被勾選的系統 id 的 JSON 陣列。沒裝、壞掉都當作空。
pub fn parse(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|r| serde_json::from_str::<Vec<String>>(r).ok()).unwrap_or_default()
}

pub async fn init_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let _ = sqlx::query("ALTER TABLE agents ADD COLUMN systems TEXT").execute(pool).await;
    Ok(())
}

/// 設定（空清單＝移除）。下一張派出去的卡才生效。
pub async fn set_agent_systems(pool: &SqlitePool, agent_id: &str, ids: &[String]) -> Result<(), sqlx::Error> {
    let mut clean: Vec<&String> = ids.iter().filter(|i| !i.trim().is_empty()).collect();
    clean.sort();
    clean.dedup();
    let raw = (!clean.is_empty()).then(|| serde_json::to_string(&clean).unwrap_or_default());
    sqlx::query("UPDATE agents SET systems = ? WHERE id = ?").bind(raw).bind(agent_id).execute(pool).await?;
    Ok(())
}

/// 被勾選的系統裡，真的存在而且啟用的（id、名稱、權限）。刪掉或停用的系統不給 Agent。
pub fn resolve(ids: &[String], conns: &[super::types::SystemConnection]) -> Vec<(String, String, Mode)> {
    ids.iter()
        .filter_map(|id| conns.iter().find(|c| &c.id == id && c.enabled))
        .map(|c| (c.id.clone(), c.name.clone(), c.mode))
        .collect()
}

/// 把系統連線端點加進 `--mcp-config` 的內容（原本沒有就新建）。
pub fn add_server_to_mcp_json(existing: Option<String>, port: u16, token: &str) -> String {
    let mut v: Value = existing.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| json!({ "mcpServers": {} }));
    if !v["mcpServers"].is_object() {
        v["mcpServers"] = json!({});
    }
    v["mcpServers"][MCP_SERVER_ID] = json!({
        "type": "http",
        "url": format!("http://127.0.0.1:{port}/systems/mcp"),
        "headers": { "Authorization": format!("Bearer {token}") }
    });
    v.to_string()
}

fn mode_text(m: Mode) -> &'static str {
    match m {
        Mode::ReadOnly => "只讀，所有寫入都會被擋下",
        Mode::ApproveEach => "每次寫入都要等使用者核准（沒人核准 5 分鐘後會被拒絕）",
        Mode::Rules => "依使用者設的規則放行，其餘要等核准",
        Mode::Autonomous => "可直接寫入（仍會留下記錄）",
    }
}

/// 附在派工提示後面：能用哪些系統、權限各是什麼，以及做事的規矩。
pub fn prompt_note(systems: &[(String, String, Mode)]) -> String {
    if systems.is_empty() {
        return String::new();
    }
    let list: Vec<String> = systems.iter().map(|(id, name, mode)| format!("- {name}（system id：{id}）：{}", mode_text(*mode))).collect();
    format!(
        "\n\n外部系統：你可以用 sys_* 工具操作下面這些網頁系統（不要用別的方式，例如自己寫腳本去連線）：\n{}\n\
         規矩：每次呼叫都帶 system；每個動作都帶 intent，並以「讀：」或「寫：」開頭；\
         先用 sys_recipe_list 看有沒有適用的配方；動作被閘門拒絕（回覆含「被閘門拒絕」）時不可重試，改用別的做法或回報；\
         權限是使用者設定的，不要想辦法繞過。登入失效或找不到元素時停下來回報。",
        list.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stored_list_parses_and_junk_is_empty() {
        assert_eq!(parse(Some(r#"["a","b"]"#)), vec!["a", "b"]);
        assert!(parse(Some("not json")).is_empty() && parse(None).is_empty());
    }

    #[test]
    fn only_existing_enabled_systems_are_given_to_an_agent() {
        use super::super::types::SystemConnection;
        let c = |id: &str, enabled: bool| SystemConnection {
            id: id.into(), name: format!("名{id}"), start_url: "https://x/".into(), allowed_hosts: vec!["x".into()],
            mode: Mode::Rules, rules: vec![], enabled,
        };
        let got = resolve(&["a".into(), "gone".into(), "off".into(), "b".into()], &[c("a", true), c("b", true), c("off", false)]);
        assert_eq!(got.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), vec!["a", "b"]);
        assert_eq!(got[0], ("a".to_string(), "名a".to_string(), Mode::Rules));
    }

    #[test]
    fn the_server_is_added_to_an_existing_config_without_losing_the_others() {
        let base = r#"{"mcpServers":{"aiterm":{"type":"http","url":"http://x"}}}"#.to_string();
        let out: Value = serde_json::from_str(&add_server_to_mcp_json(Some(base), 48320, "tok")).unwrap();
        assert_eq!(out["mcpServers"]["aiterm"]["url"], "http://x");
        assert_eq!(out["mcpServers"]["aiterm-systems"]["url"], "http://127.0.0.1:48320/systems/mcp");
        assert_eq!(out["mcpServers"]["aiterm-systems"]["headers"]["Authorization"], "Bearer tok");
        let fresh: Value = serde_json::from_str(&add_server_to_mcp_json(None, 1, "t")).unwrap();
        assert!(fresh["mcpServers"]["aiterm-systems"].is_object());
    }

    #[test]
    fn the_prompt_note_names_each_system_with_its_permission_and_is_empty_without_systems() {
        assert_eq!(prompt_note(&[]), "");
        let n = prompt_note(&[("s1".into(), "訂單系統".into(), Mode::ReadOnly), ("s2".into(), "郵件".into(), Mode::ApproveEach)]);
        assert!(n.contains("訂單系統") && n.contains("s1") && n.contains("只讀") && n.contains("郵件") && n.contains("核准"));
        assert!(n.contains("被閘門拒絕") && n.contains("intent"));
    }

    #[tokio::test]
    async fn systems_are_saved_deduped_and_cleared_per_agent() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE agents (id TEXT PRIMARY KEY)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO agents (id) VALUES ('a1')").execute(&pool).await.unwrap();
        init_schema(&pool).await.unwrap();
        set_agent_systems(&pool, "a1", &["b".into(), "a".into(), "a".into(), " ".into()]).await.unwrap();
        let raw: Option<String> = sqlx::query_scalar("SELECT systems FROM agents WHERE id='a1'").fetch_one(&pool).await.unwrap();
        assert_eq!(parse(raw.as_deref()), vec!["a", "b"]);
        set_agent_systems(&pool, "a1", &[]).await.unwrap();
        let raw: Option<String> = sqlx::query_scalar("SELECT systems FROM agents WHERE id='a1'").fetch_one(&pool).await.unwrap();
        assert_eq!(raw, None, "空清單＝移除");
    }
}
