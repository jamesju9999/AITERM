//! Server 無頭編排：對開了「由 server 執行」的共用看板專案，自動跑
//! Coder 做模組 → Reviewer 審查 → 不通過退回修正（最多 2 輪）→ 全部通過後建「合併待核准」卡。
//!
//! - **每個 Coder 一個專屬 worktree 與分支**（`<data>/worktrees/agent-<id>`、`agent/<id前12碼>`），
//!   同一個 Coder 的模組依序累積在同一條分支上；Coder 同一時間只做一張卡。
//! - **審查在一次性的 detached worktree 裡唯讀進行**，看的是被審那版（`end_sha`），不受 Coder 手上的新改動影響。
//! - **永不自動合併**：全部通過後只建一張「合併待核准」卡，管理員核准才合併（`merge_approved`）。
//! - 狀態全在卡片上，每個 tick 重新算一次（`aiterm_intake_core::orchestration`），server 重啟不會漏步驟；
//!   重啟時處理中的卡片由租約逾期放回待派送，不做中途接續。
use crate::runner::{self, Mode, WHO};
use crate::state::AppState;
use aiterm_intake_core::board_store::{self as bs, BoardAgent, BoardCard, NewWorkflowCard, QUEUED};
use aiterm_intake_core::config;
use aiterm_intake_core::host::Notifier;
use aiterm_intake_core::orchestration::{self as orch, ModuleState};
use aiterm_intake_core::store;
use aiterm_intake_core::verify;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

const HEARTBEAT_EVERY: Duration = Duration::from_secs(120);
/// 審查時最多帶多少字元的 diff 給 Reviewer。
const MAX_DIFF_CHARS: usize = 60_000;
/// 附在實作卡與拆解提示詞裡的規格全文上限。
const SPEC_TEXT_LIMIT: usize = 30_000;

/// 同一時間只讓一個 tick 在跑：迴圈與測試／手動觸發重疊時，不會重複建審查卡。
static TICK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn key(pid: &str) -> String {
    format!("orchestrate:{pid}")
}

pub async fn enabled(pool: &sqlx::SqlitePool, pid: &str) -> bool {
    store::get_setting(pool, &key(pid)).await.ok().flatten().as_deref() == Some("1")
}

pub async fn set_enabled(pool: &sqlx::SqlitePool, pid: &str, on: bool) -> Result<(), sqlx::Error> {
    store::set_setting(pool, &key(pid), if on { "1" } else { "0" }).await
}

pub fn agent_branch(agent_id: &str) -> String {
    format!("agent/{}", agent_id.chars().take(12).collect::<String>())
}

pub fn agent_worktree(st: &AppState, agent_id: &str) -> PathBuf {
    st.data_dir.join("worktrees").join(format!("agent-{agent_id}"))
}

fn review_worktree(st: &AppState, card_id: &str) -> PathBuf {
    st.data_dir.join("worktrees").join(format!("review-{card_id}"))
}

fn reasons_of(card: &BoardCard) -> Vec<String> {
    card.review_reasons.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default()
}

// ---- 推進：依卡片狀態建出下一步的卡 ----

