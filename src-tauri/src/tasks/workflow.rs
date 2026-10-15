//! Agent 協作第二期：卡片結案後的自動交接（審查 → 退回修正 → 合併）。
//! 見 `docs/superpowers/specs/2026-10-15-agent-canvas-phase2-design.md`。
//!
//! 模組鏈是一條直線：模組（work）→ 審查（review_of = 模組）→ 修正（review_of =
//! 那張審查）→ 審查（review_of = 修正）→ …。「重試」把同一張卡重新排回佇列
//! 而不是複製，所以每張卡最多只有一個後繼，鏈的尾端就是模組目前的狀態。

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Serialize;
use sqlx::SqlitePool;

use crate::tasks::agents::{self, AgentRow};
use crate::tasks::store::{self, TaskRow, STATUS_DONE, STATUS_PLANNING, STATUS_QUEUED, STATUS_RUNNING};

/// 同一個模組最多退回幾輪。第 `MAX_FIX_ROUNDS + 1` 次不通過就停下來等人。
pub const MAX_FIX_ROUNDS: usize = 2;

pub const KIND_WORK: &str = "work";
pub const KIND_REVIEW: &str = "review";
pub const KIND_FIX: &str = "fix";
pub const KIND_MERGE: &str = "merge";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleState {
    Planning,
    Queued,
    Coding,
    FixQueued,
    AwaitingReview,
    Reviewing,
    Passed,
    /// 不通過超過上限，等使用者處理。
    Attention,
    /// Coder 自己那一輪失敗（Agent 已暫停）。
    Failed,
    /// 審查卡本身失敗（沒有產出 review.json、卡住…），Reviewer 已暫停。
    ReviewFailed,
}

/// 沿著鏈走到尾端。`chain` 是 `module_id == module.id` 的卡片（順序不拘）。
fn walk<'a>(module: &'a TaskRow, chain: &'a [TaskRow]) -> (Vec<&'a TaskRow>, usize) {
    let mut path = vec![module];
    let mut fails = 0usize;
    let mut cur = module;
    loop {
        let Some(review) = chain.iter().find(|c| c.card_kind == KIND_REVIEW && c.review_of.as_deref() == Some(cur.id.as_str())) else {
            break;
        };
        path.push(review);
        if review.verdict.as_deref() != Some("fail") {
            break;
        }
        fails += 1;
        let Some(fix) = chain.iter().find(|c| c.card_kind == KIND_FIX && c.review_of.as_deref() == Some(review.id.as_str())) else {
            break;
        };
        path.push(fix);
        cur = fix;
    }
    (path, fails)
}

/// 模組目前的狀態。`has_reviewer`：專案現在有沒有 Reviewer——修正卡做完但還
/// 沒有審查卡時，有 Reviewer 代表審查卡馬上就會建立（等待審查），沒有就算通過。
pub fn module_state(module: &TaskRow, chain: &[TaskRow], has_reviewer: bool) -> ModuleState {
    let (path, fails) = walk(module, chain);
    let tip = *path.last().expect("path always holds the module");
    let is_work = tip.card_kind != KIND_REVIEW;

    if is_work {
        match tip.status.as_str() {
            STATUS_RUNNING => return ModuleState::Coding,
            STATUS_QUEUED => {
                return if tip.card_kind == KIND_FIX { ModuleState::FixQueued } else { ModuleState::Queued }
            }
            STATUS_PLANNING => return ModuleState::Planning,
            _ => {}
        }
    } else {
        match tip.status.as_str() {
            STATUS_RUNNING => return ModuleState::Reviewing,
            STATUS_QUEUED => return ModuleState::AwaitingReview,
            _ => {}
        }
    }
    if module.module_override {
        return ModuleState::Passed;
    }
    if is_work {
        if tip.outcome.as_deref() != Some("success") {
            return ModuleState::Failed;
        }
        // 模組本身做完、從來沒有審查 → 完成時沒有 Reviewer，算通過。
        if tip.card_kind == KIND_WORK || !has_reviewer {
            return ModuleState::Passed;
        }
        return ModuleState::AwaitingReview;
    }
    match tip.verdict.as_deref() {
        Some("pass") => ModuleState::Passed,
        Some("fail") if fails > MAX_FIX_ROUNDS => ModuleState::Attention,
        Some("fail") => ModuleState::FixQueued,
        _ => ModuleState::ReviewFailed,
    }
}

