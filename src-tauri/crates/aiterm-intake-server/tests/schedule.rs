//! 共用看板的定時器：只由 server 觸發一次、到點建一輪、上一輪沒結案就跳過、結算後歸檔、連續失敗自動停用、
//! 管理員才能改。時間由測試給（假時鐘），不用真的等。
use aiterm_intake_client::api::Api;
use aiterm_intake_core::api::Role;
use aiterm_intake_core::board_store::{self as bs, BoardCard, NewAgent};
use aiterm_intake_server::scheduler;
use aiterm_intake_server::testkit::TestServer;
use chrono::{Duration, Local, NaiveDateTime};
use reqwest::Method;
use serde_json::{json, Value};

struct Rig {
    srv: TestServer,
    admin: Api,
    member: Api,
    pid: String,
    coder: bs::BoardAgent,
    base: NaiveDateTime,
}

async fn agent(srv: &TestServer, pid: &str, kind: &str, name: &str) -> bs::BoardAgent {
    bs::create_agent(&srv.state().pools.intake, NewAgent { project_id: pid, kind, name, role: "", prompt_prefix: "", canvas_x: 0.0, canvas_y: 0.0, produces_spec: false, model: None, cli: "claude", created_by: "t" }).await.unwrap().unwrap()
}

async fn rig() -> Rig {
    let srv = TestServer::start().await;
    let (admin, member) = (srv.paired("小華", Role::Admin).await, srv.paired("小明", Role::Member).await);
    let pid = srv.project("範例科技").await;
    let coder = agent(&srv, &pid, "coder", "工程師1").await;
    Rig { srv, admin, member, pid, coder, base: Local::now().naive_local() }
}