/// 對一個專案：依每個模組的狀態建審查卡、修正卡，全部通過就建合併卡。回傳新建的卡數。
pub async fn advance(st: &AppState, pid: &str) -> Result<usize, sqlx::Error> {
    let pool = &st.pools.intake;
    let agents = bs::list_agents(pool, pid).await?;
    let cards = bs::project_cards(pool, pid).await?;
    let reviewer = agents.iter().find(|a| a.kind == "reviewer");
    let orchestrator = agents.iter().find(|a| a.kind == "orchestrator");
    let mods = orch::modules(&cards, &agents);
    let mut created = 0;
    for m in &mods {
        let chain: Vec<BoardCard> = cards.iter().filter(|c| c.module_id.as_deref() == Some(m.id.as_str()) && matches!(c.card_kind.as_str(), "review" | "fix" | "plan")).cloned().collect();
        let state = orch::module_status(m, &chain, reviewer.is_some(), orchestrator.is_some());
        match state {
            ModuleState::NeedsReview { target } => {
                let reviewed = cards.iter().find(|c| c.id == target).unwrap_or(m);
                let title = format!("審查：{}", m.title);
                bs::create_workflow_card(pool, NewWorkflowCard { project_id: pid, title: &title, body: &orch::review_card_body(m, reviewed), kind: "review", agent_id: reviewer.map(|a| a.id.as_str()), module_id: Some(&m.id), review_of: Some(&target), status: QUEUED, depends_on: None, from_spec: None }).await?;
                created += 1;
            }
            ModuleState::NeedsFix { review, round } => {
                let reasons = cards.iter().find(|c| c.id == review).map(reasons_of).unwrap_or_default();
                let title = format!("修正（第 {round} 次）：{}", m.title);
                bs::create_workflow_card(pool, NewWorkflowCard { project_id: pid, title: &title, body: &orch::fix_body(m, &reasons, round), kind: "fix", agent_id: m.agent_id.as_deref(), module_id: Some(&m.id), review_of: Some(&review), status: QUEUED, depends_on: None, from_spec: None }).await?;
                created += 1;
            }
            ModuleState::NeedsPlan { sha } => {
                // 規格審查通過：交給 Orchestrator 拆解（沒有 Orchestrator 時 module_status 不會走到這裡）。
                let Some(orc) = orchestrator else { continue };
                let title = format!("拆解規格：{}", m.title);
                let plan = bs::create_workflow_card(pool, NewWorkflowCard { project_id: pid, title: &title, body: &format!("把通過審查的規格「{}」拆成可以分頭實作的任務，依相依順序派給工程師。", m.title), kind: "plan", agent_id: Some(&orc.id), module_id: Some(&m.id), review_of: None, status: QUEUED, depends_on: None, from_spec: None }).await?;
                bs::set_shas(pool, &plan.id, m.start_sha.as_deref(), sha.as_deref()).await?;
                created += 1;
            }
            _ => {}
        }
    }
    // 已經有一張等著核准的就不再建；核准過（done）的不算——合併之後新完成的模組要能再有一張。
    let pending_merge = cards.iter().any(|c| c.card_kind == "merge" && c.status != bs::DONE);
    // 剛建的審查／修正卡讓狀態變了，用最新的卡片重算「是不是全部通過」。
    let latest = if created > 0 { bs::project_cards(pool, pid).await? } else { cards.clone() };
    if !pending_merge && orch::merge_status(&latest, &agents) == orch::MergeStatus::Ready {
        let coders: Vec<&BoardAgent> = agents.iter().filter(|a| a.kind == "coder" && mods.iter().any(|m| m.agent_id.as_deref() == Some(a.id.as_str()))).collect();
        let list: String = coders.iter().map(|a| format!("- {}（分支 {}）\n", a.name, agent_branch(&a.id))).collect();
        let body = format!("所有模組都審查通過。核准後會把下列分支合併進專案資料夾目前的分支（不會自動合併，要管理員在 AITerm 核准）：\n\n{list}");
        bs::create_workflow_card(pool, NewWorkflowCard { project_id: pid, title: "合併待核准", body: &body, kind: "merge", agent_id: None, module_id: None, review_of: None, status: bs::PLANNING, depends_on: None, from_spec: None }).await?;
        created += 1;
    }
    Ok(created)
}

// ---- 派工 ----

/// 掃一次：推進每個開了編排的專案，再在並行上限內把待派送的卡派給閒置的 Agent。回傳這次開始處理的卡數。
pub async fn tick_once(st: &Arc<AppState>) -> usize {
    let _one_at_a_time = TICK.lock().await;
    let settings = runner::load(&st.pools.intake).await;
    if settings.validate().is_err() {
        return 0;
    }
    let mut started = 0;
    let mut checked = false;
    for p in crate::routes_data::project_list(st).await {
        if !enabled(&st.pools.intake, &p.id).await {
            continue;
        }
        if matches!(advance(st, &p.id).await, Ok(n) if n > 0) {
            st.events.changed();
        }
        if !checked {
            checked = true;
            if !runner::availability(st, false).await.available {
                return started; // claude 不能用：不領卡，卡片留在待派送
            }
        }
        let Ok(agents) = bs::list_agents(&st.pools.intake, &p.id).await else { continue };
        let Ok(cards) = bs::project_cards(&st.pools.intake, &p.id).await else { continue };
        for card in cards.iter().filter(|c| c.status == QUEUED && matches!(c.card_kind.as_str(), "work" | "fix" | "review" | "plan")) {
            if st.runner.running() >= settings.max_concurrent {
                return started;
            }
            let Some(agent) = card.agent_id.as_deref().and_then(|id| agents.iter().find(|a| a.id == id)) else { continue };
            if agent.paused_card_id.is_some() || bs::agent_busy(&st.pools.intake, &agent.id).await.unwrap_or(true) {
                continue;
            }
            // 上次因為額度用完被放回來的卡：重置時間沒到就先不派。
            if bs::card_waiting(&st.pools.intake, &card.id, crate::db::now()).await.unwrap_or(false) {
                continue;
            }
            // 前置（依賴）在別位工程師手上、還沒通過審查：先不開工，等它過了才有可以接的程式碼。
            if !deps_ready(card, &cards, &agents) {
                continue;
            }
            if !matches!(bs::claim_card(&st.pools.intake, &card.id, WHO, crate::db::now()).await, Ok(true)) {
                continue;
            }
            st.runner.running.fetch_add(1, Ordering::SeqCst);
            started += 1;
            tokio::spawn(run_card(st.clone(), card.id.clone()));
        }
    }
    started
}