/// 交接過程中值得通知使用者的事。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowEvent {
    Attention { module_id: String, module_title: String },
}

/// `<專案>/tasks/<review_id>/review.json`——Reviewer 寫判定的位置。
pub fn review_json_path(project_path: &Path, review_id: &str) -> std::path::PathBuf {
    crate::tasks::task_dir(project_path, review_id).join("review.json")
}

fn reasons_list(review: &TaskRow) -> Vec<String> {
    review
        .review_reasons
        .as_deref()
        .and_then(|r| serde_json::from_str::<Vec<String>>(r).ok())
        .unwrap_or_default()
}

pub fn review_body(module: &TaskRow, reviewed: &TaskRow, json_path: &Path, previous: Option<&TaskRow>) -> String {
    let mut s = format!(
        "請審查模組「{title}」的程式碼變更。\n\
         - 目前目錄已切到受審的版本 {end}（唯讀用途，不要修改任何檔案）\n\
         - 變更範圍：請先執行 git diff {start}..{end} 看完整 diff\n\
         \n模組需求：\n{req}\n",
        title = module.title,
        start = reviewed.start_sha.as_deref().unwrap_or("HEAD~1"),
        end = reviewed.end_sha.as_deref().unwrap_or("HEAD"),
        req = module.body,
    );
    if let Some(prev) = previous {
        s.push_str("\n這是修正後的再審查。上一輪的審查意見：\n");
        for r in reasons_list(prev) {
            s.push_str(&format!("- {r}\n"));
        }
    }
    s.push_str(&format!(
        "\n判定規則：只有正確性錯誤或與需求不符才判定 fail；風格建議寫進 reasons，但仍判定 pass。\n\
         審查完成後，把結果寫成 JSON 檔到：{path}\n\
         格式：{{\"verdict\": \"pass\" 或 \"fail\", \"reasons\": [\"…\"]}}\n",
        path = json_path.display(),
    ));
    s
}

pub fn fix_body(module: &TaskRow, review: &TaskRow, round: usize) -> String {
    let mut s = format!(
        "審查沒有通過，請修正模組「{title}」（第 {round} 輪修正，最多 {max} 輪）。\n審查意見：\n",
        title = module.title,
        max = MAX_FIX_ROUNDS,
    );
    let reasons = reasons_list(review);
    if reasons.is_empty() {
        s.push_str("- （審查沒有提供細節）\n");
    }
    for r in reasons {
        s.push_str(&format!("- {r}\n"));
    }
    s.push_str(&format!("\n原始需求：\n{}\n", module.body));
    s
}

pub fn merge_body(entries: &[(String, String, Vec<String>)]) -> String {
    let mut s = String::from(
        "你是統籌合併者。請在目前目錄（整合分支）依序合併以下分支：\n",
    );
    for (name, branch, modules) in entries {
        s.push_str(&format!("- {branch}（{name}：{}）\n", modules.join("、")));
    }
    s.push_str(
        "\n每合併一個分支就處理衝突：先理解雙方的意圖再解決，然後 commit。\n\
         全部合併完成後，如果專案有測試就跑一次。不要 push。\n",
    );
    s
}

struct NewCard<'a> {
    title: &'a str,
    body: &'a str,
    project_dir: &'a str,
    agent_id: &'a str,
    card_kind: &'a str,
    module_id: Option<&'a str>,
    review_of: Option<&'a str>,
    start_sha: Option<&'a str>,
    end_sha: Option<&'a str>,
    sort_order: f64,
}

