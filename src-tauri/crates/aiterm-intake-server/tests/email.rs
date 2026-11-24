//! Server 的 Email：回覆信件與設定、狀態。收信的輪詢要真的 IMAP 信箱，改由實機驗證；
//! 進度判斷的邏輯在 `imap` 模組的單元測試，歸屬規則在 core 的 `mail_source` 測試。
use aiterm_intake_core::api::Role;
use aiterm_intake_core::config::Config;
use aiterm_intake_core::store::IntakeItem;
use aiterm_intake_server::testkit::TestServer;
use reqwest::Method;
use serde_json::{json, Value};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_email_question_is_answered_by_mail_in_the_original_thread() {
    let srv = TestServer::start().await;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    *srv.state().mail_tap.lock() = Some(tx);
    let admin = srv.paired("小華", Role::Admin).await;
    let pid = srv.project("範例科技").await;
    let cfg = Config { enabled: true, ..Config::default() };
    admin.request(Method::PUT, &format!("/v1/projects/{pid}/config"), Some(json!({"config": cfg, "telegram_default": false}))).await.unwrap();
    sqlx::query("INSERT INTO intake_items (id,source,sender,sender_level,subject,body,message_id,status,reply_draft,created_at,updated_at,kind,project_id,to_addrs,cc_addrs) VALUES ('e1','email','bob@corp.com','internal','印表機卡紙','請協助','<m1@corp.com>','draft','請先關機再取出卡住的紙。',1,1,'question',?,'help@example.com,amy@corp.com','lee@corp.com')")
        .bind(&pid)
        .execute(&srv.state().pools.intake)
        .await
        .unwrap();

    admin.request(Method::POST, "/v1/items/e1/reply", None).await.unwrap();
    let (to, subject, body) = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.expect("沒有寄出").unwrap();
    assert_eq!(to, "bob@corp.com");
    assert!(subject.starts_with("Re:") && subject.contains("印表機卡紙"), "{subject}");
    assert!(body.contains("取出卡住的紙"));
    let item: IntakeItem = admin.get("/v1/items/e1").await.unwrap();
    assert_eq!(item.status, "done");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mail_settings_carry_the_receiving_side_and_status_reports_it() {
    let srv = TestServer::start().await;
    let admin = srv.paired("小華", Role::Admin).await;
    let status: Value = admin.get("/v1/status").await.unwrap();
    assert_eq!(status["email_receiving"], false);
    admin.request(Method::PUT, "/v1/mail", Some(json!({"smtp_host": "smtp.example.com", "smtp_port": 587, "imap_host": "imap.example.com", "imap_port": 993, "username": "bot@example.com", "from": ""}))).await.unwrap();
    admin.request(Method::PUT, "/v1/secrets/smtp_password", Some(json!({"value": "pw"}))).await.unwrap();
    let v: Value = admin.get("/v1/mail").await.unwrap();
    assert_eq!(v["settings"]["imap_host"], "imap.example.com");
    let status: Value = admin.get("/v1/status").await.unwrap();
    assert_eq!(status["email_receiving"], true);
    assert!(status["email_error"].is_null());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn settings_saved_before_receiving_existed_still_load() {
    let srv = TestServer::start().await;
    let admin = srv.paired("小華", Role::Admin).await;
    sqlx::query("INSERT OR REPLACE INTO intake_settings(key,value) VALUES ('server_mail','{\"smtp_host\":\"smtp.example.com\",\"smtp_port\":587,\"username\":\"bot@example.com\",\"from\":\"\"}')")
        .execute(&srv.state().pools.intake)
        .await
        .unwrap();
    let v: Value = admin.get("/v1/mail").await.unwrap();
    assert_eq!((v["settings"]["smtp_host"].as_str(), v["settings"]["imap_port"].as_u64(), v["settings"]["imap_host"].as_str()), (Some("smtp.example.com"), Some(993), Some("")));
}
