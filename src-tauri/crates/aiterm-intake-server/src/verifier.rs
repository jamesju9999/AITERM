//! Server 驗收者：全部模組審查通過後，先在**拋棄式的整合 worktree** 用 Claude Code（Bash＋無頭瀏覽器）驗收，
//! 通過才建「合併待核准」；不通過就依失敗項開修正卡（`vfix`）給對應工程師，最多 2 輪。
//! 設計見 `docs/superpowers/specs/2026-11-26-server-verifier-design.md`。
//!
//! - `advance_verification`：每個 tick 由 `orchestrator::advance` 呼叫，依閘門（`verifier::gate`）建驗收卡／合併卡／修正卡。
//! - `run_verify`：處理一張驗收卡（建整合 worktree、跑 Claude、讀 verify.json、存報告、一定清理）。
//! - 驗收被打斷（重啟、額度用完）時**重新驗收，不接續對話**：整合 worktree 是拋棄式的，伺服器與瀏覽器的狀態都已消失，接續沒有意義。
use crate::orchestrator::{agent_branch, create_merge_card};
use crate::runner::{self, Mode, RunnerSettings, SessionUse};
use crate::state::AppState;
use crate::{deny, verify_env};
use aiterm_intake_core::board_store::{self as bs, BoardAgent, BoardCard, NewWorkflowCard, QUEUED};
use aiterm_intake_core::orchestration::{self as orch, ModuleState};
use aiterm_intake_core::verifier::{self as vcore, Gate, VerifyCardView};
use std::path::{Path, PathBuf};

/// 驗收一輪的逾時與回合上限下限（比一般卡寬：要建置、啟動伺服器、開瀏覽器）。
const VERIFY_MIN_TIMEOUT_SECS: u64 = 1800;
const VERIFY_MIN_TURNS: u32 = 120;

/// 這張驗收卡的資料夾（verify.json 與證據）：`<data>/verify/<card id>/`。
pub fn card_dir(st: &AppState, card_id: &str) -> PathBuf {
    st.data_dir.join("verify").join(card_id)
}

fn integration_worktree(st: &AppState, card_id: &str) -> PathBuf {
    st.data_dir.join("worktrees").join(format!("verify-{}", card_id.chars().take(12).collect::<String>()))
}

fn chain_of(cards: &[BoardCard], module_id: &str) -> Vec<BoardCard> {
    cards.iter().filter(|c| c.module_id.as_deref() == Some(module_id) && matches!(c.card_kind.as_str(), "review" | "fix" | "vfix" | "plan")).cloned().collect()
}

/// 各模組目前通過的版本（`(模組 id, sha)`）。
fn passed_versions(mods: &[&BoardCard], cards: &[BoardCard], agents: &[BoardAgent]) -> Vec<(String, Option<String>)> {
    let (has_reviewer, has_orch) = (agents.iter().any(|a| a.kind == "reviewer"), agents.iter().any(|a| a.kind == "orchestrator"));
    mods.iter()
        .filter_map(|m| match orch::module_status(m, &chain_of(cards, &m.id), has_reviewer, has_orch) {
            ModuleState::Passed { sha } => Some((m.id.clone(), sha)),
            _ => None,
        })
        .collect()
}

async fn verify_views(pool: &sqlx::SqlitePool, project_id: &str) -> Result<Vec<VerifyCardView>, sqlx::Error> {
    let mut out = Vec::new();
    for c in bs::verify_cards(pool, project_id).await? {
        let signature = bs::get_verify(pool, &c.id).await?.map(|v| v.signature).unwrap_or_default();
        out.push(VerifyCardView { id: c.id, signature, status: c.status, outcome: c.outcome, verdict: c.verdict });
    }
    Ok(out)
}

/// 建一張驗收卡（排進待派送，指派給驗收者），並記下簽章與這次要用的 3 個埠。
pub async fn create_verify_card(pool: &sqlx::SqlitePool, project_id: &str, verifier: &BoardAgent, full_signature: &str) -> Result<BoardCard, sqlx::Error> {
    let card = bs::create_workflow_card(
        pool,
        NewWorkflowCard {
            project_id,
            title: "驗收：整合後的成果",
            body: "把各工程師通過審查的成果合併到拋棄式的工作資料夾，實際建置、執行、操作，照驗收條件逐條確認。",
            kind: "verify",
            agent_id: Some(&verifier.id),
            module_id: None,
            review_of: None,
            status: QUEUED,
            depends_on: None,
            from_spec: None,
        },
    )
    .await?;
    bs::set_verify_meta(pool, &card.id, full_signature, &verify_env::pick_free_ports(3)).await?;
    Ok(card)
}

