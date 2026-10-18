//! 每張 Agent 卡共同的「專案背景」：使用者寫的專案簡介＋自動產生的團隊地圖。
//! 平行的 Agent 各自只看到自己那張卡，沒有單一 session 那種全貌；這段讓每位都知道
//! 整個專案在做什麼、別人負責哪些模組。見
//! docs/superpowers/specs/2026-10-18-agent-project-context-design.md。

use std::collections::HashSet;

use sqlx::SqlitePool;

use crate::tasks::agents::AgentRow;
use crate::tasks::store::TaskRow;
use crate::tasks::workflow::{deps_of, module_state, ModuleState, KIND_FIX, KIND_REVIEW, KIND_WORK};

const BRIEF_KEY: &str = "project_brief";
/// 放進提示時的上限（字元）。畫面在 2000 字就提醒，這裡是防止貼上整份文件時把每張卡撐大。
pub const BRIEF_PROMPT_LIMIT: usize = 6000;
/// 模組數在這以內就全部列出；超過時只完整列出跟這張卡有關的，其他只列標題。
pub const MAP_FULL_LIMIT: usize = 15;

pub async fn get_brief(pool: &SqlitePool) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT value FROM project_settings WHERE key = ?").bind(BRIEF_KEY).fetch_optional(pool).await
}

/// 空白＝刪除。
pub async fn set_brief(pool: &SqlitePool, brief: Option<&str>) -> Result<(), sqlx::Error> {
    match brief.map(str::trim).filter(|b| !b.is_empty()) {
        Some(b) => {
            sqlx::query("INSERT INTO project_settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
                .bind(BRIEF_KEY)
                .bind(b)
                .execute(pool)
                .await?;
        }
        None => {
            sqlx::query("DELETE FROM project_settings WHERE key = ?").bind(BRIEF_KEY).execute(pool).await?;
        }
    }
    Ok(())
}

fn state_label(s: ModuleState) -> &'static str {
    match s {
        ModuleState::Planning => "還沒開工",
        ModuleState::Queued => "排隊中",
        ModuleState::Coding => "實作中",
        ModuleState::FixQueued => "等待修正",
        ModuleState::AwaitingReview => "等待審查",
        ModuleState::Reviewing => "審查中",
        ModuleState::Passed => "已通過",
        ModuleState::Attention => "需要人處理",
        ModuleState::Failed => "失敗",
        ModuleState::ReviewFailed => "審查失敗",
        ModuleState::Dispatching => "拆解中",
        ModuleState::DispatchFailed => "拆解失敗",
    }
}

/// 這張卡屬於哪個模組：模組卡是自己，修正卡與審查卡記在 `module_id`。
fn own_module(task: &TaskRow) -> Option<&str> {
    match task.card_kind.as_str() {
        KIND_WORK => Some(task.id.as_str()),
        KIND_FIX | KIND_REVIEW => task.module_id.as_deref(),
        _ => None,
    }
}

/// 團隊地圖：已指派給工程師的模組，每個一行（標題、負責人、狀態、相依）。沒有模組時是空字串。
pub fn team_map(task: &TaskRow, tasks: &[TaskRow], agents: &[AgentRow]) -> String {
    let mut modules: Vec<&TaskRow> = tasks.iter().filter(|t| t.card_kind == KIND_WORK && t.agent_id.is_some()).collect();
    if modules.is_empty() {
        return String::new();
    }
    modules.sort_by(|a, b| a.sort_order.total_cmp(&b.sort_order));
    let has_reviewer = agents.iter().any(|a| a.kind == "reviewer");
    let title_of = |id: &str| tasks.iter().find(|t| t.id == id).map(|t| t.title.as_str());
    let owner_of = |id: Option<&str>| agents.iter().find(|a| Some(a.id.as_str()) == id).map_or("?", |a| a.name.as_str());

    let mine = own_module(task);
    // 跟這張卡有關的：自己的模組、它的前置、以及以它為前置的。
    let mut related: HashSet<&str> = HashSet::new();
    if let Some(m) = mine {
        related.insert(m);
        if let Some(me) = tasks.iter().find(|t| t.id == m) {
            for d in deps_of(me) {
                if let Some(t) = modules.iter().find(|t| t.id == d) {
                    related.insert(t.id.as_str());
                }
            }
        }
        for t in &modules {
            if deps_of(t).iter().any(|d| d == m) {
                related.insert(t.id.as_str());
            }
        }
    }
    let full = modules.len() <= MAP_FULL_LIMIT;

    let mut out = String::new();
    let mut others: Vec<&str> = Vec::new();
    for m in &modules {
        if !full && !related.contains(m.id.as_str()) {
            others.push(m.title.as_str());
            continue;
        }
        let chain: Vec<TaskRow> = tasks.iter().filter(|c| c.module_id.as_deref() == Some(m.id.as_str())).cloned().collect();
        let state = state_label(module_state(m, &chain, has_reviewer));
        let deps: Vec<String> = deps_of(m).iter().filter_map(|d| title_of(d)).map(|t| format!("「{t}」")).collect();
        let deps = if deps.is_empty() { String::new() } else { format!("；相依 {}", deps.join("、")) };
        let me = if Some(m.id.as_str()) == mine { "　← 你這張卡" } else { "" };
        out.push_str(&format!("- 「{}」：{}，{}{}{}\n", m.title, owner_of(m.agent_id.as_deref()), state, deps, me));
    }
    if !others.is_empty() {
        out.push_str(&format!("- 其他模組：{}\n", others.iter().map(|t| format!("「{t}」")).collect::<Vec<_>>().join("、")));
    }
    out
}

