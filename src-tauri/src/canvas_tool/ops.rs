//! Ask AI 的 Agent 協作工具：純邏輯（找專案／角色、檢查、建卡、列卡、讀結果）。
//! 不碰 UI 與確認框；確認在 `mod.rs`。設計見
//! docs/superpowers/specs/2026-11-18-ask-ai-agent-canvas-tools-design.md。

use std::path::Path;

use serde_json::json;
use sqlx::SqlitePool;

use crate::projects::ProjectHandle;
use crate::tasks::agents::{self, AgentRow};
use crate::tasks::store::{self, TaskRow};
use crate::tasks::workflow;

/// 派出去的卡內容最前面的來源標記，讓看板上看得出是 Ask AI 派的。
pub const MARK: &str = "【由 Ask AI 派工】";
pub const MAX_TITLE: usize = 80;
pub const MAX_INSTRUCTION: usize = 4000;
const MAX_LIST: usize = 50;
const TRANSCRIPT_TAIL: usize = 3000;

/// 依 id 或名稱（不分大小寫）找專案。`key` 空白且只有一個專案開著就用它。
pub fn find_project<'a>(list: &'a [ProjectHandle], key: &str) -> Result<&'a ProjectHandle, String> {
    if list.is_empty() {
        return Err("目前沒有開啟的專案。請先在工作看板開啟專案。".into());
    }
    let key = key.trim();
    if key.is_empty() {
        return match list {
            [only] => Ok(only),
            _ => Err(format!("有多個專案開著，請指定 project：{}", names(list.iter().map(|p| p.name.as_str())))),
        };
    }
    let hits: Vec<&ProjectHandle> = list.iter().filter(|p| p.id == key || p.name.trim().eq_ignore_ascii_case(key)).collect();
    match hits.as_slice() {
        [one] => Ok(one),
        [] => Err(format!("找不到專案「{key}」。開著的專案：{}", names(list.iter().map(|p| p.name.as_str())))),
        many => Err(format!("有多個專案叫「{key}」，請改用 id：{}", many.iter().map(|p| format!("{}（{}）", p.name, p.id)).collect::<Vec<_>>().join("、"))),
    }
}

/// 依 id 或名稱（不分大小寫）找角色。
pub fn find_agent<'a>(list: &'a [AgentRow], key: &str) -> Result<&'a AgentRow, String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("需要指定 agent（角色名稱或 id）".into());
    }
    let hits: Vec<&AgentRow> = list.iter().filter(|a| a.id == key || a.name.trim().eq_ignore_ascii_case(key)).collect();
    match hits.as_slice() {
        [one] => Ok(one),
        [] => Err(format!("找不到角色「{key}」。現有的：{}", names(list.iter().map(|a| a.name.as_str())))),
        many => Err(format!("有多個角色叫「{key}」，請改用 id：{}", many.iter().map(|a| format!("{}（{}）", a.name, a.id)).collect::<Vec<_>>().join("、"))),
    }
}

fn names<'a>(it: impl Iterator<Item = &'a str>) -> String {
    let v: Vec<&str> = it.collect();
    if v.is_empty() { "（沒有）".into() } else { v.join("、") }
}

/// 檢查派工內容，回傳去掉前後空白的 (標題, 指示)。
pub fn validate_dispatch(agent: &AgentRow, title: &str, instruction: &str) -> Result<(String, String), String> {
    if agent.kind != "coder" && agent.kind != "verifier" {
        return Err(format!("「{}」是 {}，只能派給工程師（coder）或驗收者（verifier）；審核者與總管由流程自動帶入。", agent.name, agent.kind));
    }
    let (title, instruction) = (title.trim(), instruction.trim());
    if title.is_empty() {
        return Err("title 不可空白".into());
    }
    if title.chars().count() > MAX_TITLE {
        return Err(format!("title 太長（上限 {MAX_TITLE} 字）"));
    }
    if instruction.is_empty() {
        return Err("instruction 不可空白：請寫清楚要做什麼、輸出要求是什麼".into());
    }
    if instruction.chars().count() > MAX_INSTRUCTION {
        return Err(format!("instruction 太長（上限 {MAX_INSTRUCTION} 字），請濃縮"));
    }
    Ok((title.to_string(), instruction.to_string()))
}

/// 這個角色現在有沒有排隊或執行中的卡（同工作流程挑角色時算負載的方式）。
pub async fn agent_busy(pool: &SqlitePool, agent_id: &str) -> Result<bool, String> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks WHERE agent_id = ? AND status IN ('queued', 'running') AND archived_at IS NULL")
        .bind(agent_id)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(n > 0)
}

pub fn list_projects_text(list: &[ProjectHandle]) -> String {
    if list.is_empty() {
        return "目前沒有開啟的專案。".into();
    }
    list.iter().map(|p| format!("- {}（id：{}，資料夾：{}）", p.name, p.id, p.path.display())).collect::<Vec<_>>().join("\n")
}

