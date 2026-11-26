//! 共用看板的定時器：每天／每週／間隔建一輪卡片，**只由 server 觸發**——多人同時連線也只會建一次。
//!
//! 每次掃描：先結算上一輪（成功歸零連續失敗並把完成的卡歸檔、失敗加一、連續 3 輪失敗自動停用），
//! 再依週期規則判斷到點了沒。上一輪還沒結案就跳過（記一筆 skipped），不會堆積。
//! 週期的時間點預設是 server 主機的本地時間；定時器設了時區（`tz`）就用那個時區的牆上時間。卡片建好後，專案開了「由 server 執行」就由 server 的編排做，
//! 沒開就留在待派送，由成員領取。
use crate::state::AppState;
use aiterm_intake_core::board_store::{self as bs, BoardSchedule, NewWorkflowCard, QUEUED};
use aiterm_intake_core::host::Notifier;
use aiterm_intake_core::orchestration::{self as orch, ModuleState};
use aiterm_intake_core::schedule::{due_at, local_ts};
use chrono::NaiveDateTime;
use std::sync::Arc;
use std::time::Duration;

/// 多久掃一次。
pub const TICK_EVERY: Duration = Duration::from_secs(20);

/// 同一時間只讓一個掃描在跑：迴圈與「立即執行」重疊時不會同一時間點建兩次。
static TICK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundResult {
    Pending,
    Success,
    Failed,
    /// 沒有進行中的輪（或那張卡被刪了）。
    Gone,
}

#[derive(Debug, PartialEq)]
pub enum Fired {
    Created { card_id: String },
    Skipped,
}

pub fn card_title(name: &str) -> String {
    format!("定時：{name}")
}

fn work_body(instruction: &str) -> String {
    format!("這是定時觸發的工作（每次觸發都是新的一輪，請依下面的指示完成，並照一般流程交付）：\n\n{}\n", instruction.trim())
}

/// 上一輪現在的狀態。專案由 server 編排時，看整條模組鏈（審查通過才算成功）；
/// 否則（成員領取）看那張卡自己的結果——沒人建審查卡，等審查會永遠等不到。
pub async fn round_result(st: &AppState, s: &BoardSchedule) -> Result<RoundResult, String> {
    let pool = &st.pools.intake;
    let Some(id) = s.last_card_id.as_deref() else { return Ok(RoundResult::Gone) };
    let Some(card) = bs::get_card(pool, id).await.map_err(|e| e.to_string())? else { return Ok(RoundResult::Gone) };
    if !crate::orchestrator::enabled(pool, &s.project_id).await {
        return Ok(match (card.status.as_str(), card.outcome.as_deref()) {
            (bs::DONE, Some("success")) => RoundResult::Success,
            (bs::DONE, _) => RoundResult::Failed,
            _ => RoundResult::Pending,
        });
    }
    let agents = bs::list_agents(pool, &s.project_id).await.map_err(|e| e.to_string())?;
    let cards = bs::project_cards(pool, &s.project_id).await.map_err(|e| e.to_string())?;
    let chain: Vec<_> = cards.iter().filter(|c| c.module_id.as_deref() == Some(card.id.as_str()) && matches!(c.card_kind.as_str(), "review" | "fix" | "plan")).cloned().collect();
    Ok(match orch::module_status(&card, &chain, agents.iter().any(|a| a.kind == "reviewer"), agents.iter().any(|a| a.kind == "orchestrator")) {
        ModuleState::Passed { .. } => RoundResult::Success,
        ModuleState::Exhausted | ModuleState::Stuck(_) => RoundResult::Failed,
        _ => RoundResult::Pending,
    })
}

/// 判定上一輪並更新狀態；回傳這次是不是有結案（成功／失敗）。
async fn settle(st: &AppState, s: &BoardSchedule, now: i64) -> Result<bool, String> {
    let pool = &st.pools.intake;
    let card = s.last_card_id.clone();
    match round_result(st, s).await? {
        RoundResult::Pending => Ok(false),
        RoundResult::Gone => {
            if card.is_some() {
                bs::clear_schedule_round(pool, &s.id).await.map_err(|e| e.to_string())?;
            }
            Ok(false)
        }
        RoundResult::Success => {
            bs::settle_schedule_round(pool, &s.id, true).await.map_err(|e| e.to_string())?;
            bs::record_schedule_run(pool, &s.id, now, card.as_deref(), "success").await.map_err(|e| e.to_string())?;
            if let Some(c) = card.as_deref() {
                bs::archive_round(pool, c, now).await.map_err(|e| e.to_string())?;
            }
            Ok(true)
        }
        RoundResult::Failed => {
            let streak = bs::settle_schedule_round(pool, &s.id, false).await.map_err(|e| e.to_string())?;
            bs::record_schedule_run(pool, &s.id, now, card.as_deref(), "failed").await.map_err(|e| e.to_string())?;
            if streak >= bs::SCHEDULE_MAX_FAIL_STREAK {
                bs::disable_schedule(pool, &s.id, bs::REASON_FAILURES).await.map_err(|e| e.to_string())?;
            }
            Ok(true)
        }
    }
}