/// 放在卡片內容前面的一整段。簡介與地圖都沒有時是空字串。
pub fn context_section(brief: Option<&str>, map: &str) -> String {
    let brief = brief.map(str::trim).filter(|b| !b.is_empty());
    if brief.is_none() && map.trim().is_empty() {
        return String::new();
    }
    let mut s = String::from(
        "## 專案背景\n（整個專案的背景，給你參考。只做你這張卡負責的部分，別人負責的模組不要動；需要別人的東西時照下面的分工與相依去用。）\n",
    );
    if let Some(b) = brief {
        let b: String = if b.chars().count() > BRIEF_PROMPT_LIMIT {
            format!("{}\n（專案簡介太長，已截斷）", b.chars().take(BRIEF_PROMPT_LIMIT).collect::<String>())
        } else {
            b.to_string()
        };
        s.push_str(&format!("\n### 專案簡介\n{b}\n"));
    }
    if !map.trim().is_empty() {
        s.push_str(&format!("\n### 團隊分工\n{map}"));
    }
    s.push_str("\n---\n\n");
    s
}

/// 派工時用：讀簡介、卡片與 Agent，組出專案背景。讀不到就略過（不擋派工）。
pub async fn project_context(pool: &SqlitePool, task: &TaskRow) -> String {
    let loaded = async {
        let brief = get_brief(pool).await?;
        let tasks = crate::tasks::store::list_tasks(pool).await?;
        let agents = crate::tasks::agents::list_agents(pool).await?;
        Ok::<_, sqlx::Error>((brief, tasks, agents))
    }
    .await;
    match loaded {
        Ok((brief, tasks, agents)) => context_section(brief.as_deref(), &team_map(task, &tasks, &agents)),
        Err(e) => {
            eprintln!("project context for {}: {e}", task.id);
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::workflow::tests::test_row;

    fn agent(id: &str, kind: &str, name: &str) -> AgentRow {
        AgentRow {
            id: id.into(), kind: kind.into(), name: name.into(), role: String::new(), prompt_prefix: String::new(),
            project_dir: "/r".into(), canvas_x: 0.0, canvas_y: 0.0, worktree_path: None, branch: None,
            paused_task_id: None, created_at: String::new(), last_merge_signature: None, produces_spec: false,
            model: None, cli: "claude".into(),
        }
    }

    fn module(id: &str, title: &str, owner: &str, order: f64) -> TaskRow {
        TaskRow { title: title.into(), agent_id: Some(owner.into()), sort_order: order, status: "queued".into(), outcome: None, ..test_row(id) }
    }

    #[tokio::test]
    async fn the_brief_round_trips_and_blank_clears_it() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        assert_eq!(get_brief(&pool).await.unwrap(), None);
        set_brief(&pool, Some("  番茄鐘網頁 App，React＋Vite  ")).await.unwrap();
        assert_eq!(get_brief(&pool).await.unwrap().as_deref(), Some("番茄鐘網頁 App，React＋Vite"));
        set_brief(&pool, Some("改過")).await.unwrap();
        assert_eq!(get_brief(&pool).await.unwrap().as_deref(), Some("改過"));
        set_brief(&pool, Some("   ")).await.unwrap();
        assert_eq!(get_brief(&pool).await.unwrap(), None);
    }

    #[test]
    fn the_map_lists_owner_state_and_dependencies_and_marks_this_card() {
        let agents = vec![agent("c1", "coder", "工程師1"), agent("c2", "coder", "工程師2")];
        let timer = module("m1", "計時器", "c1", 1.0);
        let stats = TaskRow { depends_on: Some(r#"["m1"]"#.into()), ..module("m2", "統計頁", "c2", 2.0) };
        let lib = TaskRow { agent_id: None, status: "planning".into(), ..test_row("lib") }; // 還在模組庫的不列
        let review = TaskRow { card_kind: KIND_REVIEW.into(), module_id: Some("m1".into()), agent_id: Some("r".into()), ..test_row("rv") };
        let tasks = vec![timer.clone(), stats.clone(), lib, review];

        let map = team_map(&stats, &tasks, &agents);
        assert_eq!(
            map,
            "- 「計時器」：工程師1，排隊中\n- 「統計頁」：工程師2，排隊中；相依 「計時器」　← 你這張卡\n",
        );
        // 修正卡與審查卡標的是它的模組。
        let fix = TaskRow { card_kind: KIND_FIX.into(), module_id: Some("m1".into()), ..test_row("f") };
        assert!(team_map(&fix, &tasks, &agents).contains("「計時器」：工程師1，排隊中　← 你這張卡"));
        assert!(team_map(&tasks[3], &tasks, &agents).contains("「計時器」：工程師1，排隊中　← 你這張卡"), "審查卡");
        // 合併卡沒有自己的模組，只看分工。
        let merge = TaskRow { card_kind: "merge".into(), ..test_row("mg") };
        assert!(!team_map(&merge, &tasks, &agents).contains("你這張卡"));
        assert_eq!(team_map(&merge, &[], &agents), "");
    }

    #[test]
    fn many_modules_keep_full_rows_only_for_related_ones() {
        let agents = vec![agent("c1", "coder", "工程師1")];
        let mut tasks: Vec<TaskRow> = (0..MAP_FULL_LIMIT + 3).map(|i| module(&format!("m{i}"), &format!("模組{i}"), "c1", i as f64)).collect();
        tasks[5].depends_on = Some(r#"["m2"]"#.into()); // 我（m5）的前置是 m2
        tasks[9].depends_on = Some(r#"["m5"]"#.into()); // m9 以我為前置
        let me = tasks[5].clone();
        let map = team_map(&me, &tasks, &agents);
        assert_eq!(map.lines().filter(|l| l.contains("：工程師1")).count(), 3, "{map}");
        for t in ["「模組2」：", "「模組5」：", "「模組9」："] {
            assert!(map.contains(t), "{t} 要完整列出：{map}");
        }
        let others = map.lines().last().unwrap();
        assert!(others.starts_with("- 其他模組：「模組0」、「模組1」、「模組3」"), "{others}");
        assert!(!others.contains("「模組5」"));
        // 剛好在上限內就全部列出。
        let few: Vec<TaskRow> = tasks.iter().take(MAP_FULL_LIMIT).cloned().collect();
        assert!(!team_map(&few[0], &few, &agents).contains("其他模組"));
    }

    #[test]
    fn the_section_combines_brief_and_map_and_caps_the_brief() {
        assert_eq!(context_section(None, ""), "");
        assert_eq!(context_section(Some("  "), "  "), "");
        let s = context_section(Some("番茄鐘"), "- 「計時器」：工程師1，排隊中\n");
        assert!(s.starts_with("## 專案背景\n"), "{s}");
        assert!(s.contains("別人負責的模組不要動"));
        assert!(s.contains("### 專案簡介\n番茄鐘\n"));
        assert!(s.contains("### 團隊分工\n- 「計時器」"));
        assert!(s.ends_with("\n---\n\n"));
        assert!(!context_section(None, "- x\n").contains("專案簡介"));
        assert!(!context_section(Some("b"), "").contains("團隊分工"));

        let long = "字".repeat(BRIEF_PROMPT_LIMIT + 10);
        let s = context_section(Some(&long), "");
        assert_eq!(s.matches('字').count(), BRIEF_PROMPT_LIMIT);
        assert!(s.contains("已截斷"));
    }
}
