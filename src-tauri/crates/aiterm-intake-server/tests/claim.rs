//! 領取處理的端到端測試：真的 TLS、真的 Client、多位成員同時操作。
use aiterm_intake_client::api::Api;
use aiterm_intake_client::ClientError;
use aiterm_intake_core::api::Role;
use aiterm_intake_core::rules::Level;
use aiterm_intake_core::store::{self, IntakeItem, NewItem};
use aiterm_intake_server::channels;
use aiterm_intake_server::testkit::TestServer;
use reqwest::Method;
use serde_json::json;

async fn add(srv: &TestServer, pid: &str, subject: &str, status: &str) -> IntakeItem {
    store::insert_item(
        &srv.state().pools.intake,
        NewItem { project_id: pid, id: None, source: "line", sender: "line:u0123456789abcdef0123456789abcdef", level: Level::Internal, subject, body: "內容", message_id: subject, status },
    )
    .await
    .unwrap()
    .unwrap()
}

async fn claim(api: &Api, id: &str) -> Result<IntakeItem, ClientError> {
    api.typed(Method::POST, &format!("/v1/items/{id}/claim"), None).await
}

async fn get(api: &Api, id: &str) -> IntakeItem {
    api.get(&format!("/v1/items/{id}")).await.unwrap()
}

#[tokio::test]
async fn two_members_claiming_the_same_item_at_once_only_one_wins() {
    let srv = TestServer::start().await;
    let pid = srv.project("甲").await;
    let (a, b) = (srv.paired("小明", Role::Member).await, srv.paired("小華", Role::Member).await);
    let item = add(&srv, &pid, "題目", store::QUEUED).await;
    let (ra, rb) = tokio::join!(claim(&a, &item.id), claim(&b, &item.id));
    let wins = [ra.is_ok(), rb.is_ok()].iter().filter(|x| **x).count();
    assert_eq!(wins, 1, "同一題只能一個人領到：{ra:?} / {rb:?}");
    let loser = if ra.is_err() { ra } else { rb };
    assert!(matches!(loser, Err(ClientError::Server(m)) if m.contains("已被領取")));
    let got = get(&a, &item.id).await;
    assert_eq!(got.status, store::RUNNING);
    assert!(got.claimed_by.as_deref() == Some("小明") || got.claimed_by.as_deref() == Some("小華"));
}

#[tokio::test]
async fn an_unapproved_item_cannot_be_claimed_and_an_unknown_one_is_404() {
    let srv = TestServer::start().await;
    let pid = srv.project("甲").await;
    let m = srv.paired("小明", Role::Member).await;
    let pending = add(&srv, &pid, "待核准", store::PENDING).await;
    assert!(matches!(claim(&m, &pending.id).await, Err(ClientError::Server(m)) if m.contains("核准")));
    assert_eq!(claim(&m, "nope").await.unwrap_err(), ClientError::Server("not found".into()));
    m.request(Method::POST, &format!("/v1/items/{}/approve", pending.id), None).await.unwrap();
    assert!(claim(&m, &pending.id).await.is_ok(), "核准之後就領得到");
}

#[tokio::test]
async fn claim_next_gives_the_oldest_then_nothing() {
    let srv = TestServer::start().await;
    let pid = srv.project("甲").await;
    let m = srv.paired("小明", Role::Member).await;
    let first = add(&srv, &pid, "一", store::QUEUED).await;
    add(&srv, &pid, "二", store::PENDING).await;
    let got: Option<IntakeItem> = m.typed(Method::POST, &format!("/v1/projects/{pid}/claim-next"), None).await.unwrap();
    assert_eq!(got.map(|i| i.id), Some(first.id));
    let none: Option<IntakeItem> = m.typed(Method::POST, &format!("/v1/projects/{pid}/claim-next"), None).await.unwrap();
    assert!(none.is_none(), "沒有待派送的就回 null，不會領到待核准的");
}

#[tokio::test]
async fn only_the_claimer_can_heartbeat_and_only_they_or_an_admin_can_release() {
    let srv = TestServer::start().await;
    let pid = srv.project("甲").await;
    let (a, b, admin) = (srv.paired("小明", Role::Member).await, srv.paired("小華", Role::Member).await, srv.paired("管理", Role::Admin).await);
    let item = add(&srv, &pid, "題目", store::QUEUED).await;
    claim(&a, &item.id).await.unwrap();
    let hb = |api: &Api| {
        let api = api.clone();
        let id = item.id.clone();
        async move { api.typed::<bool>(Method::POST, &format!("/v1/items/{id}/heartbeat"), None).await.unwrap() }
    };
    assert!(hb(&a).await);
    assert!(!hb(&b).await, "別人不能續約");
    let rel = |api: &Api| {
        let api = api.clone();
        let id = item.id.clone();
        async move { api.typed::<bool>(Method::POST, &format!("/v1/items/{id}/release"), None).await.unwrap() }
    };
    assert!(!rel(&b).await, "別的成員不能放回");
    assert!(rel(&admin).await, "管理員可以放回");
    assert_eq!(get(&a, &item.id).await.status, store::QUEUED);
    claim(&b, &item.id).await.unwrap();
    assert!(rel(&b).await, "領取者本人可以放回");
}