async fn insert_card(pool: &SqlitePool, c: &NewCard<'_>) -> Result<String, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO tasks (id, title, body, project_dir, status, parallel_ok, interactive, sort_order,
                            agent_id, card_kind, module_id, review_of, start_sha, end_sha)
         VALUES (?, ?, ?, ?, 'queued', 1, 0, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(c.title)
    .bind(c.body)
    .bind(c.project_dir)
    .bind(c.sort_order)
    .bind(c.agent_id)
    .bind(c.card_kind)
    .bind(c.module_id)
    .bind(c.review_of)
    .bind(c.start_sha)
    .bind(c.end_sha)
    .execute(pool)
    .await?;
    Ok(id)
}

async fn queued_tail(pool: &SqlitePool) -> Result<f64, sqlx::Error> {
    sqlx::query_scalar("SELECT CAST(COALESCE(MAX(sort_order), 0) + 1 AS REAL) FROM tasks WHERE status = 'queued'")
        .fetch_one(pool)
        .await
}

fn find_kind<'a>(all: &'a [AgentRow], kind: &str) -> Option<&'a AgentRow> {
    all.iter().find(|a| a.kind == kind)
}

/// 一張 Agent 卡片結案（`finish_task` 之後）時呼叫：建立後續卡片。
pub async fn after_card_finished(
    pool: &SqlitePool,
    project_path: &Path,
    task_id: &str,
) -> Result<Vec<WorkflowEvent>, sqlx::Error> {
    let mut events = Vec::new();
    let Some(task) = store::get_task(pool, task_id).await? else {
        return Ok(events);
    };
    if task.agent_id.is_none() || task.status != STATUS_DONE {
        return Ok(events);
    }
    let all_agents = agents::list_agents(pool).await?;
    let success = task.outcome.as_deref() == Some("success");

    match task.card_kind.as_str() {
        KIND_WORK | KIND_FIX if success => {
            let module = if task.card_kind == KIND_FIX {
                match task.module_id.as_deref() {
                    Some(mid) => store::get_task(pool, mid).await?,
                    None => None,
                }
            } else {
                Some(task.clone())
            };
            if let (Some(module), Some(reviewer)) = (module, find_kind(&all_agents, "reviewer")) {
                let previous = match task.review_of.as_deref() {
                    Some(rid) => store::get_task(pool, rid).await?,
                    None => None,
                };
                // 先插入拿到 id，才知道 review.json 放哪；body 隨後補上。
                let title = if task.card_kind == KIND_FIX {
                    format!("審查：{}（修正後）", module.title)
                } else {
                    format!("審查：{}", module.title)
                };
                let id = insert_card(
                    pool,
                    &NewCard {
                        title: &title,
                        body: "",
                        project_dir: &module.project_dir,
                        agent_id: &reviewer.id,
                        card_kind: KIND_REVIEW,
                        module_id: Some(&module.id),
                        review_of: Some(&task.id),
                        start_sha: task.start_sha.as_deref(),
                        end_sha: task.end_sha.as_deref(),
                        sort_order: queued_tail(pool).await?,
                    },
                )
                .await?;
                let body = review_body(&module, &task, &review_json_path(project_path, &id), previous.as_ref());
                sqlx::query("UPDATE tasks SET body = ? WHERE id = ?").bind(&body).bind(&id).execute(pool).await?;
            }
        }
        KIND_REVIEW if success && task.verdict.as_deref() == Some("fail") => {
            let Some(mid) = task.module_id.as_deref() else { return Ok(events) };
            let Some(module) = store::get_task(pool, mid).await? else { return Ok(events) };
            let fails: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM tasks WHERE module_id = ? AND card_kind = 'review' AND verdict = 'fail'",
            )
            .bind(mid)
            .fetch_one(pool)
            .await?;
            let fails = fails as usize;
            if fails > MAX_FIX_ROUNDS {
                events.push(WorkflowEvent::Attention { module_id: module.id.clone(), module_title: module.title.clone() });
            } else if let Some(coder) = module.agent_id.as_deref() {
                let order = match agents::first_queued_order(pool, coder).await? {
                    Some(first) => first - 1.0,
                    None => queued_tail(pool).await?,
                };
                let title = format!("修正：{}（第 {fails} 輪）", module.title);
                let body = fix_body(&module, &task, fails);
                insert_card(
                    pool,
                    &NewCard {
                        title: &title,
                        body: &body,
                        project_dir: &module.project_dir,
                        agent_id: coder,
                        card_kind: KIND_FIX,
                        module_id: Some(&module.id),
                        review_of: Some(&task.id),
                        start_sha: None,
                        end_sha: None,
                        sort_order: order,
                    },
                )
                .await?;
            }
        }
        _ => {}
    }

    maybe_enqueue_merge(pool).await?;
    Ok(events)
}