impl Rig {
    fn pool(&self) -> &sqlx::SqlitePool {
        &self.srv.state().pools.intake
    }
    fn body(&self, name: &str, kind: &str, spec: Value) -> Value {
        json!({"name": name, "agent_id": self.coder.id, "instruction": "跑完整測試並回報", "schedule_kind": kind, "spec": spec})
    }
    /// 建一個每 5 分鐘一次的定時器；基準時間是建立當下。
    async fn interval(&self, name: &str) -> String {
        let v = self.admin.request(Method::POST, &format!("/v1/board/projects/{}/schedules", self.pid), Some(self.body(name, "interval", json!({"every_minutes": 5})))).await.unwrap();
        v["id"].as_str().unwrap().to_string()
    }
    fn at(&self, minutes: i64) -> NaiveDateTime {
        self.base + Duration::minutes(minutes)
    }
    async fn tick(&self, minutes: i64) -> usize {
        scheduler::tick_at(self.srv.state(), self.at(minutes)).await
    }
    async fn cards(&self) -> Vec<BoardCard> {
        bs::project_cards(self.pool(), &self.pid).await.unwrap()
    }
    async fn schedule(&self, id: &str) -> bs::BoardSchedule {
        bs::get_schedule(self.pool(), id).await.unwrap().unwrap()
    }
    async fn runs(&self, id: &str) -> Vec<String> {
        bs::list_schedule_runs(self.pool(), id).await.unwrap().into_iter().map(|r| r.result).collect()
    }
    /// 成員領取並完成一張卡。
    async fn member_finishes(&self, card: &BoardCard, outcome: &str) {
        assert!(bs::claim_card(self.pool(), &card.id, "小明", 1).await.unwrap());
        assert!(bs::finish_claim(self.pool(), &card.id, "小明", outcome, Some("做完了"), if outcome == "failed" { Some("出錯") } else { None }, 2).await.unwrap());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn it_fires_when_due_exactly_once_no_matter_how_many_people_are_connected_or_how_often_it_scans() {
    let r = rig().await;
    let id = r.interval("每日健檢").await;
    assert_eq!(r.tick(4).await, 0, "還沒到 5 分鐘");
    assert!(r.cards().await.is_empty());
    assert_eq!(r.tick(6).await, 1);
    // 同一時間點再掃很多次、兩位成員都連著：還是只有一張。
    for _ in 0..5 {
        assert_eq!(r.tick(6).await, 0);
    }
    let _ = (&r.admin, &r.member);
    let cards = r.cards().await;
    assert_eq!(cards.len(), 1);
    let c = &cards[0];
    assert_eq!((c.card_kind.as_str(), c.status.as_str(), c.agent_id.as_deref(), c.title.as_str()), ("work", "queued", Some(r.coder.id.as_str()), "定時：每日健檢"));
    assert!(c.body.contains("跑完整測試並回報") && c.body.contains("定時觸發"));
    let s = r.schedule(&id).await;
    assert_eq!((s.last_card_id.as_deref(), s.skip_streak), (Some(c.id.as_str()), 0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unfinished_round_makes_the_next_one_skip_and_a_finished_one_is_archived_before_the_next_fires() {
    let r = rig().await;
    let id = r.interval("每日健檢").await;
    r.tick(6).await;
    // 上一輪沒人做：到點了也不再建，記一筆 skipped。
    assert_eq!(r.tick(12).await, 0);
    assert_eq!((r.cards().await.len(), r.schedule(&id).await.skip_streak, r.runs(&id).await), (1, 1, vec!["skipped".to_string()]));
    // 成員做完（成功）：下一個 tick 結算成功、卡片歸檔、不再佔著「進行中」。
    let card = r.cards().await.remove(0);
    r.member_finishes(&card, "success").await;
    assert_eq!(r.tick(13).await, 0, "結算那個 tick 不到點，不會立刻再建");
    let s = r.schedule(&id).await;
    assert_eq!((s.last_card_id, s.fail_streak), (None, 0));
    assert_eq!(r.runs(&id).await, vec!["success".to_string(), "skipped".to_string()]);
    assert!(r.cards().await.is_empty(), "完成的一輪卡片已歸檔，不堆在看板上");
    assert_eq!(bs::get_card(r.pool(), &card.id).await.unwrap().unwrap().archived_at.is_some(), true);
    // 下一個預定時間點到了：新的一輪。
    assert_eq!(r.tick(19).await, 1);
    assert_eq!(r.cards().await.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn three_failed_rounds_in_a_row_switch_it_off_with_a_reason_and_turning_it_on_again_starts_clean() {
    let r = rig().await;
    let id = r.interval("每日健檢").await;
    for (n, minute) in [(1, 6), (2, 12), (3, 18)] {
        assert_eq!(r.tick(minute).await, 1, "第 {n} 輪");
        let card = r.cards().await.into_iter().find(|c| c.status == "queued").unwrap();
        r.member_finishes(&card, "failed").await;
        r.tick(minute + 1).await; // 結算
        assert_eq!(r.schedule(&id).await.fail_streak, n);
    }
    let s = r.schedule(&id).await;
    assert!(!s.enabled);
    assert_eq!(s.disabled_reason.as_deref(), Some("連續失敗"));
    assert_eq!(r.tick(40).await, 0, "停用後不再觸發");
    // 重新啟用：清掉連續失敗與原因，基準時間設成現在，不補停用期間錯過的。
    r.admin.request(Method::POST, &format!("/v1/board/schedules/{id}/enabled"), Some(json!({"enabled": true}))).await.unwrap();
    let s = r.schedule(&id).await;
    assert_eq!((s.enabled, s.fail_streak, s.disabled_reason), (true, 0, None));
    // 啟用時的基準是「現在」（假時鐘的 0 分），所以停用期間錯過的 40 幾分鐘不補：1 分鐘後還沒到點，6 分鐘後才觸發。
    assert_eq!(r.tick(1).await, 0, "剛啟用不會立刻觸發");
    assert_eq!(r.tick(6).await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_deleted_agent_switches_the_schedule_off_instead_of_firing_into_nothing() {
    let r = rig().await;
    let id = r.interval("每日健檢").await;
    bs::delete_agent(r.pool(), &r.coder.id).await.unwrap().unwrap();
    assert_eq!(r.tick(6).await, 0);
    let s = r.schedule(&id).await;
    assert_eq!((s.enabled, s.disabled_reason.as_deref()), (false, Some("角色已不存在")));
    assert!(r.cards().await.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daily_and_weekly_fire_at_their_clock_times_in_the_servers_local_time() {
    let r = rig().await;
    let at = |h: u32, m: u32, plus_days: i64| (r.base.date() + Duration::days(plus_days)).and_hms_opt(h, m, 0).unwrap();
    let daily = r.admin.request(Method::POST, &format!("/v1/board/projects/{}/schedules", r.pid), Some(r.body("早班", "daily", json!({"times": ["08:00"]})))).await.unwrap();
    let id = daily["id"].as_str().unwrap().to_string();
    // 把建立時間倒回前一天晚上，這樣「明天 08:00」是下一個時間點。
    sqlx::query("UPDATE board_schedules SET last_fired_at = ? WHERE id = ?").bind(aiterm_intake_core::schedule::local_ts(at(23, 0, -1))).bind(&id).execute(r.pool()).await.unwrap();
    assert_eq!(scheduler::tick_at(r.srv.state(), at(7, 59, 0)).await, 0);
    assert_eq!(scheduler::tick_at(r.srv.state(), at(8, 0, 0)).await, 1);
    assert_eq!(scheduler::tick_at(r.srv.state(), at(9, 0, 0)).await, 0, "同一個時間點不會再觸發");
    // 每週：只在勾選的星期。
    let weekday = at(9, 0, 0).format("%u").to_string().parse::<u32>().unwrap();
    let other = if weekday == 1 { 2 } else { 1 };
    let w = r.admin.request(Method::POST, &format!("/v1/board/projects/{}/schedules", r.pid), Some(r.body("週報", "weekly", json!({"times": ["10:00"], "days": [other]})))).await.unwrap();
    sqlx::query("UPDATE board_schedules SET last_fired_at = ? WHERE id = ?").bind(aiterm_intake_core::schedule::local_ts(at(9, 0, -1))).bind(w["id"].as_str().unwrap()).execute(r.pool()).await.unwrap();
    assert_eq!(scheduler::tick_at(r.srv.state(), at(10, 30, 0)).await, 0, "今天不是勾選的星期");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_admins_manage_schedules_and_bad_input_is_refused_with_the_reason() {
    let r = rig().await;
    let url = format!("/v1/board/projects/{}/schedules", r.pid);
    let ok = r.body("甲", "interval", json!({"every_minutes": 5}));
    assert!(r.member.request(Method::POST, &url, Some(ok.clone())).await.is_err(), "成員不能建");
    let id = r.interval("乙").await;
    assert!(r.member.request(Method::PUT, &format!("/v1/board/schedules/{id}"), Some(ok.clone())).await.is_err());
    assert!(r.member.request(Method::DELETE, &format!("/v1/board/schedules/{id}"), None).await.is_err());
    assert!(r.member.request(Method::POST, &format!("/v1/board/schedules/{id}/enabled"), Some(json!({"enabled": false}))).await.is_err());
    assert!(r.member.request(Method::POST, &format!("/v1/board/schedules/{id}/run"), None).await.is_err());
    // 成員看得到（唯讀）。
    let listed = r.member.request(Method::GET, &url, None).await.unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);

    let err = |v: Value| async { r.admin.request(Method::POST, &url, Some(v)).await.unwrap_err().to_string() };
    assert!(err(r.body("太密", "interval", json!({"every_minutes": 4}))).await.contains("間隔不能小於"));
    assert!(err(r.body("壞時間", "daily", json!({"times": ["25:00"]}))).await.contains("時間格式"));
    assert!(err(json!({ "name": " ", "agent_id": r.coder.id, "instruction": "x", "schedule_kind": "interval", "spec": {"every_minutes": 5} })).await.contains("名稱"));
    assert!(err(json!({ "name": "a", "agent_id": r.coder.id, "instruction": " ", "schedule_kind": "interval", "spec": {"every_minutes": 5} })).await.contains("指令"));
    assert!(err(json!({ "name": "a", "agent_id": "沒有", "instruction": "x", "schedule_kind": "interval", "spec": {"every_minutes": 5} })).await.contains("找不到這個角色"));
    let reviewer = agent(&r.srv, &r.pid, "reviewer", "審核者").await;
    assert!(err(json!({ "name": "a", "agent_id": reviewer.id, "instruction": "x", "schedule_kind": "interval", "spec": {"every_minutes": 5} })).await.contains("只能叫起工程師"));
    let other_pid = r.srv.project("別的專案").await;
    let foreign = agent(&r.srv, &other_pid, "coder", "別人的").await;
    assert!(err(json!({ "name": "a", "agent_id": foreign.id, "instruction": "x", "schedule_kind": "interval", "spec": {"every_minutes": 5} })).await.contains("不在同一個專案"));
    // 管理員可以改、刪。
    r.admin.request(Method::PUT, &format!("/v1/board/schedules/{id}"), Some(r.body("乙改名", "interval", json!({"every_minutes": 30})))).await.unwrap();
    assert_eq!(r.schedule(&id).await.name, "乙改名");
    r.admin.request(Method::DELETE, &format!("/v1/board/schedules/{id}"), None).await.unwrap();
    assert!(r.admin.request(Method::DELETE, &format!("/v1/board/schedules/{id}"), None).await.is_err(), "已經刪掉了");
    assert!(bs::list_schedule_runs(r.pool(), &id).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_now_starts_a_round_without_using_up_the_next_slot_and_is_blocked_while_one_is_open() {
    let r = rig().await;
    let id = r.interval("每日健檢").await;
    let before = r.schedule(&id).await.last_fired_at;
    let v = r.admin.request(Method::POST, &format!("/v1/board/schedules/{id}/run"), None).await.unwrap();
    assert!(v.is_string(), "回傳新卡 id：{v}");
    assert_eq!(r.cards().await.len(), 1);
    assert_eq!(r.schedule(&id).await.last_fired_at, before, "不吃掉下一個預定時間點");
    // 這一輪還沒結案：再按一次被擋下（回 null），記一筆 skipped。
    let v = r.admin.request(Method::POST, &format!("/v1/board/schedules/{id}/run"), None).await.unwrap();
    assert!(v.is_null(), "{v}");
    assert_eq!((r.cards().await.len(), r.runs(&id).await), (1, vec!["skipped".to_string()]));
}
