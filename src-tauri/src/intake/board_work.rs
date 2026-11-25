//! 連到 server 時領取看板卡片：向 server 原子領取，在本機建一張**影子卡片**走現有的派工流程
//! （終端機分頁、worktree、對話紀錄都留在本機），再把狀態與結果回寫 server。
//!
//! - 每位成員為每個 server 專案設定「我電腦上的專案」：影子卡片放在哪個本機看板專案、在哪個資料夾執行。
//! - 影子卡片完成（或被刪掉、拖回計畫中）由本機定時掃描發現，回寫 server；處理中每 2 分鐘向 server 續約。
//! - 領取者失聯：server 的租約逾期後把卡片放回待派送，別人可以重領。
use super::board_remote;
use super::remote::{self, RemoteState};
use crate::projects::ProjectRegistry;
use crate::tasks::store;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, Runtime};

const MAP_KEY: &str = "board_map:";
/// 影子卡片上的標籤，讓使用者在本機看板認得它來自 server。
pub const SHADOW_LABEL: &str = "🌐 server";

/// 這位成員自己電腦上，某個 server 專案對應的看板專案與執行資料夾。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct BoardMap {
    /// 影子卡片放在哪個本機看板專案（`ProjectRegistry` 的專案 id）。
    pub board_project_id: String,
    /// Claude Code 在哪個資料夾執行（這台電腦上的專案資料夾）。
    pub work_dir: String,
}

pub async fn load_map(pool: &sqlx::SqlitePool, server_project_id: &str) -> Option<BoardMap> {
    let raw = aiterm_intake_core::store::get_setting(pool, &format!("{MAP_KEY}{server_project_id}")).await.ok().flatten()?;
    serde_json::from_str::<BoardMap>(&raw).ok().filter(|m| !m.board_project_id.is_empty() && !m.work_dir.is_empty())
}

/// 存對應。看板專案要是這台電腦上開著的，資料夾要存在。
pub async fn save_map(pool: &sqlx::SqlitePool, reg: &ProjectRegistry, server_project_id: &str, map: &BoardMap) -> Result<(), String> {
    if reg.get(&map.board_project_id).is_none() {
        return Err("找不到這個本機看板專案（可能已關閉）".into());
    }
    let dir = map.work_dir.trim();
    if dir.is_empty() || !std::path::Path::new(dir).is_dir() {
        return Err(format!("這台電腦上找不到這個資料夾：{dir}"));
    }
    let clean = BoardMap { board_project_id: map.board_project_id.clone(), work_dir: dir.to_string() };
    aiterm_intake_core::store::set_setting(pool, &format!("{MAP_KEY}{server_project_id}"), &serde_json::to_string(&clean).map_err(|e| e.to_string())?).await.map_err(|e| e.to_string())
}

fn changed<R: Runtime>(app: &AppHandle<R>) {
    let _ = app.emit("tasks-updated", ());
}

/// 領取一張卡片：先確認本機設定齊全，再向 server 領，最後在本機建影子卡片並排進派工佇列。
/// 任何一步失敗都不會留下「server 以為你在做、本機沒有」的狀態（領到之後本機建不起來就放回）。
pub async fn claim<R: Runtime>(app: &AppHandle<R>, card_id: &str) -> Result<(), String> {
    let (api, rs) = remote::active(app).ok_or("尚未連到 server")?;
    let db = app.try_state::<super::IntakeDb>().ok_or("intake 未就緒")?;
    let reg = app.try_state::<ProjectRegistry>().ok_or("看板未就緒")?;
    let card = board_remote::get_card(&api, &rs, card_id).await.map_err(|e| remote::fail(&rs, e))?;
    let map = load_map(&db.pool, &card.project_id).await.ok_or("請先設定「我電腦上的專案」：選一個本機看板專案與執行資料夾")?;
    let handle = reg.get(&map.board_project_id).ok_or("這個 server 專案對應的本機看板專案已不存在，請重新設定")?;
    if !std::path::Path::new(&map.work_dir).is_dir() {
        return Err(format!("執行資料夾不存在：{}", map.work_dir));
    }
    let claimed = board_remote::claim(&api, &rs, card_id).await.map_err(|e| remote::fail(&rs, e))?;
    match make_shadow(&handle.pool, &claimed, &map).await {
        Ok(local_id) => {
            rs.track_board_claim(card_id, &handle.id, &local_id);
            changed(app);
            Ok(())
        }
        Err(e) => {
            board_remote::release(&api, card_id).await.ok();
            Err(format!("無法在本機建立卡片，已放回 server：{e}"))
        }
    }
}