#[tokio::test]
async fn the_result_becomes_a_redacted_draft_and_only_the_claimer_may_submit_it() {
    let srv = TestServer::start().await;
    let pid = srv.project("甲").await;
    let (a, b) = (srv.paired("小明", Role::Member).await, srv.paired("小華", Role::Member).await);
    let item = add(&srv, &pid, "題目", store::QUEUED).await;
    claim(&a, &item.id).await.unwrap();
    let path = format!("/v1/items/{}/result", item.id);
    assert!(matches!(b.request(Method::POST, &path, Some(json!({"reply":"偷交"}))).await, Err(ClientError::Server(m)) if m.contains("不是你領取")));
    assert!(a.request(Method::POST, &path, Some(json!({}))).await.is_err(), "沒帶 reply 或 error 不行");
    a.request(Method::POST, &path, Some(json!({"reply":"請看 /Users/someone/proj 與 sk-abcdefghijklmnop1234"}))).await.unwrap();
    let done = get(&b, &item.id).await;
    assert_eq!(done.status, store::DRAFT);
    let draft = done.reply_draft.unwrap();
    assert!(!draft.contains("someone") && !draft.contains("sk-abcdef"), "回覆要去敏：{draft}");
    assert_eq!(done.claimed_by.as_deref(), Some("小明"), "草稿上留著是誰處理的");
}

#[tokio::test]
async fn a_failure_report_sends_the_item_back_to_pending_with_the_reason() {
    let srv = TestServer::start().await;
    let pid = srv.project("甲").await;
    let a = srv.paired("小明", Role::Member).await;
    let item = add(&srv, &pid, "題目", store::QUEUED).await;
    claim(&a, &item.id).await.unwrap();
    a.request(Method::POST, &format!("/v1/items/{}/result", item.id), Some(json!({"error":"Claude Code 已結束，這題沒有完成"}))).await.unwrap();
    let g = get(&a, &item.id).await;
    assert_eq!((g.status.as_str(), g.error.as_deref(), g.claimed_by), (store::PENDING, Some("Claude Code 已結束，這題沒有完成"), None));
}

#[tokio::test]
async fn an_expired_lease_is_taken_back_and_the_old_owner_can_no_longer_submit() {
    let srv = TestServer::start().await;
    let pid = srv.project("甲").await;
    let (a, b) = (srv.paired("小明", Role::Member).await, srv.paired("小華", Role::Member).await);
    let item = add(&srv, &pid, "題目", store::QUEUED).await;
    claim(&a, &item.id).await.unwrap();
    sqlx::query("UPDATE intake_items SET claim_expires_at = 1 WHERE id = ?").bind(&item.id).execute(&srv.state().pools.intake).await.unwrap();
    channels::tick_once(srv.state()).await;
    assert_eq!(get(&b, &item.id).await.status, store::QUEUED, "逾期被收回");
    assert!(claim(&b, &item.id).await.is_ok(), "別人可以領");
    let late = a.request(Method::POST, &format!("/v1/items/{}/result", item.id), Some(json!({"reply":"太晚了"}))).await;
    assert!(matches!(late, Err(ClientError::Server(m)) if m.contains("不是你領取")), "原領取者晚交會被拒絕");
}

#[tokio::test]
async fn attachments_can_be_downloaded_by_the_claimer_but_not_from_outside_the_folder() {
    use aiterm_intake_core::attach::{save_all, Raw};
    let srv = TestServer::start().await;
    let pid = srv.project("甲").await;
    let m = srv.paired("小明", Role::Member).await;
    let item = add(&srv, &pid, "附檔", store::QUEUED).await;
    let (saved, _) = save_all(&srv.dir.path().join("attachments"), &item.id, vec![Raw { name: "log.bin".into(), bytes: vec![0, 1, 2, 3, 255] }]);
    store::add_attachment(&srv.state().pools.intake, &item.id, &saved[0]).await.unwrap();
    let rows: Vec<store::AttachmentRow> = m.get(&format!("/v1/items/{}/attachments", item.id)).await.unwrap();
    let bytes = m.download(&format!("/v1/attachments/{}/file", rows[0].id)).await.unwrap();
    assert_eq!(bytes, vec![0, 1, 2, 3, 255], "二進位內容原樣下載");
    sqlx::query("UPDATE intake_attachments SET path = ? WHERE id = ?").bind(srv.dir.path().join("secrets.key").to_string_lossy().as_ref()).bind(&rows[0].id).execute(&srv.state().pools.intake).await.unwrap();
    assert!(m.download(&format!("/v1/attachments/{}/file", rows[0].id)).await.is_err(), "目錄外的檔案不能下載");
}

#[tokio::test]
async fn claims_results_and_releases_are_audited() {
    let srv = TestServer::start().await;
    let pid = srv.project("甲").await;
    let (a, admin) = (srv.paired("小明", Role::Member).await, srv.paired("管理", Role::Admin).await);
    let item = add(&srv, &pid, "題目", store::QUEUED).await;
    claim(&a, &item.id).await.unwrap();
    a.request(Method::POST, &format!("/v1/items/{}/result", item.id), Some(json!({"reply":"好"}))).await.unwrap();
    let audit: Vec<serde_json::Value> = admin.get("/v1/audit").await.unwrap();
    for action in ["item.claim", "item.result"] {
        assert!(audit.iter().any(|e| e["action"] == action && e["member"] == "小明" && e["target"] == item.id.as_str()), "{action} 沒有稽核：{audit:?}");
    }
}
