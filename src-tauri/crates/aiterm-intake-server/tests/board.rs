//! 共用看板的端到端測試：真的 TLS、真的 Client、多位成員同時操作卡片。
use aiterm_intake_client::api::Api;
use aiterm_intake_client::ClientError;
use aiterm_intake_core::api::Role;
use aiterm_intake_core::board_store::{self as bs, BoardCard, BoardCardView};
use aiterm_intake_server::testkit::TestServer;
use reqwest::Method;
use serde_json::{json, Value};

struct Rig {
    srv: TestServer,
    pid: String,
}

async fn rig() -> Rig {
    let srv = TestServer::start().await;
    let pid = srv.project("範例科技").await;
    Rig { srv, pid }
}

async fn create(api: &Api, pid: &str, title: &str) -> BoardCard {
    api.typed(Method::POST, &format!("/v1/board/projects/{pid}/cards"), Some(json!({"title": title, "body": "內容"}))).await.unwrap()
}

async fn list(api: &Api, pid: &str) -> Vec<BoardCardView> {
    api.get(&format!("/v1/board/projects/{pid}/cards")).await.unwrap()
}

async fn mv(api: &Api, c: &BoardCard, to: &str) -> Result<BoardCard, ClientError> {
    api.typed(Method::POST, &format!("/v1/board/cards/{}/move", c.id), Some(json!({"version": c.version, "to_status": to}))).await
}