/// 處理一張已領取的卡；不管成敗都會結案，不會留在處理中。
async fn run_card(st: Arc<AppState>, card_id: String) {
    let beat = {
        let (pool, id) = (st.pools.intake.clone(), card_id.clone());
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(HEARTBEAT_EVERY).await;
                if !matches!(bs::renew_claim(&pool, &id, WHO, crate::db::now()).await, Ok(true)) {
                    break;
                }
            }
        })
    };
    let result = run_inner(&st, &card_id).await;
    beat.abort();
    let pool = &st.pools.intake;
    let now = crate::db::now();
    match result {
        Ok(summary) => {
            bs::finish_claim(pool, &card_id, WHO, "success", Some(&summary), None, now).await.ok();
            bs::clear_card_wait(pool, &card_id).await.ok();
            crate::audit::record(&st.pools.server, WHO, "board.orchestrate", &card_id).await;
        }
        Err(e) => {
            // 額度或速率限制不是這張卡的錯：放回待派送，等重置後自動續跑（有次數上限，超過才算失敗）。
            let waited = match runner::quota_wait(&e, now) {
                Some(until) => match bs::set_card_wait(pool, &card_id, until, &e.chars().take(200).collect::<String>()).await {
                    Ok(n) if n <= bs::QUOTA_MAX_ATTEMPTS => {
                        let note = format!("Claude Code 額度或速率限制，約 {} 後自動續跑（第 {n} 次等待）", chrono::DateTime::from_timestamp(until, 0).map(|d| d.with_timezone(&chrono::Local).format("%H:%M").to_string()).unwrap_or_default());
                        bs::requeue_with_note(pool, &card_id, WHO, &note, now).await.unwrap_or(false)
                    }
                    _ => false,
                },
                None => false,
            };
            if waited {
                crate::audit::record(&st.pools.server, WHO, "board.orchestrate_wait", &card_id).await;
            } else {
                bs::finish_claim(pool, &card_id, WHO, "failed", None, Some(&e), now).await.ok();
                crate::audit::record(&st.pools.server, WHO, "board.orchestrate_fail", &card_id).await;
            }
        }
    }
    st.runner.running.fetch_sub(1, Ordering::SeqCst);
    st.events.changed();
}

async fn work_dir_of(st: &AppState, project_id: &str) -> Result<PathBuf, String> {
    let wd: String = sqlx::query_scalar("SELECT work_dir FROM projects WHERE id = ?").bind(project_id).fetch_optional(&st.pools.server).await.ok().flatten().unwrap_or_default();
    let p = PathBuf::from(wd.trim());
    if wd.trim().is_empty() {
        return Err("這個專案沒有設定工作資料夾（work_dir），server 不知道要在哪裡做".into());
    }
    if !p.is_dir() {
        return Err(format!("工作資料夾不存在：{}", p.display()));
    }
    Ok(p)
}

