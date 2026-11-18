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
         每次都自己操作畫面取得真實資訊，不要用 sys_recipe_run 重播操作流程（重播只做動作、拿不到內容）；動作被閘門拒絕（回覆含「被閘門拒絕」）時不可重試，改用別的做法或回報；\
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
            mode: Mode::Rules, rules: vec![], enabled, model: None,
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

    /// 真的 Claude Code：拿著限定 token，`sys_list` 只看得到被勾選的系統，用別的系統會被擋。
    /// 會用到少量模型額度。`cargo test -p app --lib -- --ignored --test-threads=1 systems::skill::tests::real_claude`
    #[tokio::test]
    #[ignore]
    async fn real_claude_with_a_scoped_token_sees_and_uses_only_its_systems() {
        use crate::config::ConfigStore;
        use crate::systems::endpoint::EndpointState;
        use crate::systems::state::SystemsState;
        use crate::systems::types::SystemConnection;
        use std::sync::Arc;

        let dir = tempfile::tempdir().unwrap();
        let cfg = Arc::new(ConfigStore::new_at(dir.path().join("config.json")));
        let mk = |id: &str, name: &str| SystemConnection {
            id: id.into(), name: name.into(), start_url: "http://127.0.0.1:1/".into(), allowed_hosts: vec!["127.0.0.1".into()],
            mode: Mode::ReadOnly, rules: vec![], enabled: true, model: None,
        };
        cfg.update(|a| {
            a.system_connections.push(mk("allowed1", "被授權的系統"));
            a.system_connections.push(mk("secret9", "沒授權的系統"));
        })
        .unwrap();
        let st = Arc::new(SystemsState::with_pool(cfg, crate::systems::audit::open_in_memory().await));
        let ep = Arc::new(EndpointState::default());
        let port = crate::systems::browser::pick_free_port().unwrap();
        ep.start(st, "master-token".into(), port).await.unwrap();
        let token = ep.issue_scoped_token(&["allowed1".to_string()]);

        let mcp = dir.path().join("mcp.json");
        std::fs::write(&mcp, add_server_to_mcp_json(None, port, &token)).unwrap();
        let prompt = "請依序做兩件事，並把結果原樣回報：1) 呼叫 sys_list，列出你看到的系統 id；\
                      2) 呼叫 sys_snapshot，system 填 secret9，回報工具回傳的完整文字。不要做別的。";
        let mut child = tokio::process::Command::new("claude")
            .args(["-p", "--model", "haiku", "--mcp-config", &mcp.to_string_lossy(), "--allowedTools", ALLOWED_TOOLS, "--output-format", "text"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("需要 claude");
        use tokio::io::AsyncWriteExt;
        child.stdin.take().unwrap().write_all(prompt.as_bytes()).await.unwrap();
        let out = tokio::time::timeout(std::time::Duration::from_secs(180), child.wait_with_output()).await.unwrap().unwrap();
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        eprintln!("claude 的回報：\n{text}");
        assert!(text.contains("allowed1"), "要看得到被授權的系統：{text}");
        assert!(!text.contains("沒授權的系統"), "不能看得到沒授權的系統的名稱：{text}");
        assert!(text.contains("沒有被授權"), "用別的系統要被擋下並回報：{text}");
        ep.stop();
    }
}
