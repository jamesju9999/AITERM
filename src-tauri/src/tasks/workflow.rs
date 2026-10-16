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
/// Orchestrator 拆解一份通過審查的規格（見規格先行 spec）。
pub const KIND_PLAN: &str = "plan";
/// 一份規格最多拆成幾張實作卡。
pub const MAX_PLAN_TASKS: usize = 20;
/// 附在實作卡內文裡的規格全文上限（字元）。
const SPEC_APPENDIX_LIMIT: usize = 30_000;

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
    /// 規格通過審查，Orchestrator 正在（或排隊）拆解成實作卡。
    Dispatching,
    /// 拆解失敗（沒有產出有效的 tasks.json…），Orchestrator 已暫停。
    DispatchFailed,
}

/// 規格模組「通過」之前還要過拆解這一關：有拆解卡就看它的結果。沒有拆解卡
/// （沒有 Orchestrator）就照一般模組算通過。
fn spec_gate(module: &TaskRow, chain: &[TaskRow]) -> ModuleState {
    if !module.is_spec {
        return ModuleState::Passed;
    }
    match chain.iter().find(|c| c.card_kind == KIND_PLAN) {
        None => ModuleState::Passed,
        Some(p) if p.status == STATUS_QUEUED || p.status == STATUS_RUNNING => ModuleState::Dispatching,
        Some(p) if p.outcome.as_deref() == Some("success") => ModuleState::Passed,
        Some(_) => ModuleState::DispatchFailed,
    }
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
            return spec_gate(module, chain);
        }
        return ModuleState::AwaitingReview;
    }
    match tip.verdict.as_deref() {
        Some("pass") => spec_gate(module, chain),
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

/// `<專案>/tasks/<plan_id>/tasks.json`——Orchestrator 寫拆解結果的位置。
pub fn tasks_json_path(project_path: &Path, plan_id: &str) -> std::path::PathBuf {
    crate::tasks::task_dir(project_path, plan_id).join("tasks.json")
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
    // 規格與程式碼要看的東西不同：同一位審核者，依交件種類換檢核重點。
    let what = if module.is_spec { "規格文件" } else { "程式碼變更" };
    let mut s = format!(
        "請審查模組「{title}」的{what}。\n\
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
    let rule = if module.is_spec {
        "檢核重點：\n\
         - 需求是否都有涵蓋，範圍與不做的事是否寫清楚\n\
         - 資料模型、API 與介面、畫面流程是否定義到可以直接實作\n\
         - 驗收條件是否具體、可以測試\n\
         - 有沒有前後矛盾，或模糊到會擋住實作的地方（待確認事項有列出就不算）\n\
         判定規則：缺少關鍵內容、出現矛盾、或模糊到無法實作才判定 fail；用詞與排版建議寫進 reasons，但仍判定 pass。"
    } else {
        "判定規則：只有正確性錯誤或與需求不符才判定 fail；風格建議寫進 reasons，但仍判定 pass。"
    };
    s.push_str(&format!(
        "\n{rule}\n\
         審查完成後，把結果寫成 JSON 檔到：{path}\n\
         格式：{{\"verdict\": \"pass\" 或 \"fail\", \"reasons\": [\"…\"]}}\n",
        path = json_path.display(),
    ));
    s
}

/// 審查卡標題：規格與程式碼分開標示，看板與桌布上一眼看得出在審什麼。
fn review_title(module: &TaskRow, after_fix: bool) -> String {
    let what = if module.is_spec { "審查規格" } else { "審查" };
    if after_fix {
        format!("{what}：{}（修正後）", module.title)
    } else {
        format!("{what}：{}", module.title)
    }
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
        "你是協調者。請在目前目錄（整合分支）依序合併以下分支：\n",
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

pub fn plan_body(
    module: &TaskRow,
    start: Option<&str>,
    end: Option<&str>,
    implementers: &[(String, String)],
    json_path: &Path,
) -> String {
    let mut s = format!(
        "請把模組「{title}」已通過審查的規格拆解成可以分頭實作的任務。\n\
         - 目前目錄已切到規格的版本 {end}（唯讀用途，不要修改任何檔案）\n\
         - 規格的變更範圍：請先執行 git diff {start}..{end}，讀完規格文件\n\
         \n原始需求：\n{req}\n\
         \n可以接手實作的工程師（任務會自動派給最閒的一位，你不用指定）：\n",
        title = module.title,
        start = start.unwrap_or("HEAD~1"),
        end = end.unwrap_or("HEAD"),
        req = module.body,
    );
    if implementers.is_empty() {
        s.push_str("- （目前沒有）\n");
    }
    for (name, role) in implementers {
        s.push_str(&format!("- {name}（{role}）\n"));
    }
    s.push_str(&format!(
        "\n拆解原則：每個任務要能獨立實作與審查；body 要自給自足，寫清楚要做什麼、對應規格的哪個章節、驗收條件。\n\
         最多 {max} 個任務。\n\
         \n相依關係（depends_on）：每位工程師在自己的分支上做，看不到別人還沒合併的程式碼。\n\
         - 任務需要用到前面某些任務寫出來的程式碼，就在 depends_on 列出那些任務的索引（0 起算，只能指向排在它前面的任務）。\n\
         - 整合類任務（例如把各模組組裝起來）要列出它要組裝的所有模組。\n\
         - 被依賴的任務排在前面；彼此獨立的任務不要寫 depends_on，才能平行做。\n\
         系統會把有相依的任務排給同一位工程師，或等前置審查通過、合併進來之後才開工。\n\
         \n完成後把結果寫成 JSON 檔到：{path}\n\
         格式：{{\"tasks\": [{{\"title\": \"…\", \"body\": \"…\"}}, {{\"title\": \"…\", \"body\": \"…\", \"depends_on\": [0]}}]}}\n",
        max = MAX_PLAN_TASKS,
        path = json_path.display(),
    ));
    s
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct PlanTask {
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// 依賴的前面任務的索引（0 起算）。
    #[serde(default)]
    pub depends_on: Vec<usize>,
}

#[derive(serde::Deserialize)]
struct PlanFile {
    tasks: Vec<PlanTask>,
}

/// tasks.json：1 到 `MAX_PLAN_TASKS` 張、每張標題非空。
pub fn parse_plan_json(text: &str) -> Result<Vec<PlanTask>, String> {
    let f: PlanFile = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    let tasks: Vec<PlanTask> = f
        .tasks
        .into_iter()
        .map(|mut t| {
            t.depends_on.sort_unstable();
            t.depends_on.dedup();
            PlanTask { title: t.title.trim().to_string(), ..t }
        })
        .collect();
    if tasks.is_empty() {
        return Err("tasks 是空的".into());
    }
    if tasks.len() > MAX_PLAN_TASKS {
        return Err(format!("最多 {MAX_PLAN_TASKS} 個任務，收到 {}", tasks.len()));
    }
    if tasks.iter().any(|t| t.title.is_empty()) {
        return Err("有任務沒有標題".into());
    }
    for (i, t) in tasks.iter().enumerate() {
        if let Some(bad) = t.depends_on.iter().find(|&&d| d >= i) {
            return Err(format!(
                "第 {i} 個任務「{}」的 depends_on 有 {bad}：只能指向排在它前面的任務（0 到 {}）",
                t.title,
                i as isize - 1
            ));
        }
    }
    Ok(tasks)
}

/// 規格範圍內新增／修改的檔案在 `end` 的內容，串成一段附在實作卡內文。
/// 實作的 Coder 在自己的分支上，看不到規格撰寫師分支上的檔案，所以要附全文。
async fn spec_appendix(repo_dir: &str, start: Option<&str>, end: Option<&str>) -> String {
    let (Some(start), Some(end)) = (start, end) else { return String::new() };
    let client = crate::vcs::git::GitClient::new(repo_dir.to_string(), None);
    let files = match client.changed_files(start, end).await {
        Ok(f) => f,
        Err(e) => return format!("\n\n（讀取規格檔案失敗：{e}）\n"),
    };
    let mut out = String::from("\n\n## 規格全文\n");
    for f in files {
        let Ok(text) = client.show_file(end, &f).await else { continue };
        out.push_str(&format!("\n### {f}\n\n{text}\n"));
        if out.chars().count() > SPEC_APPENDIX_LIMIT {
            let cut: String = out.chars().take(SPEC_APPENDIX_LIMIT).collect();
            return format!("{cut}\n\n（規格太長，已截斷；完整內容在 {end} 的 {f} 等檔案）\n");
        }
    }
    out
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
    from_spec: Option<&'a str>,
}

async fn insert_card(pool: &SqlitePool, c: &NewCard<'_>) -> Result<String, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO tasks (id, title, body, project_dir, status, parallel_ok, interactive, sort_order,
                            agent_id, card_kind, module_id, review_of, start_sha, end_sha, from_spec)
         VALUES (?, ?, ?, ?, 'queued', 1, 0, ?, ?, ?, ?, ?, ?, ?, ?)",
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
    .bind(c.from_spec)
    .execute(pool)
    .await?;
    Ok(id)
}

/// 建一張拆解卡給 Orchestrator。`start/end` 是通過審查的規格版本範圍。
async fn create_plan_card(
    pool: &SqlitePool,
    project_path: &Path,
    module: &TaskRow,
    orch: &AgentRow,
    all_agents: &[AgentRow],
    start: Option<&str>,
    end: Option<&str>,
) -> Result<String, sqlx::Error> {
    let implementers: Vec<(String, String)> = all_agents
        .iter()
        .filter(|a| a.kind == "coder" && !a.produces_spec)
        .map(|a| (a.name.clone(), a.role.clone()))
        .collect();
    let title = format!("拆解規格：{}", module.title);
    let id = insert_card(
        pool,
        &NewCard {
            title: &title,
            body: "",
            project_dir: &module.project_dir,
            agent_id: &orch.id,
            card_kind: KIND_PLAN,
            module_id: Some(&module.id),
            review_of: None,
            start_sha: start,
            end_sha: end,
            sort_order: queued_tail(pool).await?,
            from_spec: None,
        },
    )
    .await?;
    let body = plan_body(module, start, end, &implementers, &tasks_json_path(project_path, &id));
    sqlx::query("UPDATE tasks SET body = ? WHERE id = ?").bind(&body).bind(&id).execute(pool).await?;
    Ok(id)
}

/// 依「最閒」挑 Coder：queued+running 卡數最少，再比名下 work 卡總數，再比
/// 建立順序。產出規格的 Coder 不參與。`load` 由呼叫端持有，每派一張就 +1，
/// 同一次拆出的多張卡才會平均分散。
fn pick_least_busy<'a>(
    candidates: &[&'a AgentRow],
    load: &HashMap<String, (usize, usize)>,
) -> Option<&'a AgentRow> {
    candidates.iter().copied().min_by(|a, b| {
        let la = load.get(&a.id).copied().unwrap_or((0, 0));
        let lb = load.get(&b.id).copied().unwrap_or((0, 0));
        la.cmp(&lb).then(a.created_at.cmp(&b.created_at)).then(a.id.cmp(&b.id))
    })
}

/// 拆解卡成功：讀 tasks.json，建實作卡並派給最閒的 Coder。沒有可實作的
/// Coder 時建在「計畫中」並回報 Attention。
async fn dispatch_plan(
    pool: &SqlitePool,
    project_path: &Path,
    plan: &TaskRow,
    all_agents: &[AgentRow],
) -> Result<Vec<WorkflowEvent>, sqlx::Error> {
    let mut events = Vec::new();
    let Some(mid) = plan.module_id.as_deref() else { return Ok(events) };
    let Some(module) = store::get_task(pool, mid).await? else { return Ok(events) };
    let tasks = match std::fs::read_to_string(tasks_json_path(project_path, &plan.id))
        .map_err(|e| e.to_string())
        .and_then(|t| parse_plan_json(&t))
    {
        Ok(t) => t,
        Err(e) => {
            // finalize 已經驗證過；走到這裡代表檔案在中間被動過。不建卡、提醒使用者。
            eprintln!("dispatch_plan {}: {e}", plan.id);
            events.push(WorkflowEvent::Attention { module_id: module.id.clone(), module_title: module.title.clone() });
            return Ok(events);
        }
    };
    let appendix = spec_appendix(&module.project_dir, plan.start_sha.as_deref(), plan.end_sha.as_deref()).await;

    let candidates: Vec<&AgentRow> = all_agents.iter().filter(|a| a.kind == "coder" && !a.produces_spec).collect();
    let current = store::list_tasks(pool).await?;
    let mut load: HashMap<String, (usize, usize)> = HashMap::new();
    for c in &candidates {
        let busy = current
            .iter()
            .filter(|t| t.agent_id.as_deref() == Some(c.id.as_str()) && (t.status == STATUS_QUEUED || t.status == STATUS_RUNNING))
            .count();
        let total = current
            .iter()
            .filter(|t| t.agent_id.as_deref() == Some(c.id.as_str()) && t.card_kind == KIND_WORK)
            .count();
        load.insert(c.id.clone(), (busy, total));
    }

    // 第 i 個任務建出來的卡：(id, 負責人)。相依的索引靠它換成 id、找出前置的擁有者。
    let mut created: Vec<(String, Option<String>)> = Vec::new();
    for t in tasks {
        let body = format!(
            "{}\n\n（這是從規格「{}」拆出來的任務；規格文件也一併附在下面。）{}",
            t.body, module.title, appendix
        );
        let deps: Vec<&(String, Option<String>)> = t.depends_on.iter().filter_map(|&i| created.get(i)).collect();
        let depends_on = (!deps.is_empty())
            .then(|| serde_json::to_string(&deps.iter().map(|(id, _)| id).collect::<Vec<_>>()).unwrap_or_default());
        // 有相依：交給擁有最多個前置的工程師（同一條分支上接著做）；平手比忙碌程度。
        let mut owned: HashMap<&str, usize> = HashMap::new();
        for (_, o) in &deps {
            if let Some(o) = o.as_deref() {
                *owned.entry(o).or_default() += 1;
            }
        }
        let pick = match owned.values().max() {
            Some(&most) => {
                let owners: Vec<&AgentRow> =
                    candidates.iter().copied().filter(|c| owned.get(c.id.as_str()) == Some(&most)).collect();
                pick_least_busy(&owners, &load).or_else(|| pick_least_busy(&candidates, &load))
            }
            None => pick_least_busy(&candidates, &load),
        };
        match pick {
            Some(coder) => {
                let id = insert_card(
                    pool,
                    &NewCard {
                        title: &t.title,
                        body: &body,
                        project_dir: &coder.project_dir,
                        agent_id: &coder.id,
                        card_kind: KIND_WORK,
                        module_id: None,
                        review_of: None,
                        start_sha: None,
                        end_sha: None,
                        sort_order: queued_tail(pool).await?,
                        from_spec: Some(&module.id),
                    },
                )
                .await?;
                sqlx::query("UPDATE tasks SET depends_on = ? WHERE id = ?").bind(&depends_on).bind(&id).execute(pool).await?;
                let e = load.entry(coder.id.clone()).or_insert((0, 0));
                e.0 += 1;
                e.1 += 1;
                created.push((id, Some(coder.id.clone())));
            }
            None => {
                let id = store::create_task(pool, &t.title, &body, &module.project_dir, true, false).await?;
                sqlx::query("UPDATE tasks SET from_spec = ?, depends_on = ? WHERE id = ?")
                    .bind(&module.id)
                    .bind(&depends_on)
                    .bind(&id)
                    .execute(pool)
                    .await?;
                created.push((id, None));
            }
        }
    }
    if candidates.is_empty() {
        events.push(WorkflowEvent::Attention { module_id: module.id.clone(), module_title: module.title.clone() });
    }
    Ok(events)
}

/// 閒著的工程師接走別人還沒開始的實作卡。拆解當下才分派，之後才加入（或先做完）
/// 的工程師原本只能乾等——實機上 17 張全堆在一位工程師身上。
///
/// - 只搬「規格拆出來」（`from_spec`）且還在排隊的 work 卡：使用者手動指派的
///   模組代表明確意圖，不動。
/// - 接手者：沒暫停、不產出規格、手上沒有 queued/running 卡的工程師。
/// - 來源：手上 queued+running 最多、且至少 2 張的工程師（只剩一張又閒著就讓他自己做）。
///   搬它佇列裡最前面那張，盡量維持拆解時的先後順序。
///
/// 回傳搬了幾張。每個接手者一輪最多接一張。
pub async fn rebalance_idle_coders(pool: &SqlitePool) -> Result<usize, sqlx::Error> {
    let coders: Vec<AgentRow> = agents::list_agents(pool)
        .await?
        .into_iter()
        .filter(|a| a.kind == "coder" && !a.produces_spec)
        .collect();
    if coders.len() < 2 {
        return Ok(0);
    }
    let paused: HashSet<String> = agents::paused_agent_ids(pool).await?.into_iter().collect();
    let mut tasks = store::list_tasks(pool).await?;
    let active = |tasks: &[TaskRow], id: &str| {
        tasks
            .iter()
            .filter(|t| t.agent_id.as_deref() == Some(id) && (t.status == STATUS_QUEUED || t.status == STATUS_RUNNING))
            .count()
    };
    let mut moved = 0;
    for thief in coders.iter().filter(|a| !paused.contains(&a.id)) {
        if active(&tasks, &thief.id) > 0 {
            continue;
        }
        let Some(victim) = coders
            .iter()
            .filter(|v| v.id != thief.id && active(&tasks, &v.id) >= 2)
            .max_by_key(|v| active(&tasks, &v.id))
        else {
            continue;
        };
        let mut queue: Vec<&TaskRow> = tasks
            .iter()
            .filter(|t| t.agent_id.as_deref() == Some(victim.id.as_str()) && t.status == STATUS_QUEUED)
            .collect();
        queue.sort_by(|a, b| a.sort_order.partial_cmp(&b.sort_order).unwrap_or(std::cmp::Ordering::Equal));
        // 原主人還沒開跑時，隊首是它下一張就要做的，留給它。
        let victim_running = tasks
            .iter()
            .any(|t| t.agent_id.as_deref() == Some(victim.id.as_str()) && t.status == STATUS_RUNNING);
        let Some(card) = queue
            .into_iter()
            .skip(if victim_running { 0 } else { 1 })
            .find(|t| t.card_kind == KIND_WORK && t.from_spec.is_some())
        else {
            continue;
        };
        let card_id = card.id.clone();
        // 條件式更新：排程器可能剛好在這之間把它認領走。
        let done = sqlx::query(
            "UPDATE tasks SET agent_id = ?, project_dir = ? WHERE id = ? AND status = 'queued' AND agent_id = ?",
        )
        .bind(&thief.id)
        .bind(&thief.project_dir)
        .bind(&card_id)
        .bind(&victim.id)
        .execute(pool)
        .await?
        .rows_affected();
        if done == 1 {
            moved += 1;
            if let Some(t) = tasks.iter_mut().find(|t| t.id == card_id) {
                t.agent_id = Some(thief.id.clone());
                t.project_dir = thief.project_dir.clone();
            }
        }
    }
    Ok(moved)
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
            if let (Some(module), None, Some(orch)) =
                (module.as_ref(), find_kind(&all_agents, "reviewer"), find_kind(&all_agents, "orchestrator"))
            {
                // 沒有 Reviewer：規格做完直接交給 Orchestrator 拆解。
                if module.is_spec {
                    create_plan_card(pool, project_path, module, orch, &all_agents, task.start_sha.as_deref(), task.end_sha.as_deref())
                        .await?;
                }
            }
            if let (Some(module), Some(reviewer)) = (module, find_kind(&all_agents, "reviewer")) {
                let previous = match task.review_of.as_deref() {
                    Some(rid) => store::get_task(pool, rid).await?,
                    None => None,
                };
                // 先插入拿到 id，才知道 review.json 放哪；body 隨後補上。
                let title = review_title(&module, task.card_kind == KIND_FIX);
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
                        from_spec: None,
                    },
                )
                .await?;
                let body = review_body(&module, &task, &review_json_path(project_path, &id), previous.as_ref());
                sqlx::query("UPDATE tasks SET body = ? WHERE id = ?").bind(&body).bind(&id).execute(pool).await?;
            }
        }
        KIND_REVIEW if success && task.verdict.as_deref() == Some("pass") => {
            let module = match task.module_id.as_deref() {
                Some(mid) => store::get_task(pool, mid).await?,
                None => None,
            };
            if let (Some(module), Some(orch)) = (module, find_kind(&all_agents, "orchestrator")) {
                if module.is_spec && !module.module_override {
                    create_plan_card(pool, project_path, &module, orch, &all_agents, task.start_sha.as_deref(), task.end_sha.as_deref())
                        .await?;
                }
            }
        }
        KIND_PLAN if success => {
            events.extend(dispatch_plan(pool, project_path, &task, &all_agents).await?);
        }
        KIND_MERGE if success => {
            let verdict = fast_forward_repo(&task.project_dir, task.end_sha.as_deref()).await;
            sqlx::query("UPDATE tasks SET verdict = ? WHERE id = ?").bind(verdict).bind(&task.id).execute(pool).await?;
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
                        from_spec: None,
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

/// 合併卡的 `verdict`：整合分支有沒有合回程式庫目前的分支。不是 `repo_merged`
/// 的值說明為什麼沒有自動合，畫面據此提示使用者按「合併分支」。
pub const REPO_MERGED: &str = "repo_merged";
pub const REPO_DIRTY: &str = "repo_dirty";
pub const REPO_MERGE_IN_PROGRESS: &str = "repo_merge_in_progress";
pub const REPO_DIVERGED: &str = "repo_diverged";

/// 合併卡成功後，把程式庫目前的分支快轉到整合結果（`end_sha`）。
///
/// 只做快轉：程式庫有未提交的改動、停在沒收尾的合併、或分支已經分岔
/// （使用者在程式庫自己 commit 過）都不動，留給使用者按「合併分支」決定。
async fn fast_forward_repo(repo_dir: &str, end_sha: Option<&str>) -> &'static str {
    let Some(sha) = end_sha else { return REPO_DIVERGED };
    let repo = crate::vcs::git::GitClient::new(repo_dir.to_string(), None);
    if repo.is_merge_in_progress().await {
        return REPO_MERGE_IN_PROGRESS;
    }
    if repo.has_uncommitted_changes().await.unwrap_or(true) {
        return REPO_DIRTY;
    }
    match repo.merge_ff_only(sha).await {
        Ok(()) => REPO_MERGED,
        Err(_) => REPO_DIVERGED,
    }
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
    let title = format!("合併 {} 個工程師分支", entries.len());
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
            from_spec: None,
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

    // ── 規格先行 ─────────────────────────────────────────────────────────

    async fn spec_writer(pool: &SqlitePool) -> String {
        let id = agent(pool, "coder", "規格撰寫師").await;
        agents::set_produces_spec(pool, &id, true).await.unwrap();
        id
    }

    /// 一份規格模組：由規格撰寫師做完、審查通過。回傳 (模組 id, 審查卡 id)。
    async fn passed_spec(pool: &SqlitePool, writer: &str, p: &Path, start: &str, end: &str) -> (String, String) {
        let id = store::create_task(pool, "A 規格", "寫 A 的規格", "/repo", true, false).await.unwrap();
        agents::assign_task(pool, &id, Some(writer)).await.unwrap();
        store::dispatch_for_test(pool, &id, "tab").await;
        store::set_start_sha(pool, &id, start).await.unwrap();
        store::set_end_sha(pool, &id, end).await.unwrap();
        store::finish_task(pool, &id, "success", None, None).await.unwrap();
        after_card_finished(pool, p, &id).await.unwrap();
        let review = of_kind(&all(pool).await, KIND_REVIEW)[0].id.clone();
        finish_review(pool, &review, "pass").await;
        (id, review)
    }

    fn chain_of(tasks: &[TaskRow], m: &str) -> Vec<TaskRow> {
        tasks.iter().filter(|t| t.module_id.as_deref() == Some(m)).cloned().collect()
    }

    #[tokio::test]
    async fn an_approved_spec_goes_to_the_orchestrator_for_splitting() {
        let pool = mem_pool().await;
        let w = spec_writer(&pool).await;
        agent(&pool, "coder", "實作一號").await;
        agent(&pool, "reviewer", "R").await;
        let o = agent(&pool, "orchestrator", "O").await;
        let p = Path::new("/proj");
        let (m, review) = passed_spec(&pool, &w, p, "s0", "e1").await;

        after_card_finished(&pool, p, &review).await.unwrap();

        let tasks = all(&pool).await;
        let plans = of_kind(&tasks, KIND_PLAN);
        assert_eq!(plans.len(), 1);
        let plan = plans[0];
        assert_eq!(plan.agent_id.as_deref(), Some(o.as_str()));
        assert_eq!(plan.module_id.as_deref(), Some(m.as_str()));
        assert_eq!((plan.start_sha.as_deref(), plan.end_sha.as_deref()), (Some("s0"), Some("e1")));
        assert!(plan.body.contains("實作一號"), "要列出可實作的 Coder");
        assert!(!plan.body.contains("規格撰寫師（"), "產出規格的 Coder 不能出現在實作清單");
        assert!(plan.body.contains(&tasks_json_path(p, &plan.id).display().to_string()));
        assert!(of_kind(&tasks, KIND_MERGE).is_empty(), "拆解完成前不能合併");
        let module = tasks.iter().find(|t| t.id == m).unwrap();
        assert_eq!(module_state(module, &chain_of(&tasks, &m), true), ModuleState::Dispatching);
    }

    #[tokio::test]
    async fn without_an_orchestrator_an_approved_spec_simply_passes() {
        let pool = mem_pool().await;
        let w = spec_writer(&pool).await;
        agent(&pool, "reviewer", "R").await;
        let p = Path::new("/proj");
        let (m, review) = passed_spec(&pool, &w, p, "s0", "e1").await;
        after_card_finished(&pool, p, &review).await.unwrap();
        let tasks = all(&pool).await;
        assert!(of_kind(&tasks, KIND_PLAN).is_empty());
        let module = tasks.iter().find(|t| t.id == m).unwrap();
        assert_eq!(module_state(module, &chain_of(&tasks, &m), true), ModuleState::Passed);
    }

    #[tokio::test]
    async fn a_failed_plan_is_dispatch_failed() {
        let pool = mem_pool().await;
        let w = spec_writer(&pool).await;
        agent(&pool, "reviewer", "R").await;
        agent(&pool, "orchestrator", "O").await;
        let p = Path::new("/proj");
        let (m, review) = passed_spec(&pool, &w, p, "s0", "e1").await;
        after_card_finished(&pool, p, &review).await.unwrap();
        let plan = of_kind(&all(&pool).await, KIND_PLAN)[0].id.clone();
        store::dispatch_for_test(&pool, &plan, "t").await;
        store::finish_task(&pool, &plan, "failed", Some("沒有 tasks.json"), None).await.unwrap();
        let tasks = all(&pool).await;
        let module = tasks.iter().find(|t| t.id == m).unwrap();
        assert_eq!(module_state(module, &chain_of(&tasks, &m), true), ModuleState::DispatchFailed);
    }

    fn git(dir: &Path, args: &[&str]) {
        assert!(std::process::Command::new("git").args(args).current_dir(dir).status().unwrap().success());
    }

    fn head(dir: &Path) -> String {
        let out = std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(dir).output().unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// 合併卡成功後，程式庫目前的分支快轉到整合結果；程式庫不乾淨或已分岔
    /// 就不動，verdict 記下原因。
    #[tokio::test]
    async fn a_successful_merge_fast_forwards_the_repo_only_when_it_is_safe() {
        let pool = mem_pool().await;
        let orch = agent(&pool, "orchestrator", "O").await;
        let repo = tempfile::tempdir().unwrap();
        let dir = repo.path();
        git(dir, &["init", "-q", "-b", "main"]);
        git(dir, &["config", "user.email", "t@t"]);
        git(dir, &["config", "user.name", "t"]);
        git(dir, &["commit", "-q", "--allow-empty", "-m", "init"]);
        let base = head(dir);
        // 整合分支：比 main 多一個有檔案的 commit。
        git(dir, &["checkout", "-q", "-b", "integration"]);
        std::fs::write(dir.join("game.ts"), "x").unwrap();
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", "integrate"]);
        let integrated = head(dir);
        git(dir, &["checkout", "-q", "main"]);

        let repo_dir = dir.to_string_lossy().into_owned();
        let merge = |end: String| {
            let pool = pool.clone();
            let (orch, repo_dir) = (orch.clone(), repo_dir.clone());
            async move {
                let id = insert_card(
                    &pool,
                    &NewCard {
                        title: "合併", body: "", project_dir: &repo_dir, agent_id: &orch, card_kind: KIND_MERGE,
                        module_id: None, review_of: None, start_sha: None, end_sha: None, sort_order: 1.0, from_spec: None,
                    },
                )
                .await
                .unwrap();
                store::dispatch_for_test(&pool, &id, "tab-m").await;
                store::set_end_sha(&pool, &id, &end).await.unwrap();
                store::finish_task(&pool, &id, "success", None, None).await.unwrap();
                after_card_finished(&pool, Path::new("/p"), &id).await.unwrap();
                store::get_task(&pool, &id).await.unwrap().unwrap().verdict
            }
        };

        // 不乾淨：有未追蹤的檔案 → 不動。
        std::fs::write(dir.join("notes.txt"), "mine").unwrap();
        assert_eq!(merge(integrated.clone()).await.as_deref(), Some(REPO_DIRTY));
        assert_eq!(head(dir), base);
        std::fs::remove_file(dir.join("notes.txt")).unwrap();

        // 乾淨而且可以快轉 → 檔案出現在程式庫。
        assert_eq!(merge(integrated.clone()).await.as_deref(), Some(REPO_MERGED));
        assert_eq!(head(dir), integrated);
        assert!(dir.join("game.ts").exists());

        // 使用者在 main 上自己 commit 過、整合結果不是它的後代 → 不動。
        git(dir, &["reset", "-q", "--hard", &base]);
        git(dir, &["commit", "-q", "--allow-empty", "-m", "mine"]);
        let mine = head(dir);
        assert_eq!(merge(integrated.clone()).await.as_deref(), Some(REPO_DIVERGED));
        assert_eq!(head(dir), mine);
    }

    /// 一張由規格拆出、已派給 `coder` 的實作卡（queued）。
    async fn spec_card(pool: &SqlitePool, coder: &str, title: &str, order: f64) -> String {
        insert_card(
            pool,
            &NewCard {
                title, body: "做", project_dir: "/repo", agent_id: coder, card_kind: KIND_WORK,
                module_id: None, review_of: None, start_sha: None, end_sha: None,
                sort_order: order, from_spec: Some("spec"),
            },
        )
        .await
        .unwrap()
    }

    fn owner(tasks: &[TaskRow], id: &str) -> String {
        tasks.iter().find(|t| t.id == id).unwrap().agent_id.clone().unwrap()
    }

    #[tokio::test]
    async fn an_idle_coder_takes_the_next_unstarted_spec_card_from_the_busiest_coder() {
        // 實機：工程師3 在拆解完成後才加入，17 張全在工程師2 身上，工程師3 一直閒著。
        let pool = mem_pool().await;
        let busy = agent(&pool, "coder", "工程師2").await;
        let late = agent(&pool, "coder", "工程師3").await;
        sqlx::query("UPDATE agents SET project_dir = '/late' WHERE id = ?").bind(&late).execute(&pool).await.unwrap();
        let running = spec_card(&pool, &busy, "跑中", 1.0).await;
        store::dispatch_for_test(&pool, &running, "tab").await;
        let next = spec_card(&pool, &busy, "下一張", 2.0).await;
        let later = spec_card(&pool, &busy, "再下一張", 3.0).await;

        assert_eq!(rebalance_idle_coders(&pool).await.unwrap(), 1);
        let tasks = all(&pool).await;
        assert_eq!(owner(&tasks, &next), late, "接走最前面還沒開始的那張");
        assert_eq!(tasks.iter().find(|t| t.id == next).unwrap().project_dir, "/late", "工作目錄換成接手者的程式庫");
        assert_eq!(owner(&tasks, &later), busy);
        assert_eq!(owner(&tasks, &running), busy, "跑中的卡不動");
        // 接手者已經有卡了，不會再多拿。
        assert_eq!(rebalance_idle_coders(&pool).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn rebalancing_leaves_manual_assignments_and_ineligible_coders_alone() {
        let pool = mem_pool().await;
        let busy = agent(&pool, "coder", "忙").await;
        let paused = agent(&pool, "coder", "暫停").await;
        sqlx::query("UPDATE agents SET paused_task_id = 'x' WHERE id = ?").bind(&paused).execute(&pool).await.unwrap();
        let writer = spec_writer(&pool).await;
        agent(&pool, "reviewer", "R").await;
        let idle = agent(&pool, "coder", "閒").await;
        // 使用者手動指派的模組卡（不是規格拆出來的）：閒著的工程師也不搶。
        for t in ["A", "B"] {
            let id = store::create_task(&pool, t, "", "/repo", true, false).await.unwrap();
            agents::assign_task(&pool, &id, Some(&busy)).await.unwrap();
        }
        assert_eq!(rebalance_idle_coders(&pool).await.unwrap(), 0);
        sqlx::query("DELETE FROM agents WHERE id = ?").bind(&idle).execute(&pool).await.unwrap();

        // 有可搶的規格卡，但閒著的只有暫停中的 Coder、規格撰寫師與 Reviewer：都不接。
        let run = spec_card(&pool, &busy, "跑中", 5.0).await;
        store::dispatch_for_test(&pool, &run, "tab").await;
        let waiting = spec_card(&pool, &busy, "等著", 6.0).await;
        assert_eq!(rebalance_idle_coders(&pool).await.unwrap(), 0);
        assert_eq!(owner(&all(&pool).await, &waiting), busy);
        assert_ne!(owner(&all(&pool).await, &waiting), writer);

        // 規格卡只有一張而且原主人閒著：讓原主人自己做，不搶。
        sqlx::query("UPDATE agents SET paused_task_id = NULL WHERE id = ?").bind(&paused).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM tasks WHERE id <> ?").bind(&waiting).execute(&pool).await.unwrap();
        assert_eq!(rebalance_idle_coders(&pool).await.unwrap(), 0);
        assert_eq!(owner(&all(&pool).await, &waiting), busy);
    }

    #[tokio::test]
    async fn a_finished_plan_spreads_tasks_over_the_least_busy_coders_with_the_spec_attached() {
        // 真的 git repo：規格檔案在範圍內被新增，實作卡要附上它的全文。
        let repo = tempfile::tempdir().unwrap();
        git(repo.path(), &["init", "-q"]);
        git(repo.path(), &["config", "user.email", "t@t"]);
        git(repo.path(), &["config", "user.name", "T"]);
        std::fs::write(repo.path().join("README.md"), "x").unwrap();
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-qm", "init"]);
        let start = head(repo.path());
        std::fs::create_dir_all(repo.path().join("docs/specs")).unwrap();
        std::fs::write(repo.path().join("docs/specs/A.md"), "# A 規格\n登入要支援 SSO").unwrap();
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-qm", "spec"]);
        let end = head(repo.path());
        let repo_dir = repo.path().to_string_lossy().into_owned();

        let pool = mem_pool().await;
        let w = spec_writer(&pool).await;
        // 指派時卡片的工作目錄會換成 Agent 的程式庫，所以規格撰寫師要指向真的 repo。
        sqlx::query("UPDATE agents SET project_dir = ? WHERE id = ?").bind(&repo_dir).bind(&w).execute(&pool).await.unwrap();
        let busy = agent(&pool, "coder", "忙碌的").await;
        let idle = agent(&pool, "coder", "閒的").await;
        let o = agent(&pool, "orchestrator", "O").await;
        // 「忙碌的」手上已經有一張排隊的卡。
        let other = store::create_task(&pool, "其他", "", &repo_dir, true, false).await.unwrap();
        agents::assign_task(&pool, &other, Some(&busy)).await.unwrap();

        let project = tempfile::tempdir().unwrap();
        let spec = store::create_task(&pool, "A 規格", "寫 A 的規格", &repo_dir, true, false).await.unwrap();
        agents::assign_task(&pool, &spec, Some(&w)).await.unwrap();
        store::dispatch_for_test(&pool, &spec, "tab").await;
        store::set_start_sha(&pool, &spec, &start).await.unwrap();
        store::set_end_sha(&pool, &spec, &end).await.unwrap();
        store::finish_task(&pool, &spec, "success", None, None).await.unwrap();
        // 沒有 Reviewer：規格做完直接交給 Orchestrator。
        after_card_finished(&pool, project.path(), &spec).await.unwrap();
        let plan = of_kind(&all(&pool).await, KIND_PLAN)[0].clone();
        assert_eq!(plan.agent_id.as_deref(), Some(o.as_str()));

        let json = tasks_json_path(project.path(), &plan.id);
        std::fs::create_dir_all(json.parent().unwrap()).unwrap();
        std::fs::write(&json, r#"{"tasks":[{"title":"A-後端 API","body":"做 API"},{"title":"A-登入頁","body":"做畫面"},{"title":"A-測試","body":"補測試"}]}"#).unwrap();
        store::dispatch_for_test(&pool, &plan.id, "t").await;
        store::finish_task(&pool, &plan.id, "success", None, None).await.unwrap();
        let events = after_card_finished(&pool, project.path(), &plan.id).await.unwrap();
        assert!(events.is_empty());

        let tasks = all(&pool).await;
        let impl_cards: Vec<&TaskRow> = tasks.iter().filter(|t| t.from_spec.as_deref() == Some(spec.as_str())).collect();
        assert_eq!(impl_cards.len(), 3);
        let to = |title: &str| impl_cards.iter().find(|t| t.title == title).unwrap().agent_id.clone().unwrap();
        // 閒的（0 張）先拿第一張；之後兩邊各 1 張平手，平手時誰先拿不重要，
        // 但下一張一定給另一位——所以三張必定是 閒的 2、忙碌的 1。
        assert_eq!(to("A-後端 API"), idle);
        let count = |id: &str| impl_cards.iter().filter(|t| t.agent_id.as_deref() == Some(id)).count();
        assert_eq!((count(&idle), count(&busy)), (2, 1));
        assert!(impl_cards.iter().all(|t| t.agent_id.as_deref() != Some(w.as_str())), "規格撰寫師不接實作");
        assert!(impl_cards.iter().all(|t| t.card_kind == KIND_WORK && t.status == STATUS_QUEUED));
        assert!(impl_cards[0].body.contains("登入要支援 SSO"), "要附規格全文：{}", impl_cards[0].body);
        let module = tasks.iter().find(|t| t.id == spec).unwrap();
        assert_eq!(module_state(module, &chain_of(&tasks, &spec), false), ModuleState::Passed);
        assert!(of_kind(&tasks, KIND_MERGE).is_empty(), "實作卡還沒做，不能合併");
    }

    /// 規格撰寫師 `w` 交出一份規格（沒有 Reviewer，直接交給 Orchestrator），
    /// Orchestrator 寫出 `tasks_json` 後結案。回傳拆出的實作卡（依建立順序）。
    async fn split_spec(pool: &SqlitePool, w: &str, project: &Path, tasks_json: &str) -> Vec<TaskRow> {
        let spec = store::create_task(pool, "A 規格", "寫 A 的規格", "/repo", true, false).await.unwrap();
        agents::assign_task(pool, &spec, Some(w)).await.unwrap();
        store::dispatch_for_test(pool, &spec, "tab").await;
        store::finish_task(pool, &spec, "success", None, None).await.unwrap();
        after_card_finished(pool, project, &spec).await.unwrap();
        let plan = of_kind(&all(pool).await, KIND_PLAN)[0].clone();
        let json = tasks_json_path(project, &plan.id);
        std::fs::create_dir_all(json.parent().unwrap()).unwrap();
        std::fs::write(&json, tasks_json).unwrap();
        store::dispatch_for_test(pool, &plan.id, "t").await;
        store::finish_task(pool, &plan.id, "success", None, None).await.unwrap();
        after_card_finished(pool, project, &plan.id).await.unwrap();
        let mut cards: Vec<TaskRow> =
            all(pool).await.into_iter().filter(|t| t.from_spec.as_deref() == Some(spec.as_str())).collect();
        cards.sort_by(|a, b| a.sort_order.partial_cmp(&b.sort_order).unwrap());
        cards
    }

    fn deps_of(card: &TaskRow) -> Vec<String> {
        card.depends_on.as_deref().map(|j| serde_json::from_str(j).unwrap()).unwrap_or_default()
    }

    #[tokio::test]
    async fn dependent_tasks_go_to_the_coder_that_owns_most_of_their_prerequisites() {
        let pool = mem_pool().await;
        let w = spec_writer(&pool).await;
        let c1 = agent(&pool, "coder", "C1").await;
        let c2 = agent(&pool, "coder", "C2").await;
        agent(&pool, "orchestrator", "O").await;
        let project = tempfile::tempdir().unwrap();
        let cards = split_spec(
            &pool,
            &w,
            project.path(),
            r#"{"tasks":[
                {"title":"audio"},
                {"title":"storage"},
                {"title":"audio 測試","depends_on":[0]},
                {"title":"整合","depends_on":[0,1,2]},
                {"title":"雙邊","depends_on":[0,1]}
            ]}"#,
        )
        .await;
        let by = |title: &str| cards.iter().find(|c| c.title == title).unwrap();
        let owner = |title: &str| by(title).agent_id.clone().unwrap();
        // 沒有相依：照最閒，兩位各拿一張（平手時誰先拿不重要）。
        let (a, b) = (owner("audio"), owner("storage"));
        assert_ne!(a, b);
        assert!([&c1, &c2].contains(&&a) && [&c1, &c2].contains(&&b));
        // 相依集中到擁有前置的人，即使他已經比較忙。
        assert_eq!(owner("audio 測試"), a);
        assert_eq!(owner("整合"), a, "audio 的負責人擁有 2 個前置、另一位只有 1 個");
        // 兩邊各擁有一個前置：平手挑比較閒的（a 已經 3 張、b 1 張）。
        assert_eq!(owner("雙邊"), b);
        // 相依存成實作卡的 id。
        assert_eq!(deps_of(by("整合")), vec![by("audio").id.clone(), by("storage").id.clone(), by("audio 測試").id.clone()]);
        assert!(deps_of(by("audio")).is_empty() && by("audio").depends_on.is_none());
    }

    #[tokio::test]
    async fn without_any_implementer_the_split_tasks_land_in_planning_and_raise_attention() {
        let pool = mem_pool().await;
        let w = spec_writer(&pool).await;
        agent(&pool, "orchestrator", "O").await;
        let project = tempfile::tempdir().unwrap();
        let spec = store::create_task(&pool, "A 規格", "", "/repo", true, false).await.unwrap();
        agents::assign_task(&pool, &spec, Some(&w)).await.unwrap();
        store::dispatch_for_test(&pool, &spec, "tab").await;
        store::finish_task(&pool, &spec, "success", None, None).await.unwrap();
        after_card_finished(&pool, project.path(), &spec).await.unwrap();
        let plan = of_kind(&all(&pool).await, KIND_PLAN)[0].id.clone();
        let json = tasks_json_path(project.path(), &plan);
        std::fs::create_dir_all(json.parent().unwrap()).unwrap();
        std::fs::write(&json, r#"{"tasks":[{"title":"T1","body":"b"}]}"#).unwrap();
        store::dispatch_for_test(&pool, &plan, "t").await;
        store::finish_task(&pool, &plan, "success", None, None).await.unwrap();

        let events = after_card_finished(&pool, project.path(), &plan).await.unwrap();

        assert_eq!(events.len(), 1);
        let t1 = all(&pool).await.into_iter().find(|t| t.title == "T1").unwrap();
        assert_eq!((t1.status.as_str(), t1.agent_id), (STATUS_PLANNING, None));
        assert_eq!(t1.from_spec.as_deref(), Some(spec.as_str()));
    }

    #[test]
    fn plan_json_parsing_checks_count_and_titles() {
        assert_eq!(parse_plan_json(r#"{"tasks":[{"title":" A ","body":"b"}]}"#).unwrap()[0].title, "A");
        assert!(parse_plan_json(r#"{"tasks":[]}"#).is_err());
        assert!(parse_plan_json(r#"{"tasks":[{"title":"  "}]}"#).is_err());
        let many = format!(r#"{{"tasks":[{}]}}"#, vec![r#"{"title":"x"}"#; MAX_PLAN_TASKS + 1].join(","));
        assert!(parse_plan_json(&many).is_err());
        assert!(parse_plan_json("nope").is_err());
    }

    #[test]
    fn plan_json_depends_on_must_point_to_an_earlier_task() {
        // 舊格式：沒有 depends_on＝沒有相依。
        assert_eq!(parse_plan_json(r#"{"tasks":[{"title":"A"}]}"#).unwrap()[0].depends_on, Vec::<usize>::new());
        let ok = parse_plan_json(r#"{"tasks":[{"title":"A"},{"title":"B"},{"title":"整合","depends_on":[0,1,1]}]}"#).unwrap();
        assert_eq!(ok[2].depends_on, vec![0, 1], "重複的索引只留一個");
        for (json, why) in [
            (r#"{"tasks":[{"title":"A","depends_on":[0]}]}"#, "依賴自己"),
            (r#"{"tasks":[{"title":"A","depends_on":[1]},{"title":"B"}]}"#, "依賴後面的任務"),
            (r#"{"tasks":[{"title":"A"},{"title":"B","depends_on":[7]}]}"#, "超出範圍"),
        ] {
            let err = parse_plan_json(json).expect_err(why);
            assert!(err.contains("depends_on"), "{why}：錯誤訊息要指出 depends_on：{err}");
        }
    }

    #[test]
    fn the_plan_prompt_asks_for_dependencies() {
        let module = test_row("M");
        let body = plan_body(&module, Some("s"), Some("e"), &[("工程師2".into(), "r".into())], Path::new("/t/tasks.json"));
        assert!(body.contains("depends_on"), "{body}");
        assert!(body.contains("整合"), "要提醒整合類任務列出所有要組裝的模組：{body}");
    }

    #[test]
    fn spec_modules_get_a_spec_review_checklist_code_modules_do_not() {
        let mut m = TaskRow { ..test_row("M") };
        let reviewed = TaskRow { start_sha: Some("a".into()), end_sha: Some("b".into()), ..test_row("W") };
        let code = review_body(&m, &reviewed, Path::new("/p/review.json"), None);
        assert!(code.contains("程式碼變更") && !code.contains("驗收條件是否具體"));
        assert_eq!(review_title(&m, false), "審查：M");
        m.is_spec = true;
        let spec = review_body(&m, &reviewed, Path::new("/p/review.json"), None);
        assert!(spec.contains("規格文件") && spec.contains("驗收條件是否具體") && spec.contains("git diff a..b"));
        assert!(spec.contains("請審查模組「M」"), "保留同一個開頭，代理（含驗收用的假 claude）才認得出是審查");
        assert_eq!(review_title(&m, true), "審查規格：M（修正後）");
    }

    fn test_row(id: &str) -> TaskRow {
        TaskRow {
            id: id.into(), title: id.into(), body: "需求".into(), project_dir: "/r".into(), status: "done".into(),
            parallel_ok: true, interactive: false, sort_order: 0.0, outcome: Some("success".into()), tab_id: None,
            transcript_path: None, error_message: None, created_at: String::new(), dispatched_at: None,
            finished_at: None, ai_summary: None, archived_at: None, session_id: None, session_path: None,
            use_bridge: false, bridge_tiers: None, label: None, worktree_path: None, worktree_branch: None,
            isolate_worktree: None, agent_id: None, start_sha: None, end_sha: None, card_kind: "work".into(),
            module_id: None, review_of: None, verdict: None, review_reasons: None, module_override: false,
            is_spec: false, from_spec: None, quota_wait_at: None, depends_on: None, assigned_from: None, assigned_at: None, premerged: None,
        }
    }
}