async fn make_shadow(pool: &sqlx::SqlitePool, card: &aiterm_intake_core::board_store::BoardCard, map: &BoardMap) -> Result<String, String> {
    let id = store::create_task(pool, &card.title, &card.body, &map.work_dir, card.parallel_ok, card.interactive).await.map_err(|e| e.to_string())?;
    store::set_label(pool, &id, Some(SHADOW_LABEL)).await.map_err(|e| e.to_string())?;
    let order = store::midpoint_between(pool, store::STATUS_QUEUED, None, None).await.map_err(|e| e.to_string())?;
    store::move_task(pool, &id, store::STATUS_QUEUED, order).await.map_err(|e| e.to_string())?;
    Ok(id)
}

/// 把領取的卡片放回 server（使用者不做了）。影子卡片留在本機當紀錄，但不再回寫。
pub async fn release<R: Runtime>(app: &AppHandle<R>, card_id: &str) -> Result<(), String> {
    let (api, rs) = remote::active(app).ok_or("尚未連到 server")?;
    rs.untrack_board_claim(card_id);
    board_remote::release(&api, card_id).await.map_err(|e| remote::fail(&rs, e))?;
    changed(app);
    Ok(())
}

/// 處理中每隔多久向 server 續約。
pub const HEARTBEAT_EVERY: Duration = Duration::from_secs(120);

/// 本機定時掃描：續約、把完成的影子卡片結果回寫 server、發現影子卡片不見或被拖回計畫中就放回。
pub async fn tick<R: Runtime>(app: &AppHandle<R>, heartbeat_every: Duration) {
    let Some((api, rs)) = remote::active(app) else { return };
    let Some(reg) = app.try_state::<ProjectRegistry>() else { return };
    let mut touched = false;
    for id in rs.board_due_for_heartbeat(heartbeat_every) {
        if !matches!(board_remote::heartbeat(&api, &id).await, Ok(true)) {
            // 租約被收回（逾期或被管理員放回）：結果不會被採用，不再追蹤。
            rs.untrack_board_claim(&id);
            touched = true;
        }
    }
    for (card_id, claim) in rs.board_claims() {
        let row = match reg.get(&claim.local_project_id) {
            Some(h) => store::get_task(&h.pool, &claim.local_task_id).await.ok().flatten(),
            None => None,
        };
        match row {
            None => {
                // 影子卡片被刪掉、或它的本機專案已關閉：不可能再有結果，放回 server。
                rs.untrack_board_claim(&card_id);
                board_remote::release(&api, &card_id).await.ok();
                touched = true;
            }
            Some(t) if t.status == store::STATUS_PLANNING => {
                rs.untrack_board_claim(&card_id);
                board_remote::release(&api, &card_id).await.ok();
                touched = true;
            }
            Some(t) if t.status == store::STATUS_DONE => {
                rs.untrack_board_claim(&card_id);
                let ok = t.outcome.as_deref() == Some("success");
                let error = if ok { None } else { Some(t.error_message.clone().unwrap_or_else(|| format!("本機處理沒有成功（{}）", t.outcome.clone().unwrap_or_default()))) };
                if let Err(e) = board_remote::result(&api, &card_id, if ok { "success" } else { "failed" }, t.ai_summary.as_deref(), error.as_deref()).await {
                    // 常見原因是租約逾期、卡片已被收回；結果不會被採用。
                    log::warn!("board: 交回 server 失敗：{e}");
                    remote::fail(&rs, e);
                }
                touched = true;
            }
            Some(_) => {}
        }
    }
    if touched {
        changed(app);
    }
}