/// 角色清單：名稱、類型、技能、忙不忙。
pub async fn list_agents_text(pool: &SqlitePool) -> Result<String, String> {
    let all = agents::list_agents(pool).await.map_err(|e| e.to_string())?;
    if all.is_empty() {
        return Ok("這個專案還沒有角色。".into());
    }
    let mut lines = Vec::new();
    for a in &all {
        let busy = agent_busy(pool, &a.id).await?;
        let mut skills = Vec::new();
        let sys = crate::systems::skill::parse(a.systems.as_deref());
        if !sys.is_empty() {
            skills.push(format!("系統連線×{}", sys.len()));
        }
        if a.db_skill.is_some() {
            skills.push("資料庫".into());
        }
        if a.custom_skills.is_some() {
            skills.push("自訂技能".into());
        }
        let dispatchable = if a.kind == "coder" || a.kind == "verifier" { "" } else { "（不能直接派工）" };
        lines.push(format!(
            "- {}（id：{}，類型：{}{}，{}，技能：{}）",
            a.name,
            a.id,
            a.kind,
            dispatchable,
            if busy { "忙碌中" } else { "閒置" },
            if skills.is_empty() { "無".to_string() } else { skills.join("、") }
        ));
    }
    Ok(lines.join("\n"))
}

/// 建卡並回傳卡片 id。工程師建工作卡，驗收者建驗收卡（需要 `head_sha`）。
pub async fn dispatch(
    pool: &SqlitePool,
    project_path: &Path,
    agent: &AgentRow,
    title: &str,
    instruction: &str,
    head_sha: Option<&str>,
) -> Result<String, String> {
    if agent_busy(pool, &agent.id).await? {
        return Err(format!("「{}」現在有卡片在排隊或執行，請稍後再派，或改派別的角色。", agent.name));
    }
    let body = format!("{MARK}\n\n{instruction}\n");
    match agent.kind.as_str() {
        "coder" => workflow::create_scheduled_work_card(pool, agent, title, &body).await,
        "verifier" => {
            let sha = head_sha.ok_or("驗收卡需要受驗版本（這個專案的 HEAD），目前取不到")?;
            workflow::create_scheduled_verify_card(pool, project_path, agent, title, &body, sha).await
        }
        other => return Err(format!("不能派給 {other}")),
    }
    .map_err(|e| e.to_string())
}

fn status_label(t: &TaskRow) -> String {
    match (t.status.as_str(), t.outcome.as_deref()) {
        ("done", Some("success")) => "完成".into(),
        ("done", Some("failed")) => "失敗".into(),
        ("done", Some("cancelled")) => "已取消".into(),
        ("done", _) => "完成".into(),
        ("running", _) => "進行中".into(),
        ("queued", _) => "排隊中".into(),
        ("planning", _) => "規劃中".into(),
        (other, _) => other.to_string(),
    }
}

/// 看板上的卡片（不含封存），可依狀態過濾（planning／queued／running／done，或中文標籤）。
pub async fn list_cards_text(pool: &SqlitePool, status: Option<&str>) -> Result<String, String> {
    let tasks = store::list_tasks(pool).await.map_err(|e| e.to_string())?;
    let agents = agents::list_agents(pool).await.map_err(|e| e.to_string())?;
    let want = status.map(str::trim).filter(|s| !s.is_empty());
    let mut rows: Vec<&TaskRow> = tasks
        .iter()
        .filter(|t| want.is_none_or(|w| t.status == w || status_label(t) == w))
        .collect();
    rows.sort_by_key(|t| std::cmp::Reverse(t.dispatched_at.unwrap_or(0).max(t.finished_at.unwrap_or(0))));
    if rows.is_empty() {
        return Ok("沒有符合的卡片。".into());
    }
    let shown = rows.len().min(MAX_LIST);
    let mut out: Vec<String> = rows[..shown]
        .iter()
        .map(|t| {
            let who = t.agent_id.as_deref().and_then(|id| agents.iter().find(|a| a.id == id)).map(|a| a.name.as_str()).unwrap_or("（未指派）");
            format!("- {}（id：{}，狀態：{}，角色：{}，種類：{}）", t.title, t.id, status_label(t), who, t.card_kind)
        })
        .collect();
    if rows.len() > shown {
        out.push(format!("…還有 {} 張沒列出", rows.len() - shown));
    }
    Ok(out.join("\n"))
}