/// 簽章基底（各模組通過的版本）加重試序號：`<基底>#<序號>`。基底相同的驗收卡共用同一個基底。
pub fn base_signature(passed: &[(String, Option<String>)]) -> String {
    vcore::signature(passed, 0)
}

/// 目前這組版本該用的完整簽章：基底相同的最新一張驗收卡的簽章；還沒有就是 `<基底>#0`。
pub fn current_signature(views: &[VerifyCardView], base: &str) -> String {
    let prefix = format!("{base}#");
    views.iter().rev().find(|v| v.signature.starts_with(&prefix)).map(|v| v.signature.clone()).unwrap_or_else(|| format!("{prefix}0"))
}

/// 下一個重新驗收要用的簽章（基底相同的卡已有幾張就是第幾號）。
pub fn next_signature(views: &[VerifyCardView], base: &str) -> String {
    let prefix = format!("{base}#");
    format!("{prefix}{}", views.iter().filter(|v| v.signature.starts_with(&prefix)).count())
}

/// 全部模組通過（`merge_status == Ready`）且啟用驗收時，依閘門決定下一步。回傳這次新建的卡數。
pub async fn advance_verification(st: &AppState, pid: &str, verifier: &BoardAgent, cards: &[BoardCard], agents: &[BoardAgent], mods: &[&BoardCard]) -> Result<usize, sqlx::Error> {
    let pool = &st.pools.intake;
    let passed = passed_versions(mods, cards, agents);
    let base = base_signature(&passed);
    let views = verify_views(pool, pid).await?;
    let current = current_signature(&views, &base);
    match vcore::gate(&views, &current) {
        Gate::NeedsVerify => {
            create_verify_card(pool, pid, verifier, &current).await?;
            Ok(1)
        }
        Gate::Passed => {
            let id = views.iter().rev().find(|v| v.signature == current).map(|v| v.id.clone()).unwrap_or_default();
            let v = bs::get_verify(pool, &id).await?;
            let manual = v.as_ref().map_or(0, |v| v.manual_count);
            let by = v.as_ref().and_then(|v| v.overridden_by.clone());
            let note = match (by, manual) {
                (Some(name), _) => format!("驗收由管理員「{name}」放行"),
                (None, 0) => "也通過了驗收".to_string(),
                (None, n) => format!("也通過了驗收（{n} 項需要你確認）"),
            };
            create_merge_card(pool, pid, agents, mods, &note).await?;
            Ok(1)
        }
        Gate::Failed { rounds } => {
            let id = views.iter().rev().find(|v| v.signature == current).map(|v| v.id.clone()).unwrap_or_default();
            open_fix_cards(st, pid, &id, rounds, cards, agents, mods).await
        }
        Gate::Waiting | Gate::NeedsHuman => Ok(0),
    }
}

/// 驗收不通過：依失敗項對每個有對應模組的群組開一張修正卡給原工程師；沒有任何一個對得上模組就標 attention 等管理員。
async fn open_fix_cards(st: &AppState, pid: &str, verify_card_id: &str, round: usize, cards: &[BoardCard], agents: &[BoardAgent], mods: &[&BoardCard]) -> Result<usize, sqlx::Error> {
    let pool = &st.pools.intake;
    let report = match bs::get_verify(pool, verify_card_id).await?.and_then(|v| v.report).and_then(|r| serde_json::from_str::<vcore::VerifyReport>(&r).ok()) {
        Some(r) => r,
        None => {
            bs::set_verdict(pool, verify_card_id, "attention", &["驗收報告讀不出來".to_string()]).await?;
            return Ok(0);
        }
    };
    let ids: Vec<String> = mods.iter().map(|m| m.id.clone()).collect();
    let attr = vcore::attribute(&report, &ids);
    if attr.by_module.is_empty() {
        let reasons: Vec<String> = attr.unattributed.iter().map(|i| i.criterion.clone()).collect();
        bs::set_verdict(pool, verify_card_id, "attention", &reasons).await?;
        return Ok(0);
    }
    let mut created = 0;
    for (module_id, items) in &attr.by_module {
        let Some(m) = mods.iter().find(|m| m.id == *module_id) else { continue };
        let chain = chain_of(cards, &m.id);
        let tail = orch::chain_tail(m, &chain);
        let title = format!("驗收退回（第 {round} 輪）：{}", m.title);
        bs::create_workflow_card(
            pool,
            NewWorkflowCard {
                project_id: pid,
                title: &title,
                body: &vcore::vfix_body(&m.title, &m.body, items, verify_card_id, round),
                kind: "vfix",
                agent_id: m.agent_id.as_deref(),
                module_id: Some(&m.id),
                review_of: Some(&tail.id),
                status: QUEUED,
                depends_on: None,
                from_spec: None,
            },
        )
        .await?;
        created += 1;
    }
    let _ = agents;
    Ok(created)
}