#[allow(dead_code)]
fn _keep(_: &Arc<RemoteState>) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intake::remote::Saved;
    use aiterm_intake_core::api::Role;
    use aiterm_intake_core::board_store::{self as bs, BoardCard};
    use aiterm_intake_server::testkit::TestServer;
    use reqwest::Method;
    use serde_json::json;

    struct Rig {
        app: tauri::App<tauri::test::MockRuntime>,
        srv: TestServer,
        pid: String,
        board: crate::projects::ProjectHandle,
        work: tempfile::TempDir,
        _parent: tempfile::TempDir,
    }

    async fn pair(srv: &TestServer, name: &str) -> (aiterm_intake_client::api::Api, Saved) {
        let code = srv.invite(name, Role::Member).await;
        let paired = aiterm_intake_client::pair::pair(&srv.addr(), &code).await.unwrap();
        let api = aiterm_intake_client::api::Api::new(&srv.addr(), &paired.token, Some(paired.response.fingerprint.clone())).unwrap();
        (api, Saved { addr: srv.addr(), fingerprint: Some(paired.response.fingerprint), name: name.into(), role: Role::Member })
    }

    /// 一台已連線的 AITERM（假視窗）：有一個本機看板專案，也設好了「我電腦上的專案」對應。
    async fn rig(mapped: bool) -> Rig {
        let srv = TestServer::start().await;
        let pid = srv.project("範例科技").await;
        let app = tauri::test::mock_app();
        let h = app.handle().clone();
        let local = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        crate::intake::store::init_schema(&local).await.unwrap();
        h.manage(crate::intake::IntakeDb { pool: local });
        h.manage(Arc::new(RemoteState::default()));
        h.manage(Arc::new(crate::intake::line_server::LineServer::default()));
        h.manage(Arc::new(crate::intake::web::WebServer::default()));
        let reg = ProjectRegistry::new();
        let parent = tempfile::tempdir().unwrap();
        let board = reg.create(parent.path(), "本機看板", "").await.unwrap();
        h.manage(reg);
        let work = tempfile::tempdir().unwrap();
        let rs = h.state::<Arc<RemoteState>>().inner().clone();
        rs.set_attach_root(work.path().join("cache"));
        let (api, saved) = pair(&srv, "小華").await;
        remote::install(&h, saved, api).await;
        let rig = Rig { app, srv, pid, board, work, _parent: parent };
        if mapped {
            rig.map().await.unwrap();
        }
        rig
    }

    impl Rig {
        fn rs(&self) -> Arc<RemoteState> {
            self.app.handle().state::<Arc<RemoteState>>().inner().clone()
        }
        fn local(&self) -> sqlx::SqlitePool {
            self.app.handle().state::<crate::intake::IntakeDb>().pool.clone()
        }
        async fn map(&self) -> Result<(), String> {
            let reg = self.app.handle().state::<ProjectRegistry>();
            save_map(&self.local(), &reg, &self.pid, &BoardMap { board_project_id: self.board.id.clone(), work_dir: self.work.path().to_string_lossy().into() }).await
        }
        /// 另一位成員先把一張卡片排進待派送。
        async fn queued_card(&self, title: &str) -> BoardCard {
            let (api, _) = pair(&self.srv, "小明").await;
            let c: BoardCard = api.typed(Method::POST, &format!("/v1/board/projects/{}/cards", self.pid), Some(json!({"title": title, "body": "請處理", "parallel_ok": false}))).await.unwrap();
            api.typed(Method::POST, &format!("/v1/board/cards/{}/move", c.id), Some(json!({"version": c.version, "to_status": "queued", "sort_order": 1.0}))).await.unwrap()
        }
        async fn server_card(&self, id: &str) -> BoardCard {
            bs::get_card(&self.srv.state().pools.intake, id).await.unwrap().unwrap()
        }
        async fn shadows(&self) -> Vec<crate::tasks::store::TaskRow> {
            store::list_tasks(&self.board.pool).await.unwrap()
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_local_mapping_is_validated_saved_per_server_project_and_required_before_claiming() {
        let r = rig(false).await;
        let c = r.queued_card("x").await;
        let e = claim(r.app.handle(), &c.id).await.unwrap_err();
        assert!(e.contains("請先設定"), "{e}");
        assert_eq!(r.server_card(&c.id).await.status, "queued", "沒設定對應就不會去領");

        let reg = r.app.handle().state::<ProjectRegistry>();
        let bad_project = save_map(&r.local(), &reg, &r.pid, &BoardMap { board_project_id: "不存在".into(), work_dir: r.work.path().to_string_lossy().into() }).await;
        assert!(bad_project.unwrap_err().contains("看板專案"));
        let bad_dir = save_map(&r.local(), &reg, &r.pid, &BoardMap { board_project_id: r.board.id.clone(), work_dir: "/不存在/的資料夾".into() }).await;
        assert!(bad_dir.unwrap_err().contains("找不到"));
        r.map().await.unwrap();
        let m = load_map(&r.local(), &r.pid).await.unwrap();
        assert_eq!((m.board_project_id.as_str(), m.work_dir.as_str()), (r.board.id.as_str(), r.work.path().to_str().unwrap()));
        assert!(load_map(&r.local(), "別的專案").await.is_none(), "每個 server 專案各自一份");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn claiming_creates_a_queued_shadow_card_in_the_mapped_folder_and_marks_the_server_card_mine() {
        let r = rig(true).await;
        let c = r.queued_card("印表機卡紙").await;
        claim(r.app.handle(), &c.id).await.unwrap();
        let s = r.server_card(&c.id).await;
        assert_eq!((s.status.as_str(), s.claimed_by.as_deref()), ("running", Some("小華")));
        let shadows = r.shadows().await;
        assert_eq!(shadows.len(), 1);
        let t = &shadows[0];
        assert_eq!((t.title.as_str(), t.body.as_str(), t.status.as_str(), t.label.as_deref(), t.parallel_ok), ("印表機卡紙", "請處理", "queued", Some(SHADOW_LABEL), false));
        assert_eq!(t.project_dir, r.work.path().to_string_lossy(), "在我設定的資料夾執行");
        assert_eq!(r.rs().board_claims().len(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn only_one_member_wins_a_card_and_the_loser_gets_no_shadow_card() {
        let r = rig(true).await;
        let c = r.queued_card("搶").await;
        let (other, _) = pair(&r.srv, "小美").await;
        other.request(Method::POST, &format!("/v1/board/cards/{}/claim", c.id), None).await.unwrap();
        let e = claim(r.app.handle(), &c.id).await.unwrap_err();
        assert!(e.contains("已被領取"), "{e}");
        assert!(r.shadows().await.is_empty() && r.rs().board_claims().is_empty(), "沒領到就不留影子卡片");
        assert_eq!(r.server_card(&c.id).await.claimed_by.as_deref(), Some("小美"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_finished_shadow_card_reports_success_or_failure_back_to_the_server() {
        let r = rig(true).await;
        let (ok, bad) = (r.queued_card("會成功").await, r.queued_card("會失敗").await);
        claim(r.app.handle(), &ok.id).await.unwrap();
        claim(r.app.handle(), &bad.id).await.unwrap();
        for (title, outcome, error) in [("會成功", "success", None), ("會失敗", "failed", Some("Claude Code 結束了"))] {
            let id = r.shadows().await.into_iter().find(|t| t.title == title).unwrap().id;
            store::finish_task(&r.board.pool, &id, outcome, error, None).await.unwrap();
        }
        tick(r.app.handle(), HEARTBEAT_EVERY).await;
        let (a, b) = (r.server_card(&ok.id).await, r.server_card(&bad.id).await);
        assert_eq!((a.status.as_str(), a.outcome.as_deref(), a.error_message), ("done", Some("success"), None));
        assert_eq!((b.status.as_str(), b.outcome.as_deref(), b.error_message.as_deref()), ("done", Some("failed"), Some("Claude Code 結束了")));
        assert!(r.rs().board_claims().is_empty(), "交回後不再追蹤");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_shadow_card_that_is_deleted_or_dragged_back_to_planning_puts_the_server_card_back() {
        let r = rig(true).await;
        let (gone, back) = (r.queued_card("被刪").await, r.queued_card("拖回").await);
        claim(r.app.handle(), &gone.id).await.unwrap();
        claim(r.app.handle(), &back.id).await.unwrap();
        let rows = r.shadows().await;
        let id_of = |t: &str| rows.iter().find(|x| x.title == t).unwrap().id.clone();
        sqlx::query("DELETE FROM tasks WHERE id = ?").bind(id_of("被刪")).execute(&r.board.pool).await.unwrap();
        sqlx::query("UPDATE tasks SET status = 'planning' WHERE id = ?").bind(id_of("拖回")).execute(&r.board.pool).await.unwrap();
        tick(r.app.handle(), HEARTBEAT_EVERY).await;
        for id in [&gone.id, &back.id] {
            let s = r.server_card(id).await;
            assert_eq!((s.status.as_str(), s.claimed_by), ("queued", None), "放回待派送，別人可以領");
        }
        assert!(r.rs().board_claims().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn heartbeats_renew_the_lease_and_a_lease_the_server_took_back_stops_the_reporting() {
        let r = rig(true).await;
        let c = r.queued_card("長時間").await;
        claim(r.app.handle(), &c.id).await.unwrap();
        sqlx::query("UPDATE board_cards SET claim_expires_at = 1 WHERE id = ?").bind(&c.id).execute(&r.srv.state().pools.intake).await.unwrap();
        tick(r.app.handle(), Duration::ZERO).await;
        assert!(r.server_card(&c.id).await.claim_expires_at.unwrap() > 1000, "續約把到期時間往後推");

        // 管理員把它放回去了：之後本機做完也不會再回寫（結果不會被採用）。
        let admin = r.srv.paired("管理員", Role::Admin).await;
        admin.request(Method::POST, &format!("/v1/board/cards/{}/release", c.id), None).await.unwrap();
        tick(r.app.handle(), Duration::ZERO).await;
        assert!(r.rs().board_claims().is_empty(), "續約被拒絕就不再追蹤");
        let t = r.shadows().await.remove(0);
        store::finish_task(&r.board.pool, &t.id, "success", None, None).await.unwrap();
        tick(r.app.handle(), Duration::ZERO).await;
        assert_eq!(r.server_card(&c.id).await.status, "queued", "沒有被一個過期的結果蓋掉");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn releasing_hands_the_card_back_and_stops_tracking() {
        let r = rig(true).await;
        let c = r.queued_card("不做了").await;
        claim(r.app.handle(), &c.id).await.unwrap();
        release(r.app.handle(), &c.id).await.unwrap();
        assert_eq!(r.server_card(&c.id).await.status, "queued");
        assert!(r.rs().board_claims().is_empty());
    }
}