/// 所有模組都通過、而且沒有任何 Coder／Reviewer 還在忙時，替 Orchestrator
/// 建一張合併卡。同一組模組版本只建一次（`last_merge_signature`）。
/// 回傳新卡的 id。
pub async fn maybe_enqueue_merge(pool: &SqlitePool) -> Result<Option<String>, sqlx::Error> {
    let all_agents = agents::list_agents(pool).await?;
    let Some(orch) = find_kind(&all_agents, "orchestrator") else { return Ok(None) };
    if orch.paused_task_id.is_some() {
        return Ok(None);
    }
    let tasks = store::list_tasks(pool).await?;
    let busy = |aid: &str| {
        tasks.iter().any(|t| {
            t.agent_id.as_deref() == Some(aid) && (t.status == STATUS_QUEUED || t.status == STATUS_RUNNING)
        })
    };
    if busy(&orch.id) {
        return Ok(None);
    }
    // Coder／Reviewer 手上的卡一定屬於某個還沒通過的模組（work/fix 卡是模組
    // 鏈的一環，review 卡也是），所以下面「每個模組都 Passed」就已經涵蓋了
    // 「沒有人在忙」，不需要另外檢查。
    let coders: Vec<&AgentRow> = all_agents.iter().filter(|a| a.kind == "coder").collect();
    let reviewer = find_kind(&all_agents, "reviewer");
    let coder_ids: HashSet<&str> = coders.iter().map(|c| c.id.as_str()).collect();
    let modules: Vec<&TaskRow> = tasks
        .iter()
        .filter(|t| t.card_kind == KIND_WORK && t.agent_id.as_deref().is_some_and(|a| coder_ids.contains(a)))
        .collect();
    if modules.is_empty() {
        return Ok(None);
    }
    let mut chains: HashMap<&str, Vec<TaskRow>> = HashMap::new();
    for t in &tasks {
        if let Some(mid) = t.module_id.as_deref() {
            chains.entry(mid).or_default().push(t.clone());
        }
    }
    for m in &modules {
        let chain = chains.get(m.id.as_str()).map(Vec::as_slice).unwrap_or(&[]);
        if module_state(m, chain, reviewer.is_some()) != ModuleState::Passed {
            return Ok(None);
        }
    }

    // signature：每個 Coder 分支目前的 HEAD（最新成功的 work/fix 卡的 end_sha）。
    let mut entries: Vec<(String, String, Vec<String>)> = Vec::new();
    let mut sig_parts: Vec<String> = Vec::new();
    for c in &coders {
        let mine: Vec<&&TaskRow> = modules.iter().filter(|m| m.agent_id.as_deref() == Some(c.id.as_str())).collect();
        if mine.is_empty() {
            continue;
        }
        let head = tasks
            .iter()
            .filter(|t| {
                t.agent_id.as_deref() == Some(c.id.as_str())
                    && (t.card_kind == KIND_WORK || t.card_kind == KIND_FIX)
                    && t.outcome.as_deref() == Some("success")
            })
            .max_by_key(|t| t.finished_at.unwrap_or(0))
            .and_then(|t| t.end_sha.clone())
            .unwrap_or_default();
        sig_parts.push(format!("{}:{head}", c.id));
        let branch = c.branch.clone().unwrap_or_else(|| agents::branch_name(&c.id));
        entries.push((c.name.clone(), branch, mine.iter().map(|m| m.title.clone()).collect()));
    }
    sig_parts.sort();
    let signature = sig_parts.join(";");
    if orch.last_merge_signature.as_deref() == Some(signature.as_str()) {
        return Ok(None);
    }
    let body = merge_body(&entries);
    let title = format!("合併 {} 個 Coder 分支", entries.len());
    let id = insert_card(
        pool,
        &NewCard {
            title: &title,
            body: &body,
            project_dir: &orch.project_dir,
            agent_id: &orch.id,
            card_kind: KIND_MERGE,
            module_id: None,
            review_of: None,
            start_sha: None,
            end_sha: None,
            sort_order: queued_tail(pool).await?,
        },
    )
    .await?;
    sqlx::query("UPDATE agents SET last_merge_signature = ? WHERE id = ?")
        .bind(&signature)
        .bind(&orch.id)
        .execute(pool)
        .await?;
    Ok(Some(id))
}