/// 處理一張驗收卡。成功回傳摘要（驗收通過／退回）；任何無法完成驗收的情況回傳 `Err`（驗收卡失敗，可由管理員重新驗收）。
pub async fn run_verify(st: &AppState, settings: &RunnerSettings, work_dir: &Path, agent: &BoardAgent, card: &BoardCard) -> Result<String, String> {
    let pool = &st.pools.intake;
    let meta = bs::get_verify(pool, &card.id).await.map_err(|e| e.to_string())?.ok_or("這張驗收卡沒有簽章與埠的記錄")?;
    let ports = if meta.ports.is_empty() { verify_env::pick_free_ports(3) } else { meta.ports.clone() };
    // 上一次被打斷留下的東西先清掉：佔埠的伺服器、孤兒 claude、舊的整合 worktree。
    verify_env::stop_listeners(&ports);
    if let Ok(Some(old)) = bs::get_session(pool, &card.id).await {
        runner::kill_orphans(&[old.session_id.clone()]);
        runner::remove_session_file(&old.session_id);
        bs::clear_session(pool, &card.id).await.ok();
    }
    let wt = integration_worktree(st, &card.id);
    cleanup_worktree(work_dir, &wt).await;

    let dir = card_dir(st, &card.id);
    let evidence = dir.join("evidence");
    let json_path = dir.join("verify.json");
    let _ = std::fs::remove_file(&json_path);
    std::fs::create_dir_all(&evidence).map_err(|e| format!("無法建立驗收資料夾：{e}"))?;

    let result = run_in_worktree(st, settings, work_dir, agent, card, &wt, &dir, &evidence, &json_path, &ports).await;
    // 無論成敗都收尾：佔埠的伺服器、整合 worktree、對話記錄與檔案。
    verify_env::stop_listeners(&ports);
    cleanup_worktree(work_dir, &wt).await;
    if let Ok(Some(sid)) = bs::clear_session(pool, &card.id).await {
        runner::remove_session_file(&sid);
    }
    result
}

async fn cleanup_worktree(work_dir: &Path, wt: &Path) {
    let _ = runner::git(work_dir, &["worktree", "remove", "--force", &wt.to_string_lossy()]).await;
    let _ = std::fs::remove_dir_all(wt);
    let _ = runner::git(work_dir, &["worktree", "prune"]).await;
}