async fn run_inner(st: &Arc<AppState>, card_id: &str) -> Result<String, String> {
    let pool = &st.pools.intake;
    let card = bs::get_card(pool, card_id).await.map_err(|e| e.to_string())?.ok_or("卡片不見了")?;
    let agent = bs::get_agent(pool, card.agent_id.as_deref().unwrap_or("")).await.map_err(|e| e.to_string())?.ok_or("這張卡指派的 Agent 已經被刪除")?;
    let settings = runner::load(pool).await;
    let work_dir = work_dir_of(st, &card.project_id).await?;
    let cfg = config::load(pool, &card.project_id).await;
    if card.card_kind == "review" {
        return run_review(st, &settings, &work_dir, &agent, &card).await;
    }
    if card.card_kind == "plan" {
        return run_plan(st, &settings, &work_dir, &agent, &card).await;
    }
    // work／fix：在這個 Coder 專屬的 worktree 裡改。
    let wt = ensure_agent_worktree(st, &work_dir, &agent).await?;
    // 先把別位工程師已通過審查的前置合併進來；起始版本要在這之後才記，審查的 diff 才不會把前置的程式碼算成這張卡的。
    premerge_dependencies(st, &wt, &card).await?;
    let start = runner::git(&wt, &["rev-parse", "HEAD"]).await?.trim().to_string();
    let mode = Mode::Edit { verify_command: cfg.verify_command.clone() };
    let prompt = orch::work_prompt(&agent, &card);
    let before = verify::snapshot(&wt).await;
    let text = runner::exec_claude_as(&settings, &wt, &mode, &[], &prompt, agent.model.as_deref()).await?;
    // 專案設了驗證指令：Coder 做完由 server 在 worktree 裡實際跑一次（Coder 自己多半沒權限跑），結果交給審查。
    if !cfg.verify_command.trim().is_empty() {
        let report = verify::verify(&wt, before.as_deref(), &cfg.verify_command, Duration::from_secs(cfg.verify_timeout_secs.clamp(1, 600))).await;
        bs::set_verification(pool, card_id, &aiterm_intake_core::filter::redact(&report.text()), report.passed()).await.map_err(|e| e.to_string())?;
    }
    if !runner::git(&wt, &["status", "--porcelain"]).await?.trim().is_empty() {
        runner::git(&wt, &["add", "-A"]).await?;
        runner::git(&wt, &["commit", "-q", "-m", &format!("{}：{}", agent.name, card.title.chars().take(60).collect::<String>())]).await?;
    }
    let end = runner::git(&wt, &["rev-parse", "HEAD"]).await?.trim().to_string();
    bs::set_shas(pool, card_id, Some(&start), Some(&end)).await.map_err(|e| e.to_string())?;
    Ok(aiterm_intake_core::filter::redact(&text).chars().take(4000).collect())
}