/// review.json 的內容。`verdict` 只接受 pass／fail。
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct ReviewVerdict {
    pub verdict: String,
    #[serde(default)]
    pub reasons: Vec<String>,
}

pub fn parse_review_json(text: &str) -> Result<ReviewVerdict, String> {
    let v: ReviewVerdict = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    match v.verdict.as_str() {
        "pass" | "fail" => Ok(v),
        other => Err(format!("verdict 必須是 pass 或 fail，收到 {other}")),
    }
}

pub async fn set_verdict(pool: &SqlitePool, task_id: &str, v: &ReviewVerdict) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tasks SET verdict = ?, review_reasons = ? WHERE id = ?")
        .bind(&v.verdict)
        .bind(serde_json::to_string(&v.reasons).unwrap_or_else(|_| "[]".into()))
        .bind(task_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 使用者手動放行一個卡住的模組。只接受 attention／review_failed。
pub async fn override_module(pool: &SqlitePool, module_id: &str) -> Result<(), sqlx::Error> {
    let module = store::get_task(pool, module_id).await?.ok_or(sqlx::Error::RowNotFound)?;
    if module.card_kind != KIND_WORK {
        return Err(sqlx::Error::Protocol("只能放行模組卡".into()));
    }
    let chain: Vec<TaskRow> = store::list_tasks(pool)
        .await?
        .into_iter()
        .filter(|t| t.module_id.as_deref() == Some(module_id))
        .collect();
    let has_reviewer = agents::list_agents(pool).await?.iter().any(|a| a.kind == "reviewer");
    match module_state(&module, &chain, has_reviewer) {
        ModuleState::Attention | ModuleState::ReviewFailed => {}
        other => return Err(sqlx::Error::Protocol(format!("這個模組目前不需要放行（{other:?}）"))),
    }
    sqlx::query("UPDATE tasks SET module_override = 1 WHERE id = ?").bind(module_id).execute(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mem_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    async fn agent(pool: &SqlitePool, kind: &str, name: &str) -> String {
        agents::create_agent(
            pool,
            &agents::NewAgent {
                kind, name, role: "r", prompt_prefix: "", project_dir: "/repo",
                canvas_x: 0.0, canvas_y: 0.0,
            },
        )
        .await
        .unwrap()
    }

    /// 一張指派給 coder、跑完成功、有 sha 範圍的模組卡。
    async fn done_module(pool: &SqlitePool, coder: &str, title: &str, end: &str) -> String {
        let id = store::create_task(pool, title, "做 X", "/repo", true, false).await.unwrap();
        agents::assign_task(pool, &id, Some(coder)).await.unwrap();
        store::dispatch_for_test(pool, &id, "tab").await;
        store::set_start_sha(pool, &id, "s0").await.unwrap();
        store::set_end_sha(pool, &id, end).await.unwrap();
        store::finish_task(pool, &id, "success", None, None).await.unwrap();
        id
    }

    async fn all(pool: &SqlitePool) -> Vec<TaskRow> {
        store::list_tasks(pool).await.unwrap()
    }

    /// 模擬 Reviewer 跑完一張審查卡並給出判定。
    async fn finish_review(pool: &SqlitePool, review_id: &str, verdict: &str) {
        store::dispatch_for_test(pool, review_id, "tab-r").await;
        set_verdict(pool, review_id, &ReviewVerdict { verdict: verdict.into(), reasons: vec!["理由".into()] }).await.unwrap();
        store::finish_task(pool, review_id, "success", None, None).await.unwrap();
    }

    async fn finish_fix(pool: &SqlitePool, fix_id: &str, end: &str) {
        store::dispatch_for_test(pool, fix_id, "tab-f").await;
        store::set_start_sha(pool, fix_id, "f0").await.unwrap();
        store::set_end_sha(pool, fix_id, end).await.unwrap();
        store::finish_task(pool, fix_id, "success", None, None).await.unwrap();
    }

    fn of_kind<'a>(v: &'a [TaskRow], kind: &str) -> Vec<&'a TaskRow> {
        v.iter().filter(|t| t.card_kind == kind).collect()
    }

    #[tokio::test]
    async fn a_successful_module_queues_a_review_with_its_sha_range() {
        let pool = mem_pool().await;
        let c = agent(&pool, "coder", "C1").await;
        let r = agent(&pool, "reviewer", "R").await;
        let m = done_module(&pool, &c, "A 登入", "e1").await;
        let proj = Path::new("/proj");

        after_card_finished(&pool, proj, &m).await.unwrap();

        let tasks = all(&pool).await;
        let reviews = of_kind(&tasks, KIND_REVIEW);
        assert_eq!(reviews.len(), 1);
        let rv = reviews[0];
        assert_eq!(rv.agent_id.as_deref(), Some(r.as_str()));
        assert_eq!(rv.status, STATUS_QUEUED);
        assert_eq!((rv.module_id.as_deref(), rv.review_of.as_deref()), (Some(m.as_str()), Some(m.as_str())));
        assert_eq!((rv.start_sha.as_deref(), rv.end_sha.as_deref()), (Some("s0"), Some("e1")));
        assert!(rv.body.contains("git diff s0..e1"), "{}", rv.body);
        assert!(rv.body.contains(&review_json_path(proj, &rv.id).display().to_string()));
    }

    #[tokio::test]
    async fn without_a_reviewer_nothing_is_queued_and_the_module_counts_as_passed() {
        let pool = mem_pool().await;
        let c = agent(&pool, "coder", "C1").await;
        let m = done_module(&pool, &c, "A", "e1").await;
        after_card_finished(&pool, Path::new("/p"), &m).await.unwrap();
        let tasks = all(&pool).await;
        assert_eq!(tasks.len(), 1);
        assert_eq!(module_state(&tasks[0], &[], false), ModuleState::Passed);
    }

    #[tokio::test]
    async fn a_failed_review_puts_a_fix_card_at_the_front_of_the_coders_queue() {
        let pool = mem_pool().await;
        let c = agent(&pool, "coder", "C1").await;
        agent(&pool, "reviewer", "R").await;
        let m = done_module(&pool, &c, "A", "e1").await;
        let next = store::create_task(&pool, "B", "", "/repo", true, false).await.unwrap();
        agents::assign_task(&pool, &next, Some(&c)).await.unwrap();
        after_card_finished(&pool, Path::new("/p"), &m).await.unwrap();
        let review = of_kind(&all(&pool).await, KIND_REVIEW)[0].id.clone();

        finish_review(&pool, &review, "fail").await;
        let events = after_card_finished(&pool, Path::new("/p"), &review).await.unwrap();

        assert!(events.is_empty());
        let tasks = all(&pool).await;
        let fixes = of_kind(&tasks, KIND_FIX);
        assert_eq!(fixes.len(), 1);
        let fix = fixes[0];
        assert_eq!(fix.agent_id.as_deref(), Some(c.as_str()));
        assert_eq!(fix.review_of.as_deref(), Some(review.as_str()));
        assert!(fix.body.contains("理由"));
        let b = tasks.iter().find(|t| t.id == next).unwrap();
        assert!(fix.sort_order < b.sort_order, "修正卡要插到 Coder 佇列最前面");
        let chain: Vec<TaskRow> = tasks.iter().filter(|t| t.module_id.as_deref() == Some(m.as_str())).cloned().collect();
        let module = tasks.iter().find(|t| t.id == m).unwrap();
        assert_eq!(module_state(module, &chain, true), ModuleState::FixQueued);
    }

    #[tokio::test]
    async fn a_fixed_module_is_reviewed_again_and_passes() {
        let pool = mem_pool().await;
        let c = agent(&pool, "coder", "C1").await;
        agent(&pool, "reviewer", "R").await;
        let m = done_module(&pool, &c, "A", "e1").await;
        let p = Path::new("/p");
        after_card_finished(&pool, p, &m).await.unwrap();
        let r1 = of_kind(&all(&pool).await, KIND_REVIEW)[0].id.clone();
        finish_review(&pool, &r1, "fail").await;
        after_card_finished(&pool, p, &r1).await.unwrap();
        let fix = of_kind(&all(&pool).await, KIND_FIX)[0].id.clone();

        finish_fix(&pool, &fix, "e2").await;
        after_card_finished(&pool, p, &fix).await.unwrap();
        let tasks = all(&pool).await;
        let r2 = of_kind(&tasks, KIND_REVIEW).into_iter().find(|t| t.id != r1).unwrap().clone();
        assert_eq!(r2.review_of.as_deref(), Some(fix.as_str()));
        assert!(r2.body.contains("修正後的再審查"));
        assert!(r2.body.contains("git diff f0..e2"));

        finish_review(&pool, &r2.id, "pass").await;
        after_card_finished(&pool, p, &r2.id).await.unwrap();
        let tasks = all(&pool).await;
        let chain: Vec<TaskRow> = tasks.iter().filter(|t| t.module_id.as_deref() == Some(m.as_str())).cloned().collect();
        let module = tasks.iter().find(|t| t.id == m).unwrap();
        assert_eq!(module_state(module, &chain, true), ModuleState::Passed);
    }

    #[tokio::test]
    async fn the_third_failure_stops_and_raises_attention_instead_of_another_fix() {
        let pool = mem_pool().await;
        let c = agent(&pool, "coder", "C1").await;
        agent(&pool, "reviewer", "R").await;
        let m = done_module(&pool, &c, "A 登入", "e1").await;
        let p = Path::new("/p");
        after_card_finished(&pool, p, &m).await.unwrap();
        let mut last_events = vec![];
        for round in 1..=3 {
            let tasks = all(&pool).await;
            let review = of_kind(&tasks, KIND_REVIEW).into_iter().find(|t| t.status == STATUS_QUEUED).unwrap().id.clone();
            finish_review(&pool, &review, "fail").await;
            last_events = after_card_finished(&pool, p, &review).await.unwrap();
            if round < 3 {
                let fix = of_kind(&all(&pool).await, KIND_FIX).into_iter().find(|t| t.status == STATUS_QUEUED).unwrap().id.clone();
                finish_fix(&pool, &fix, &format!("e{}", round + 1)).await;
                after_card_finished(&pool, p, &fix).await.unwrap();
            }
        }
        assert_eq!(last_events, vec![WorkflowEvent::Attention { module_id: m.clone(), module_title: "A 登入".into() }]);
        let tasks = all(&pool).await;
        assert_eq!(of_kind(&tasks, KIND_FIX).len(), MAX_FIX_ROUNDS, "第三次不通過不能再建修正卡");
        let chain: Vec<TaskRow> = tasks.iter().filter(|t| t.module_id.as_deref() == Some(m.as_str())).cloned().collect();
        let module = tasks.iter().find(|t| t.id == m).unwrap();
        assert_eq!(module_state(module, &chain, true), ModuleState::Attention);

        override_module(&pool, &m).await.unwrap();
        let module = store::get_task(&pool, &m).await.unwrap().unwrap();
        assert_eq!(module_state(&module, &chain, true), ModuleState::Passed);
    }

    #[tokio::test]
    async fn override_is_refused_for_a_module_that_is_not_stuck() {
        let pool = mem_pool().await;
        let c = agent(&pool, "coder", "C1").await;
        let m = done_module(&pool, &c, "A", "e1").await;
        assert!(override_module(&pool, &m).await.is_err());
    }

    #[tokio::test]
    async fn merge_waits_for_every_module_to_pass_then_queues_once() {
        let pool = mem_pool().await;
        let c1 = agent(&pool, "coder", "Coder1").await;
        let c2 = agent(&pool, "coder", "Coder2").await;
        agent(&pool, "reviewer", "R").await;
        let o = agent(&pool, "orchestrator", "O").await;
        let p = Path::new("/p");
        let a = done_module(&pool, &c1, "A", "a1").await;
        after_card_finished(&pool, p, &a).await.unwrap();
        let d = done_module(&pool, &c2, "D", "d1").await;
        after_card_finished(&pool, p, &d).await.unwrap();
        assert!(of_kind(&all(&pool).await, KIND_MERGE).is_empty(), "審查還沒做，不能合併");

        let reviews: Vec<String> = of_kind(&all(&pool).await, KIND_REVIEW).iter().map(|t| t.id.clone()).collect();
        finish_review(&pool, &reviews[0], "pass").await;
        after_card_finished(&pool, p, &reviews[0]).await.unwrap();
        assert!(of_kind(&all(&pool).await, KIND_MERGE).is_empty(), "還有一個模組在等審查");

        finish_review(&pool, &reviews[1], "pass").await;
        after_card_finished(&pool, p, &reviews[1]).await.unwrap();
        let tasks = all(&pool).await;
        let merges = of_kind(&tasks, KIND_MERGE);
        assert_eq!(merges.len(), 1);
        assert_eq!(merges[0].agent_id.as_deref(), Some(o.as_str()));
        assert!(merges[0].body.contains(&agents::branch_name(&c1)));
        assert!(merges[0].body.contains(&agents::branch_name(&c2)));

        // 同一組版本不重複建立：合併卡做完後再觸發一次。
        store::dispatch_for_test(&pool, &merges[0].id, "tab-o").await;
        store::finish_task(&pool, &merges[0].id, "success", None, None).await.unwrap();
        assert_eq!(maybe_enqueue_merge(&pool).await.unwrap(), None);
    }

    #[tokio::test]
    async fn merge_is_not_queued_while_a_coder_still_has_work() {
        let pool = mem_pool().await;
        let c = agent(&pool, "coder", "C1").await;
        agent(&pool, "orchestrator", "O").await;
        let a = done_module(&pool, &c, "A", "a1").await;
        let b = store::create_task(&pool, "B", "", "/repo", true, false).await.unwrap();
        agents::assign_task(&pool, &b, Some(&c)).await.unwrap();
        after_card_finished(&pool, Path::new("/p"), &a).await.unwrap();
        assert!(of_kind(&all(&pool).await, KIND_MERGE).is_empty());
    }

    #[test]
    fn review_json_parsing_accepts_pass_and_fail_only() {
        assert_eq!(parse_review_json(r#"{"verdict":"pass"}"#).unwrap().reasons, Vec::<String>::new());
        assert_eq!(parse_review_json("\u{feff}{\"verdict\":\"fail\",\"reasons\":[\"x\"]}").unwrap().reasons, vec!["x"]);
        assert!(parse_review_json(r#"{"verdict":"maybe"}"#).is_err());
        assert!(parse_review_json("not json").is_err());
    }

    #[tokio::test]
    async fn a_review_card_that_failed_without_a_verdict_is_review_failed() {
        let pool = mem_pool().await;
        let c = agent(&pool, "coder", "C1").await;
        agent(&pool, "reviewer", "R").await;
        let m = done_module(&pool, &c, "A", "e1").await;
        after_card_finished(&pool, Path::new("/p"), &m).await.unwrap();
        let review = of_kind(&all(&pool).await, KIND_REVIEW)[0].id.clone();
        store::dispatch_for_test(&pool, &review, "t").await;
        store::finish_task(&pool, &review, "failed", Some("沒有 review.json"), None).await.unwrap();
        let tasks = all(&pool).await;
        let chain: Vec<TaskRow> = tasks.iter().filter(|t| t.module_id.as_deref() == Some(m.as_str())).cloned().collect();
        let module = tasks.iter().find(|t| t.id == m).unwrap();
        assert_eq!(module_state(module, &chain, true), ModuleState::ReviewFailed);
        override_module(&pool, &m).await.unwrap();
    }
}