#[allow(clippy::too_many_arguments)]
async fn run_in_worktree(st: &AppState, settings: &RunnerSettings, work_dir: &Path, agent: &BoardAgent, card: &BoardCard, wt: &Path, dir: &Path, evidence: &Path, json_path: &Path, ports: &[u16]) -> Result<String, String> {
    let pool = &st.pools.intake;
    let agents = bs::list_agents(pool, &card.project_id).await.map_err(|e| e.to_string())?;
    let cards = bs::project_cards(pool, &card.project_id).await.map_err(|e| e.to_string())?;
    let mods = orch::modules(&cards, &agents);

    // 1. 整合 worktree：專案目前的版本，依序併入各工程師的分支。衝突就整個驗收失敗（合併核准時也一樣會衝突）。
    runner::git(work_dir, &["rev-parse", "--verify", "HEAD"]).await.map_err(|_| "工作資料夾不是有提交紀錄的 git 專案，無法驗收".to_string())?;
    std::fs::create_dir_all(wt.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
    runner::git(work_dir, &["worktree", "add", "--detach", &wt.to_string_lossy(), "HEAD"]).await?;
    for a in agents.iter().filter(|a| a.kind == "coder") {
        let branch = agent_branch(&a.id);
        if runner::git(work_dir, &["rev-parse", "--verify", &format!("refs/heads/{branch}")]).await.is_err() {
            continue;
        }
        let ahead = runner::git(wt, &["rev-list", "--count", &format!("HEAD..{branch}")]).await?.trim().parse::<u32>().unwrap_or(0);
        if ahead == 0 {
            continue;
        }
        if let Err(e) = runner::git(wt, &["merge", "--no-ff", "-m", &format!("驗收用：合併「{}」", a.name), &branch]).await {
            let _ = runner::git(wt, &["merge", "--abort"]).await;
            return Err(format!("合併衝突：「{}」的分支無法併進整合版本，驗收無法進行（核准合併時也會衝突）。{e}", a.name));
        }
    }

    // 2. 提示詞：模組清單、規格全文（有規格模組時）、埠、瀏覽器。
    let has_browser = verify_env::node_available();
    let module_list: Vec<(String, String, String)> = mods.iter().map(|m| (m.id.clone(), m.title.clone(), m.body.clone())).collect();
    let mut spec = String::new();
    for m in mods.iter().filter(|m| m.is_spec) {
        if let Some(end) = m.end_sha.as_deref() {
            spec.push_str(&crate::orchestrator::spec_text(work_dir, m.start_sha.as_deref(), end).await);
        }
    }
    let prompt = vcore::verify_prompt(&module_list, &spec, &json_path.to_string_lossy(), &evidence.to_string_lossy(), ports, has_browser);

    // 3. 瀏覽器設定與禁止清單。
    let mcp_config = if has_browser {
        let p = dir.join("mcp.json");
        std::fs::write(&p, verify_env::mcp_config_json(evidence, verify_env::detect_browser())).map_err(|e| e.to_string())?;
        Some(p.to_string_lossy().into_owned())
    } else {
        None
    };
    let denylist = deny::effective_for(pool, &card.project_id).await;
    let mode = Mode::Verify { deny: denylist, browser: has_browser, mcp_config };

    // 4. 跑。對話 id 只用來在 server 被強制結束後認出孤兒 claude（重啟時 `kill_orphans`），驗收不接續對話。
    let sid = uuid::Uuid::new_v4().to_string();
    bs::set_session(pool, &card.id, &sid, None).await.map_err(|e| e.to_string())?;
    let mut s = settings.clone();
    s.timeout_secs = s.timeout_secs.max(VERIFY_MIN_TIMEOUT_SECS);
    s.max_turns = s.max_turns.max(VERIFY_MIN_TURNS);
    let run = runner::exec_claude_full(&s, wt, &mode, &[], &prompt, agent.model.as_deref(), &SessionUse::New(sid)).await?;

    // 5. 讀結論。沒寫、讀不懂、品質不合格都算失敗，不會當成通過。
    let text = std::fs::read_to_string(json_path).map_err(|_| "驗收者沒有寫出 verify.json（驗收沒有完成）".to_string())?;
    let report = vcore::parse_verify_json(&text).map_err(|e| format!("verify.json 讀不懂：{e}"))?;
    if let Err(bad) = vcore::check_quality(&report) {
        return Err(format!("驗收報告品質不合格：{}", bad.join("；")));
    }
    let manual = vcore::manual_count(&report);
    let report_json = serde_json::to_string(&report).map_err(|e| e.to_string())?;
    bs::save_verify_report(pool, &card.id, &report_json, &run.denied, manual as i64).await.map_err(|e| e.to_string())?;
    let reasons: Vec<String> = report.items.iter().filter(|i| i.status == "fail").map(|i| i.criterion.clone()).collect();
    bs::set_verdict(pool, &card.id, &report.verdict, &reasons).await.map_err(|e| e.to_string())?;
    Ok(if report.verdict == "pass" {
        if manual > 0 { format!("驗收通過（{manual} 項需要你確認）") } else { "驗收通過".to_string() }
    } else {
        format!("驗收退回 {} 項：{}", reasons.len(), reasons.join("；"))
    })
}

/// server 啟動時：上一個行程留下的驗收伺服器（佔著驗收埠）一併收掉。
pub async fn cleanup_stale(st: &AppState) {
    if let Ok(ports) = bs::all_verify_ports(&st.pools.intake).await {
        let stopped = verify_env::stop_listeners(&ports);
        if !stopped.is_empty() {
            log::warn!("上一個行程留下 {} 個驗收用的伺服器，已結束", stopped.len());
        }
    }
}