/// 一張卡片的結果：狀態、錯誤、摘要，以及對話記錄最後一段。
pub async fn read_card_text(pool: &SqlitePool, id: &str) -> Result<String, String> {
    let t = store::get_task(pool, id.trim()).await.map_err(|e| e.to_string())?.ok_or_else(|| format!("找不到卡片：{}", id.trim()))?;
    let mut out = vec![format!("{}（id：{}，狀態：{}，種類：{}）", t.title, t.id, status_label(&t), t.card_kind)];
    if t.status != "done" {
        out.push("這張卡片還沒完成，請稍後再查。".into());
    }
    if let Some(e) = t.error_message.as_deref().filter(|e| !e.trim().is_empty()) {
        out.push(format!("錯誤：{e}"));
    }
    if let Some(s) = t.ai_summary.as_deref().filter(|s| !s.trim().is_empty()) {
        out.push(format!("摘要：{s}"));
    }
    let log_path = t.session_path.as_deref().or(t.transcript_path.as_deref());
    if let Some(text) = log_path.and_then(|p| std::fs::read_to_string(p).ok()) {
        let rendered = if t.session_path.is_some() { crate::tasks::session_log::render_session_log(&text) } else { text };
        let tail = tail_chars(rendered.trim(), TRANSCRIPT_TAIL);
        if !tail.is_empty() {
            out.push(format!("對話記錄最後一段：\n{tail}"));
        }
    }
    Ok(out.join("\n"))
}

fn tail_chars(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count <= n { s.to_string() } else { format!("…{}", s.chars().skip(count - n).collect::<String>()) }
}