/// Coder 的專屬 worktree：不存在就從專案目前的版本開一個；分支已存在（重啟後）就沿用。
async fn ensure_agent_worktree(st: &AppState, work_dir: &Path, agent: &BoardAgent) -> Result<PathBuf, String> {
    let wt = agent_worktree(st, &agent.id);
    if wt.join(".git").exists() {
        return Ok(wt);
    }
    runner::git(work_dir, &["rev-parse", "--verify", "HEAD"]).await.map_err(|_| "工作資料夾不是有提交紀錄的 git 專案，無法開隔離的 worktree".to_string())?;
    std::fs::create_dir_all(wt.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
    let _ = runner::git(work_dir, &["worktree", "prune"]).await;
    let branch = agent_branch(&agent.id);
    let wt_s = wt.to_string_lossy().to_string();
    if runner::git(work_dir, &["rev-parse", "--verify", &format!("refs/heads/{branch}")]).await.is_ok() {
        runner::git(work_dir, &["worktree", "add", &wt_s, &branch]).await?;
    } else {
        runner::git(work_dir, &["worktree", "add", "-b", &branch, &wt_s, "HEAD"]).await?;
    }
    Ok(wt)
}

async fn run_review(st: &Arc<AppState>, settings: &runner::RunnerSettings, work_dir: &Path, agent: &BoardAgent, card: &BoardCard) -> Result<String, String> {
    let pool = &st.pools.intake;
    let module = bs::get_card(pool, card.module_id.as_deref().unwrap_or("")).await.map_err(|e| e.to_string())?.ok_or("找不到這張審查卡的模組")?;
    let reviewed = bs::get_card(pool, card.review_of.as_deref().unwrap_or("")).await.map_err(|e| e.to_string())?.ok_or("找不到被審的卡片")?;
    let (start, end) = (reviewed.start_sha.clone().ok_or("被審的卡片沒有記下起始版本")?, reviewed.end_sha.clone().ok_or("被審的卡片沒有記下完成版本")?);
    let diff: String = runner::git(work_dir, &["diff", &format!("{start}..{end}")]).await?.chars().take(MAX_DIFF_CHARS).collect();
    // 上一輪的理由：被審的是修正卡時，它指向上一張審查卡。
    let previous = if reviewed.card_kind == "fix" { bs::get_card(pool, reviewed.review_of.as_deref().unwrap_or("")).await.ok().flatten().map(|r| reasons_of(&r)).unwrap_or_default() } else { vec![] };
    let verification = bs::get_verification(pool, &reviewed.id).await.ok().flatten();
    let prompt = orch::review_prompt(agent, &module, &reviewed, &diff, &previous, verification.as_ref().map(|(r, p)| (r.as_str(), *p)));
    // 在被審那版的一次性 worktree 裡唯讀審查。
    let wt = review_worktree(st, &card.id);
    std::fs::create_dir_all(wt.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
    runner::git(work_dir, &["worktree", "add", "--detach", &wt.to_string_lossy(), &end]).await?;
    let out = runner::exec_claude_as(settings, &wt, &Mode::ReadOnly, &[], &prompt, agent.model.as_deref()).await;
    let _ = runner::git(work_dir, &["worktree", "remove", "--force", &wt.to_string_lossy()]).await;
    let _ = std::fs::remove_dir_all(&wt);
    let verdict = orch::parse_review(&out?)?;
    bs::set_verdict(pool, &card.id, if verdict.pass { "pass" } else { "fail" }, &verdict.reasons).await.map_err(|e| e.to_string())?;
    Ok(if verdict.pass { "審查通過".to_string() } else { format!("審查不通過：{}", verdict.reasons.join("；")) })
}

// ---- 拆解與相依 ----

/// 規格在 `end` 版的全文：`start..end` 之間新增或修改的檔案，串成一段（太長就截斷）。
/// 實作的 Coder 在自己的分支上看不到規格撰寫師分支上的檔案，所以實作卡要附全文。
async fn spec_text(work_dir: &Path, start: Option<&str>, end: &str) -> String {
    let range = match start {
        Some(s) => format!("{s}..{end}"),
        None => format!("{end}^..{end}"),
    };
    let Ok(names) = runner::git(work_dir, &["diff", "--name-only", &range]).await else { return String::new() };
    let mut out = String::new();
    for f in names.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(text) = runner::git(work_dir, &["show", &format!("{end}:{f}")]).await else { continue };
        out.push_str(&format!("### {f}\n\n{text}\n\n"));
        if out.chars().count() > SPEC_TEXT_LIMIT {
            let cut: String = out.chars().take(SPEC_TEXT_LIMIT).collect();
            return format!("{cut}\n\n（規格太長，已截斷；完整內容在版本 {end}）\n");
        }
    }
    out
}

/// 這張卡的前置是否都滿足：在別位工程師手上的前置要已通過審查；同一位的靠佇列順序；找不到的不擋。
fn deps_ready(card: &BoardCard, cards: &[BoardCard], agents: &[BoardAgent]) -> bool {
    let (has_reviewer, has_orch) = (agents.iter().any(|a| a.kind == "reviewer"), agents.iter().any(|a| a.kind == "orchestrator"));
    orch::deps_of(card).iter().filter_map(|d| cards.iter().find(|c| c.id == *d)).all(|dep| {
        if dep.agent_id == card.agent_id {
            return true;
        }
        let chain: Vec<BoardCard> = cards.iter().filter(|c| c.module_id.as_deref() == Some(dep.id.as_str()) && matches!(c.card_kind.as_str(), "review" | "fix" | "plan")).cloned().collect();
        matches!(orch::module_status(dep, &chain, has_reviewer, has_orch), ModuleState::Passed { .. })
    })
}

/// 開工前把別位工程師「已通過審查的版本」合併進這位的 worktree（已經在裡面就略過）。衝突就中止並讓這張卡失敗。
async fn premerge_dependencies(st: &AppState, wt: &Path, card: &BoardCard) -> Result<(), String> {
    let pool = &st.pools.intake;
    let module = if card.card_kind == "fix" { bs::get_card(pool, card.module_id.as_deref().unwrap_or("")).await.map_err(|e| e.to_string())?.unwrap_or_else(|| card.clone()) } else { card.clone() };
    let deps = orch::deps_of(&module);
    if deps.is_empty() {
        return Ok(());
    }
    let agents = bs::list_agents(pool, &card.project_id).await.map_err(|e| e.to_string())?;
    let cards = bs::project_cards(pool, &card.project_id).await.map_err(|e| e.to_string())?;
    let (has_reviewer, has_orch) = (agents.iter().any(|a| a.kind == "reviewer"), agents.iter().any(|a| a.kind == "orchestrator"));
    for dep in deps.iter().filter_map(|d| cards.iter().find(|c| c.id == *d)).filter(|d| d.agent_id != module.agent_id) {
        let chain: Vec<BoardCard> = cards.iter().filter(|c| c.module_id.as_deref() == Some(dep.id.as_str()) && matches!(c.card_kind.as_str(), "review" | "fix" | "plan")).cloned().collect();
        let ModuleState::Passed { sha: Some(sha) } = orch::module_status(dep, &chain, has_reviewer, has_orch) else { return Err(format!("前置「{}」還沒有通過審查的版本", dep.title)) };
        if runner::git(wt, &["merge-base", "--is-ancestor", &sha, "HEAD"]).await.is_ok() {
            continue;
        }
        if let Err(e) = runner::git(wt, &["merge", "--no-ff", "-m", &format!("預先合併前置：{}", dep.title.chars().take(40).collect::<String>()), &sha]).await {
            let _ = runner::git(wt, &["merge", "--abort"]).await;
            return Err(format!("把前置「{}」合併進來時衝突（已還原）：{e}", dep.title));
        }
    }
    Ok(())
}

/// 最閒的 Coder：先比待派送＋處理中的卡數，再比做過幾張。
fn pick_least_busy<'a>(candidates: &'a [&BoardAgent], load: &std::collections::HashMap<String, (usize, usize)>) -> Option<&'a BoardAgent> {
    candidates.iter().copied().min_by_key(|a| load.get(&a.id).copied().unwrap_or((0, 0)))
}