fn is_server_err(r: &Result<impl std::fmt::Debug, ClientError>, needle: &str) -> bool {
    matches!(r, Err(ClientError::Server(m)) if m.contains(needle))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn everyone_sees_the_same_cards_and_an_edit_from_a_stale_view_is_refused() {
    let r = rig().await;
    let (a, b) = (r.srv.paired("小明", Role::Member).await, r.srv.paired("小華", Role::Member).await);
    let c = create(&a, &r.pid, "印表機卡紙").await;
    assert_eq!((c.status.as_str(), c.created_by.as_str(), c.version), ("planning", "小明", 1));
    assert_eq!(list(&b, &r.pid).await[0].card.title, "印表機卡紙", "另一位成員看到同一張");

    // 兩人都從版本 1 開始編輯：先存的成功，後存的被擋，內容不會被靜默覆蓋。
    let edit = |t: &str| json!({"version": 1, "title": t, "body": "新內容", "parallel_ok": false, "interactive": false, "label": " 急 "});
    let first: BoardCard = a.typed(Method::PUT, &format!("/v1/board/cards/{}", c.id), Some(edit("甲改的"))).await.unwrap();
    assert_eq!((first.version, first.label.as_deref()), (2, Some("急")));
    let second = b.typed::<BoardCard>(Method::PUT, &format!("/v1/board/cards/{}", c.id), Some(edit("乙改的"))).await;
    assert!(is_server_err(&second, "已被別人修改"), "{second:?}");
    assert_eq!(list(&b, &r.pid).await[0].card.title, "甲改的");
    assert!(a.request(Method::PUT, "/v1/board/cards/不存在", Some(edit("x"))).await.is_err());
    assert!(a.request(Method::PUT, &format!("/v1/board/cards/{}", c.id), Some(json!({"version": 2, "title": "  ", "body": ""}))).await.is_err(), "標題不能空");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cards_move_between_planning_and_queued_in_the_order_the_neighbours_say() {
    let r = rig().await;
    let a = r.srv.paired("小明", Role::Member).await;
    let (x, y, z) = (create(&a, &r.pid, "x").await, create(&a, &r.pid, "y").await, create(&a, &r.pid, "z").await);
    let x: BoardCard = mv(&a, &x, "queued").await.unwrap();
    let y: BoardCard = a.typed(Method::POST, &format!("/v1/board/cards/{}/move", y.id), Some(json!({"version": y.version, "to_status": "queued", "after_id": x.id}))).await.unwrap();
    let _z: BoardCard = a.typed(Method::POST, &format!("/v1/board/cards/{}/move", z.id), Some(json!({"version": z.version, "to_status": "queued", "after_id": x.id, "before_id": y.id}))).await.unwrap();
    let titles: Vec<String> = list(&a, &r.pid).await.into_iter().map(|v| v.card.title).collect();
    assert_eq!(titles, vec!["x", "z", "y"]);
    assert!(is_server_err(&mv(&a, &x, "done").await, "不能從"), "不能手動移到完成");
    let stale = y.clone();
    mv(&a, &y, "planning").await.unwrap(); // 別人先把 y 移走了
    assert!(is_server_err(&mv(&a, &stale, "planning").await, "已被別人修改"), "拿舊版本移動會被擋");
    assert!(a.request(Method::GET, "/v1/board/projects/不存在/cards", None).await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_members_claiming_the_same_card_at_once_only_one_wins_and_only_the_claimer_finishes() {
    let r = rig().await;
    let (a, b) = (r.srv.paired("小明", Role::Member).await, r.srv.paired("小華", Role::Member).await);
    let c = create(&a, &r.pid, "要處理").await;
    assert!(a.request(Method::POST, &format!("/v1/board/cards/{}/claim", c.id), None).await.is_err(), "計畫中的不能領");
    let q = mv(&a, &c, "queued").await.unwrap();
    let path = format!("/v1/board/cards/{}/claim", q.id);
    let (ra, rb) = tokio::join!(a.typed::<BoardCard>(Method::POST, &path, None), b.typed::<BoardCard>(Method::POST, &path, None));
    assert_eq!([ra.is_ok(), rb.is_ok()].iter().filter(|x| **x).count(), 1, "{ra:?} / {rb:?}");
    let (winner, loser) = if ra.is_ok() { (&a, &b) } else { (&b, &a) };
    let name = if ra.is_ok() { "小明" } else { "小華" };
    let running = &list(&a, &r.pid).await[0].card;
    assert_eq!((running.status.as_str(), running.claimed_by.as_deref()), ("running", Some(name)));

    let res = format!("/v1/board/cards/{}/result", q.id);
    assert!(is_server_err(&loser.request(Method::POST, &res, Some(json!({"outcome": "success"}))).await, "不是你領取"));
    assert!(a.request(Method::POST, &res, Some(json!({"outcome": "亂寫"}))).await.is_err());
    winner.request(Method::POST, &format!("/v1/board/cards/{}/summary", q.id), Some(json!({"summary": "進度"}))).await.unwrap();
    assert_eq!(winner.typed::<bool>(Method::POST, &format!("/v1/board/cards/{}/heartbeat", q.id), None).await.unwrap(), true);
    assert_eq!(loser.typed::<bool>(Method::POST, &format!("/v1/board/cards/{}/heartbeat", q.id), None).await.unwrap(), false, "別人續不了約");
    winner.request(Method::POST, &res, Some(json!({"outcome": "success", "summary": "改好了，見 /Users/someone/proj sk-abcdefghijklmnop1234"}))).await.unwrap();
    let done = &list(&loser, &r.pid).await[0].card;
    assert_eq!((done.status.as_str(), done.outcome.as_deref()), ("done", Some("success")));
    let summary = done.ai_summary.clone().unwrap();
    assert!(summary.contains("改好了") && !summary.contains("someone") && !summary.contains("sk-abcdef"), "摘要要去敏：{summary}");
    assert!(winner.request(Method::POST, &res, Some(json!({"outcome": "success"}))).await.is_err(), "交過之後不能再交");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lapsed_lease_puts_the_card_back_and_release_and_failure_follow_the_rules() {
    let r = rig().await;
    let (a, b) = (r.srv.paired("小明", Role::Member).await, r.srv.paired("小華", Role::Member).await);
    let admin = r.srv.paired("管理員", Role::Admin).await;
    let c = mv(&a, &create(&a, &r.pid, "x").await, "queued").await.unwrap();
    let claim = format!("/v1/board/cards/{}/claim", c.id);
    a.request(Method::POST, &claim, None).await.unwrap();
    // 領取者失聯：租約逾期後放回待派送，別人可以領。
    assert_eq!(bs::expire_claims(&r.srv.state().pools.intake, aiterm_intake_server::db::now() + bs::CLAIM_TTL_SECS + 5).await.unwrap(), 1);
    assert_eq!(list(&b, &r.pid).await[0].card.status, "queued");
    b.request(Method::POST, &claim, None).await.unwrap();
    assert!(is_server_err(&a.request(Method::POST, &format!("/v1/board/cards/{}/release", c.id), None).await, "不是你領取"), "別人不能放回");
    admin.request(Method::POST, &format!("/v1/board/cards/{}/release", c.id), None).await.unwrap();
    a.request(Method::POST, &claim, None).await.unwrap();
    a.request(Method::POST, &format!("/v1/board/cards/{}/result", c.id), Some(json!({"outcome": "failed", "error": "Claude Code 結束了"}))).await.unwrap();
    let f = &list(&a, &r.pid).await[0].card;
    assert_eq!((f.status.as_str(), f.outcome.as_deref(), f.error_message.as_deref()), ("done", Some("failed"), Some("Claude Code 結束了")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_the_creator_or_an_admin_can_delete_and_a_running_card_cannot_be_deleted_or_archived() {
    let r = rig().await;
    let (a, b) = (r.srv.paired("小明", Role::Member).await, r.srv.paired("小華", Role::Member).await);
    let admin = r.srv.paired("管理員", Role::Admin).await;
    let c = create(&a, &r.pid, "我的").await;
    let del = format!("/v1/board/cards/{}", c.id);
    assert!(b.request(Method::DELETE, &del, None).await.is_err(), "別人不能刪");
    let q = mv(&a, &c, "queued").await.unwrap();
    a.request(Method::POST, &format!("/v1/board/cards/{}/claim", q.id), None).await.unwrap();
    assert!(is_server_err(&a.request(Method::DELETE, &del, None).await, "處理中"), "處理中不能刪");
    assert!(a.request(Method::POST, &format!("/v1/board/cards/{}/archive", c.id), Some(json!({"archived": true}))).await.is_err(), "沒做完不能封存");
    a.request(Method::POST, &format!("/v1/board/cards/{}/result", c.id), Some(json!({"outcome": "success"}))).await.unwrap();
    a.request(Method::POST, &format!("/v1/board/cards/{}/archive", c.id), Some(json!({"archived": true}))).await.unwrap();
    assert!(list(&a, &r.pid).await.is_empty());
    let archived: Vec<BoardCardView> = a.get(&format!("/v1/board/projects/{}/cards?archived=true", r.pid)).await.unwrap();
    assert_eq!(archived.len(), 1);
    a.request(Method::POST, &format!("/v1/board/cards/{}/archive", c.id), Some(json!({"archived": false}))).await.unwrap();
    assert!(b.request(Method::POST, &format!("/v1/board/projects/{}/archive-done", r.pid), None).await.is_err(), "全部封存限管理員");
    assert_eq!(admin.typed::<u64>(Method::POST, &format!("/v1/board/projects/{}/archive-done", r.pid), None).await.unwrap(), 1);
    admin.request(Method::DELETE, &del, None).await.unwrap();
    assert!(list(&a, &r.pid).await.is_empty() && bs::get_card(&r.srv.state().pools.intake, &c.id).await.unwrap().is_none(), "管理員可以刪別人的");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attachments_upload_download_and_go_with_the_card_and_comments_are_listed_in_order() {
    use base64::Engine;
    let r = rig().await;
    let (a, b) = (r.srv.paired("小明", Role::Member).await, r.srv.paired("小華", Role::Member).await);
    let c = create(&a, &r.pid, "有附件").await;
    let data = base64::engine::general_purpose::STANDARD.encode(b"ERROR boom\n");
    let att: Value = a.typed(Method::POST, &format!("/v1/board/cards/{}/attachments", c.id), Some(json!({"name": "err.log", "data_b64": data}))).await.unwrap();
    let aid = att["id"].as_str().unwrap().to_string();
    assert!(att["filename"].as_str().unwrap().ends_with("err.log"), "{att}");
    assert!(a.request(Method::POST, &format!("/v1/board/cards/{}/attachments", c.id), Some(json!({"name": "x", "data_b64": "!!!不是base64"}))).await.is_err());
    assert!(a.request(Method::POST, &format!("/v1/board/cards/{}/attachments", c.id), Some(json!({"name": "empty", "data_b64": ""}))).await.is_err(), "空檔案不收");
    assert!(a.request(Method::POST, "/v1/board/cards/不存在/attachments", Some(json!({"name": "x", "data_b64": data}))).await.is_err());
    let v = &list(&b, &r.pid).await[0];
    assert_eq!(v.attachments.len(), 1, "別人也看得到附件");
    assert!(v.card.version > 1, "新增附件也算一次變動，編輯舊版本會被擋");
    let bytes = b.download(&format!("/v1/board/attachments/{aid}/file")).await.unwrap();
    assert_eq!(String::from_utf8_lossy(&bytes), "ERROR boom\n");

    a.request(Method::POST, &format!("/v1/board/cards/{}/comments", c.id), Some(json!({"text": "我先看"}))).await.unwrap();
    b.request(Method::POST, &format!("/v1/board/cards/{}/comments", c.id), Some(json!({"text": "  收到  "}))).await.unwrap();
    assert!(b.request(Method::POST, &format!("/v1/board/cards/{}/comments", c.id), Some(json!({"text": "  "}))).await.is_err());
    let cs: Vec<Value> = a.get(&format!("/v1/board/cards/{}/comments", c.id)).await.unwrap();
    assert_eq!(cs.iter().map(|x| (x["author"].as_str().unwrap(), x["text"].as_str().unwrap())).collect::<Vec<_>>(), vec![("小明", "我先看"), ("小華", "收到")]);
    assert_eq!(list(&a, &r.pid).await[0].comment_count, 2);

    b.request(Method::DELETE, &format!("/v1/board/attachments/{aid}"), None).await.unwrap();
    assert!(list(&a, &r.pid).await[0].attachments.is_empty());
    let again: Value = a.typed(Method::POST, &format!("/v1/board/cards/{}/attachments", c.id), Some(json!({"name": "again.log", "data_b64": data}))).await.unwrap();
    let path = again["path"].as_str().unwrap().to_string();
    assert!(std::path::Path::new(&path).exists());
    a.request(Method::DELETE, &format!("/v1/board/cards/{}", c.id), None).await.unwrap();
    assert!(!std::path::Path::new(&path).exists(), "刪卡片時附件檔案一併清掉");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn board_changes_are_announced_and_the_expired_lease_sweep_runs_with_the_servers_tick() {
    let r = rig().await;
    let a = r.srv.paired("小明", Role::Member).await;
    let before = r.srv.state().events.seq();
    let c = create(&a, &r.pid, "x").await;
    assert!(r.srv.state().events.seq() > before, "建立卡片要通知所有連線的 AITerm");
    let q = mv(&a, &c, "queued").await.unwrap();
    a.request(Method::POST, &format!("/v1/board/cards/{}/claim", q.id), None).await.unwrap();
    sqlx::query("UPDATE board_cards SET claim_expires_at = 1 WHERE id = ?").bind(&q.id).execute(&r.srv.state().pools.intake).await.unwrap();
    aiterm_intake_server::channels::tick_once(r.srv.state()).await;
    assert_eq!(list(&a, &r.pid).await[0].card.status, "queued", "伺服器定時掃描把逾期的卡片放回待派送");
}