/// 給確認框用的內容。
pub fn confirm_payload(project: &str, agent: &AgentRow, title: &str, instruction: &str) -> serde_json::Value {
    json!({ "project": project, "agent": agent.name, "agent_kind": agent.kind, "title": title, "instruction": instruction })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str, name: &str, kind: &str) -> AgentRow {
        AgentRow {
            id: id.into(), kind: kind.into(), name: name.into(), role: String::new(), prompt_prefix: String::new(),
            project_dir: "/tmp/x".into(), canvas_x: 0.0, canvas_y: 0.0, worktree_path: None, branch: None, paused_task_id: None,
            created_at: String::new(), last_merge_signature: None, produces_spec: false, model: None, cli: "claude".into(),
            db_skill: None, custom_skills: None, systems: None,
        }
    }

    #[test]
    fn agents_are_found_by_name_or_id_ignoring_case() {
        let l = vec![agent("a1", "小明", "coder"), agent("a2", "Bob", "reviewer")];
        assert_eq!(find_agent(&l, "bob").unwrap().id, "a2");
        assert_eq!(find_agent(&l, "a1").unwrap().name, "小明");
        let e = find_agent(&l, "ghost").unwrap_err();
        assert!(e.contains("小明") && e.contains("Bob"), "{e}");
        assert!(find_agent(&l, "  ").is_err());
    }

    #[test]
    fn two_agents_with_the_same_name_are_not_guessed() {
        let l = vec![agent("a1", "Dev", "coder"), agent("a2", "dev", "coder")];
        let e = find_agent(&l, "DEV").unwrap_err();
        assert!(e.contains("a1") && e.contains("a2"), "{e}");
    }

    #[test]
    fn only_coders_and_verifiers_can_be_dispatched_and_text_is_bounded() {
        assert!(validate_dispatch(&agent("a", "R", "reviewer"), "t", "i").unwrap_err().contains("只能派給工程師"));
        assert!(validate_dispatch(&agent("a", "O", "orchestrator"), "t", "i").is_err());
        let c = agent("a", "C", "coder");
        assert_eq!(validate_dispatch(&c, "  查特休  ", " 去請假系統查 ").unwrap(), ("查特休".into(), "去請假系統查".into()));
        assert!(validate_dispatch(&c, " ", "i").is_err());
        assert!(validate_dispatch(&c, "t", "  ").is_err());
        assert!(validate_dispatch(&c, &"長".repeat(MAX_TITLE + 1), "i").is_err());
        assert!(validate_dispatch(&c, &"長".repeat(MAX_TITLE), "i").is_ok(), "剛好上限可以");
        assert!(validate_dispatch(&c, "t", &"字".repeat(MAX_INSTRUCTION + 1)).is_err());
    }

    #[test]
    fn tail_keeps_the_end_and_marks_the_cut() {
        assert_eq!(tail_chars("abc", 5), "abc");
        assert_eq!(tail_chars("abcdef", 3), "…def");
    }

    async fn pool() -> SqlitePool {
        let p = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&p).await.unwrap();
        p
    }

    async fn make_agent(pool: &SqlitePool, kind: &str, name: &str) -> AgentRow {
        let id = agents::create_agent(
            pool,
            &agents::NewAgent { kind, name, role: kind, prompt_prefix: "", project_dir: "/tmp/repo", canvas_x: 0.0, canvas_y: 0.0 },
        )
        .await
        .unwrap();
        agents::get_agent(pool, &id).await.unwrap().unwrap()
    }

    #[tokio::test]
    async fn dispatching_to_a_coder_creates_a_queued_card_with_the_source_mark() {
        let pool = pool().await;
        let coder = make_agent(&pool, "coder", "小明").await;
        let id = dispatch(&pool, std::path::Path::new("/tmp/p"), &coder, "查特休", "去請假系統查剩餘特休", None).await.unwrap();
        let card = store::get_task(&pool, &id).await.unwrap().unwrap();
        assert_eq!((card.status.as_str(), card.card_kind.as_str()), ("queued", "work"));
        assert_eq!(card.agent_id.as_deref(), Some(coder.id.as_str()));
        assert!(card.body.starts_with(MARK) && card.body.contains("去請假系統查剩餘特休"), "{}", card.body);
        assert_eq!(card.title, "查特休");
    }

    #[tokio::test]
    async fn a_busy_agent_is_refused_and_not_stacked() {
        let pool = pool().await;
        let coder = make_agent(&pool, "coder", "小明").await;
        assert!(!agent_busy(&pool, &coder.id).await.unwrap());
        dispatch(&pool, std::path::Path::new("/tmp/p"), &coder, "一", "做一", None).await.unwrap();
        assert!(agent_busy(&pool, &coder.id).await.unwrap());
        let e = dispatch(&pool, std::path::Path::new("/tmp/p"), &coder, "二", "做二", None).await.unwrap_err();
        assert!(e.contains("小明") && e.contains("排隊或執行"), "{e}");
        assert_eq!(store::list_tasks(&pool).await.unwrap().len(), 1, "不能堆第二張");
    }

    #[tokio::test]
    async fn a_verifier_needs_a_head_version() {
        let pool = pool().await;
        let v = make_agent(&pool, "verifier", "驗收").await;
        let e = dispatch(&pool, std::path::Path::new("/tmp/p"), &v, "驗收", "跑測試", None).await.unwrap_err();
        assert!(e.contains("HEAD"), "{e}");
    }

    #[tokio::test]
    async fn the_board_lists_cards_with_owner_and_filters_by_status() {
        let pool = pool().await;
        let coder = make_agent(&pool, "coder", "小明").await;
        let id = dispatch(&pool, std::path::Path::new("/tmp/p"), &coder, "查特休", "x", None).await.unwrap();
        let all = list_cards_text(&pool, None).await.unwrap();
        assert!(all.contains("查特休") && all.contains(&id) && all.contains("小明") && all.contains("排隊中"), "{all}");
        assert!(list_cards_text(&pool, Some("queued")).await.unwrap().contains("查特休"));
        assert!(list_cards_text(&pool, Some("done")).await.unwrap().contains("沒有符合"));
        let agents_text = list_agents_text(&pool).await.unwrap();
        assert!(agents_text.contains("小明") && agents_text.contains("忙碌中"), "{agents_text}");
    }

    #[tokio::test]
    async fn reading_a_card_says_when_it_is_unfinished_and_shows_the_summary_when_done() {
        let pool = pool().await;
        let coder = make_agent(&pool, "coder", "小明").await;
        let id = dispatch(&pool, std::path::Path::new("/tmp/p"), &coder, "查特休", "x", None).await.unwrap();
        assert!(read_card_text(&pool, &id).await.unwrap().contains("還沒完成"));
        store::set_summary(&pool, &id, "剩餘 34.5 小時").await.unwrap();
        sqlx::query("UPDATE tasks SET status='done', outcome='success' WHERE id = ?").bind(&id).execute(&pool).await.unwrap();
        let t = read_card_text(&pool, &id).await.unwrap();
        assert!(t.contains("完成") && t.contains("剩餘 34.5 小時") && !t.contains("還沒完成"), "{t}");
        assert!(read_card_text(&pool, "nope").await.unwrap_err().contains("找不到卡片"));
    }

    #[tokio::test]
    async fn projects_resolve_by_name_and_a_single_open_project_is_the_default() {
        let p = |id: &str, name: &str| ProjectHandle { id: id.into(), name: name.into(), path: "/p".into(), pool: futures_pool() };
        fn futures_pool() -> SqlitePool {
            sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_lazy("sqlite::memory:").unwrap()
        }
        let one = vec![p("p1", "請假")];
        assert_eq!(find_project(&one, "").unwrap().id, "p1");
        assert_eq!(find_project(&one, "請假").unwrap().id, "p1");
        let two = vec![p("p1", "請假"), p("p2", "訂單")];
        assert!(find_project(&two, "").unwrap_err().contains("請假") );
        assert_eq!(find_project(&two, "訂單").unwrap().id, "p2");
        assert!(find_project(&two, "x").unwrap_err().contains("找不到專案"));
        assert!(find_project(&[], "").unwrap_err().contains("沒有開啟"));
    }
}