/// 建一輪。上一輪還沒結案 → 跳過（記一筆 skipped），照樣推進基準時間，不然每個 tick 都會再試。
pub async fn fire(st: &AppState, s: &BoardSchedule, now: i64) -> Result<Fired, String> {
    let pool = &st.pools.intake;
    let agent = bs::get_agent(pool, &s.agent_id).await.map_err(|e| e.to_string())?.ok_or("目標角色已不存在")?;
    if round_result(st, s).await? == RoundResult::Pending {
        bs::mark_schedule_skipped(pool, &s.id, now, s.last_card_id.as_deref()).await.map_err(|e| e.to_string())?;
        return Ok(Fired::Skipped);
    }
    if agent.kind != "coder" {
        return Err(format!("定時器只能叫起工程師，不能是：{}", agent.kind));
    }
    let card = bs::create_workflow_card(
        pool,
        NewWorkflowCard { project_id: &s.project_id, title: &card_title(&s.name), body: &work_body(&s.instruction), kind: "work", agent_id: Some(&agent.id), module_id: None, review_of: None, status: QUEUED, depends_on: None, from_spec: None },
    )
    .await
    .map_err(|e| e.to_string())?;
    bs::mark_schedule_fired(pool, &s.id, now, &card.id).await.map_err(|e| e.to_string())?;
    Ok(Fired::Created { card_id: card.id })
}

/// 掃一次（時間由呼叫端給，測試才能控制）。回傳這次建出的卡數。
pub async fn tick_at(st: &Arc<AppState>, now: NaiveDateTime) -> usize {
    let _one_at_a_time = TICK.lock().await;
    let pool = &st.pools.intake;
    let now_ts = local_ts(now);
    let mut fired = 0;
    let Ok(all) = bs::all_schedules(pool).await else { return 0 };
    for s in all {
        if matches!(settle(st, &s, now_ts).await, Ok(true)) {
            st.events.changed();
        }
        let Ok(Some(s)) = bs::get_schedule(pool, &s.id).await else { continue };
        if !s.enabled {
            continue;
        }
        if !matches!(bs::get_agent(pool, &s.agent_id).await, Ok(Some(_))) {
            bs::disable_schedule(pool, &s.id, bs::REASON_AGENT_GONE).await.ok();
            st.events.changed();
            continue;
        }
        let Ok(spec) = s.spec() else { continue };
        // 時間點是設定的時區（`spec.tz`）的牆上時間；沒設就是 server 主機的本地時間。
        if !due_at(&s.schedule_kind, &spec, s.last_fired_at, now_ts) {
            continue;
        }
        match fire(st, &s, now_ts).await {
            Ok(Fired::Created { .. }) => {
                fired += 1;
                crate::audit::record(&st.pools.server, "server", "board.schedule_fire", &s.id).await;
                st.events.changed();
            }
            Ok(Fired::Skipped) => st.events.changed(),
            Err(e) => log::warn!("定時器「{}」觸發失敗：{e}", s.name),
        }
    }
    fired
}

/// 立即執行一次：走 `fire` 的同一條路（上一輪未結案會被擋下並記為跳過），
/// 但不吃掉下一個預定時間點——`last_fired_at` 還原成執行前的值。被擋下時回傳 `None`。
pub async fn run_now(st: &Arc<AppState>, id: &str) -> Result<Option<String>, String> {
    let _one_at_a_time = TICK.lock().await;
    let pool = &st.pools.intake;
    let s = bs::get_schedule(pool, id).await.map_err(|e| e.to_string())?.ok_or("找不到這個定時器")?;
    let before = s.last_fired_at;
    let out = fire(st, &s, chrono::Utc::now().timestamp()).await?;
    sqlx::query("UPDATE board_schedules SET last_fired_at = ? WHERE id = ?").bind(before).bind(id).execute(pool).await.map_err(|e| e.to_string())?;
    st.events.changed();
    Ok(match out {
        Fired::Created { card_id } => Some(card_id),
        Fired::Skipped => None,
    })
}

pub fn start(st: &Arc<AppState>) {
    let st2 = st.clone();
    let h = tokio::spawn(async move {
        loop {
            tokio::time::sleep(TICK_EVERY).await;
            tick_at(&st2, chrono::Local::now().naive_local()).await;
        }
    });
    *st.scheduler_task.lock() = Some(h);
}

pub fn stop(st: &AppState) {
    if let Some(h) = st.scheduler_task.lock().take() {
        h.abort();
    }
}