async fn run_plan(st: &Arc<AppState>, settings: &runner::RunnerSettings, work_dir: &Path, agent: &BoardAgent, card: &BoardCard) -> Result<String, String> {
    let pool = &st.pools.intake;
    let module = bs::get_card(pool, card.module_id.as_deref().unwrap_or("")).await.map_err(|e| e.to_string())?.ok_or("找不到要拆解的規格卡")?;
    let end = card.end_sha.clone().ok_or("拆解卡沒有記下規格的版本")?;
    let agents = bs::list_agents(pool, &card.project_id).await.map_err(|e| e.to_string())?;
    let candidates: Vec<&BoardAgent> = agents.iter().filter(|a| a.kind == "coder" && !a.produces_spec).collect();
    if candidates.is_empty() {
        return Err("沒有可以接手實作的工程師：專案裡要有至少一位「不產出規格」的 Coder".into());
    }
    let spec = spec_text(work_dir, card.start_sha.as_deref(), &end).await;
    let implementers: Vec<(String, String)> = candidates.iter().map(|a| (a.name.clone(), a.role.clone())).collect();
    let prompt = orch::plan_prompt(agent, &module, &implementers, &spec);
    // 在規格那一版的一次性 worktree 裡唯讀拆解（它可以自己讀專案裡的其他檔案）。
    let wt = review_worktree(st, &card.id);
    std::fs::create_dir_all(wt.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
    runner::git(work_dir, &["worktree", "add", "--detach", &wt.to_string_lossy(), &end]).await?;
    let out = runner::exec_claude_as(settings, &wt, &Mode::ReadOnly, &[], &prompt, agent.model.as_deref()).await;
    let _ = runner::git(work_dir, &["worktree", "remove", "--force", &wt.to_string_lossy()]).await;
    let _ = std::fs::remove_dir_all(&wt);
    let plan = orch::parse_plan(&out?)?;

    // 建實作卡：照最閒的工程師分；相依的索引換成前面建出來的卡 id。
    let current = bs::project_cards(pool, &card.project_id).await.map_err(|e| e.to_string())?;
    let mut load: std::collections::HashMap<String, (usize, usize)> = std::collections::HashMap::new();
    for c in &candidates {
        let busy = current.iter().filter(|t| t.agent_id.as_deref() == Some(c.id.as_str()) && matches!(t.status.as_str(), "queued" | "running")).count();
        let total = current.iter().filter(|t| t.agent_id.as_deref() == Some(c.id.as_str()) && t.card_kind == "work").count();
        load.insert(c.id.clone(), (busy, total));
    }
    let mut created: Vec<String> = Vec::new();
    for t in &plan.tasks {
        let coder = pick_least_busy(&candidates, &load).ok_or("沒有可以接手實作的工程師")?;
        let deps: Vec<&String> = t.depends_on.iter().filter_map(|&i| created.get(i)).collect();
        let deps_json = (!deps.is_empty()).then(|| serde_json::to_string(&deps).unwrap_or_default());
        let body = orch::impl_body(t, plan.interfaces.as_deref(), &module.title, &spec);
        let c = bs::create_workflow_card(pool, NewWorkflowCard { project_id: &card.project_id, title: &t.title, body: &body, kind: "work", agent_id: Some(&coder.id), module_id: None, review_of: None, status: QUEUED, depends_on: deps_json.as_deref(), from_spec: Some(&module.id) }).await.map_err(|e| e.to_string())?;
        let e = load.entry(coder.id.clone()).or_insert((0, 0));
        e.0 += 1;
        e.1 += 1;
        created.push(c.id);
    }
    Ok(format!("拆成 {} 張實作卡", created.len()))
}

// ---- 核准合併 ----

/// 管理員核准：把每個 Coder 的分支以 `--no-ff` 合併進專案資料夾目前的分支。
/// 任何一個衝突就中止那一個（不留半套），回報原因；已合併成功的保持合併。全部成功才把合併卡結案並清掉 worktree。
pub async fn merge_approved(st: &AppState, card: &BoardCard) -> Result<String, String> {
    if card.card_kind != "merge" || card.status == bs::DONE {
        return Err("這不是待核准的合併卡".into());
    }
    let pool = &st.pools.intake;
    let work_dir = work_dir_of(st, &card.project_id).await?;
    let agents = bs::list_agents(pool, &card.project_id).await.map_err(|e| e.to_string())?;
    // 還有模組沒審查通過或正在處理：合併會刪掉它們還在用的 worktree，先等。
    let cards = bs::project_cards(pool, &card.project_id).await.map_err(|e| e.to_string())?;
    if orch::merge_status(&cards, &agents) == orch::MergeStatus::NotReady {
        return Err("還有模組正在處理或還沒審查通過，現在合併會動到它們用的工作資料夾；等全部通過後再核准".into());
    }
    let mut merged = Vec::new();
    for a in agents.iter().filter(|a| a.kind == "coder") {
        let branch = agent_branch(&a.id);
        if runner::git(&work_dir, &["rev-parse", "--verify", &format!("refs/heads/{branch}")]).await.is_err() {
            continue;
        }
        let ahead = runner::git(&work_dir, &["rev-list", "--count", &format!("HEAD..{branch}")]).await?.trim().parse::<u32>().unwrap_or(0);
        if ahead > 0 {
            let msg = format!("合併 Agent「{}」的成果", a.name);
            if let Err(e) = runner::git(&work_dir, &["merge", "--no-ff", "-m", &msg, &branch]).await {
                let _ = runner::git(&work_dir, &["merge", "--abort"]).await;
                return Err(format!("合併「{}」失敗（已還原這一個）：{e}", a.name));
            }
            merged.push(a.name.clone());
        }
    }
    for a in agents.iter().filter(|a| a.kind == "coder") {
        let wt = agent_worktree(st, &a.id);
        let _ = runner::git(&work_dir, &["worktree", "remove", "--force", &wt.to_string_lossy()]).await;
        let _ = std::fs::remove_dir_all(&wt);
        let _ = runner::git(&work_dir, &["branch", "-D", &agent_branch(&a.id)]).await;
    }
    let _ = runner::git(&work_dir, &["worktree", "prune"]).await;
    let summary = if merged.is_empty() { "沒有需要合併的改動".to_string() } else { format!("已合併：{}", merged.join("、")) };
    bs::complete_card(pool, &card.id, "success", &summary).await.map_err(|e| e.to_string())?;
    Ok(summary)
}

pub fn start(st: &Arc<AppState>) {
    let st2 = st.clone();
    let h = tokio::spawn(async move {
        loop {
            tick_once(&st2).await;
            tokio::time::sleep(runner::TICK_EVERY).await;
        }
    });
    *st.orchestrator_task.lock() = Some(h);
}

pub fn stop(st: &AppState) {
    if let Some(h) = st.orchestrator_task.lock().take() {
        h.abort();
    }
}
