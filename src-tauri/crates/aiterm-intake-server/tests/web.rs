//! Server 的網頁表單端到端測試：設定走 API、表單走真的 HTTP、寄信改丟進測試通道。
use aiterm_intake_core::api::Role;
use aiterm_intake_core::config::{Config, WebMode};
use aiterm_intake_core::store::IntakeItem;
use aiterm_intake_server::testkit::TestServer;
use reqwest::Method;
use serde_json::{json, Value};
use std::time::Duration;

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

async fn post(port: u16, path: &str, body: Value) -> (u16, Value) {
    let r = reqwest::Client::new().post(format!("http://127.0.0.1:{port}{path}")).json(&body).send().await.unwrap();
    let status = r.status().as_u16();
    (status, r.json().await.unwrap_or(Value::Null))
}

type Mails = tokio::sync::mpsc::UnboundedReceiver<(String, String, String)>;

/// 起一台開了網頁表單的 server：bob 在白名單內；寄信丟進回傳的通道。
async fn web_server() -> (TestServer, aiterm_intake_client::api::Api, String, u16, Mails) {
    let srv = TestServer::start().await;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    *srv.state().mail_tap.lock() = Some(tx);
    let admin = srv.paired("小華", Role::Admin).await;
    let pid = srv.project("範例科技").await;
    let port = free_port();
    let cfg = Config { web_mode: WebMode::Local, web_port: port, ..Config::default() };
    admin.request(Method::PUT, &format!("/v1/projects/{pid}/config"), Some(json!({"config": cfg, "telegram_default": false}))).await.unwrap();
    let s = json!({"address": "bob@corp.com", "level": "internal", "skip_auth": false, "label": "小明"});
    admin.request(Method::PUT, &format!("/v1/projects/{pid}/senders"), Some(s)).await.unwrap();
    (srv, admin, pid, port, rx)
}

async fn next_mail(rx: &mut Mails) -> (String, String, String) {
    tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.expect("沒有收到信").unwrap()
}

fn code_in(body: &str) -> String {
    let digits: Vec<String> = body.split(|c: char| !c.is_ascii_digit()).filter(|w| w.len() == 6).map(String::from).collect();
    digits.into_iter().next().expect("信裡沒有驗證碼")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_whitelisted_member_gets_a_code_by_mail_logs_in_and_submits_a_question() {
    let (_srv, admin, pid, port, mut rx) = web_server().await;
    let base = format!("/p/{}", &pid[..8]);

    let (st, _) = post(port, &format!("{base}/api/code"), json!({"email": "bob@corp.com"})).await;
    assert_eq!(st, 200);
    let (to, _, body) = next_mail(&mut rx).await;
    assert_eq!(to, "bob@corp.com");

    let (st, login) = post(port, &format!("{base}/api/login"), json!({"email": "bob@corp.com", "code": code_in(&body)})).await;
    assert_eq!(st, 200, "{login}");
    let token = login["token"].as_str().unwrap().to_string();

    let (st, _) = post(port, &format!("{base}/api/submit"), json!({"token": token, "subject": "列印問題", "body": "印表機卡紙"})).await;
    assert_eq!(st, 200);
    let items: Vec<IntakeItem> = admin.get(&format!("/v1/projects/{pid}/items")).await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!((items[0].source.as_str(), items[0].sender.as_str(), items[0].subject.as_str()), ("web", "bob@corp.com", "列印問題"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stranger_gets_the_same_answer_but_no_mail() {
    let (_srv, _admin, pid, port, mut rx) = web_server().await;
    let base = format!("/p/{}", &pid[..8]);
    let (a, ja) = post(port, &format!("{base}/api/code"), json!({"email": "bob@corp.com"})).await;
    let (b, jb) = post(port, &format!("{base}/api/code"), json!({"email": "eve@evil.com"})).await;
    assert_eq!((a, &ja), (b, &jb), "回應不能透露白名單");
    assert_eq!(next_mail(&mut rx).await.0, "bob@corp.com");
    assert!(tokio::time::timeout(Duration::from_millis(500), rx.recv()).await.is_err(), "陌生人不該收到信");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turning_the_form_off_stops_the_server_and_status_reports_it() {
    let (srv, admin, pid, port, _rx) = web_server().await;
    let status: Value = admin.get("/v1/status").await.unwrap();
    assert_eq!((status["web_listening"].as_bool(), status["web_port"].as_u64()), (Some(true), Some(port as u64)));
    let cfg = Config { web_mode: WebMode::Off, web_port: port, ..Config::default() };
    admin.request(Method::PUT, &format!("/v1/projects/{pid}/config"), Some(json!({"config": cfg, "telegram_default": false}))).await.unwrap();
    let status: Value = admin.get("/v1/status").await.unwrap();
    assert_eq!(status["web_listening"].as_bool(), Some(false));
    drop(srv);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_web_question_is_answered_by_mail() {
    let (srv, admin, pid, port, mut rx) = web_server().await;
    let base = format!("/p/{}", &pid[..8]);
    post(port, &format!("{base}/api/code"), json!({"email": "bob@corp.com"})).await;
    let code = code_in(&next_mail(&mut rx).await.2);
    let (_, login) = post(port, &format!("{base}/api/login"), json!({"email": "bob@corp.com", "code": code})).await;
    post(port, &format!("{base}/api/submit"), json!({"token": login["token"], "subject": "列印問題", "body": "印表機卡紙"})).await;
    let items: Vec<IntakeItem> = admin.get(&format!("/v1/projects/{pid}/items")).await.unwrap();
    let id = items[0].id.clone();
    sqlx::query("UPDATE intake_items SET status='draft', reply_draft='請先關機再取出卡住的紙。' WHERE id=?").bind(&id).execute(&srv.state().pools.intake).await.unwrap();

    admin.request(Method::POST, &format!("/v1/items/{id}/reply"), None).await.unwrap();
    let (to, subject, body) = next_mail(&mut rx).await;
    assert_eq!(to, "bob@corp.com");
    assert!(subject.contains("列印問題"), "{subject}");
    assert!(body.contains("取出卡住的紙"));
    let item: IntakeItem = admin.get(&format!("/v1/items/{id}")).await.unwrap();
    assert_eq!(item.status, "done");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mail_settings_round_trip_and_the_password_is_never_returned() {
    let srv = TestServer::start().await;
    let admin = srv.paired("小華", Role::Admin).await;
    let member = srv.paired("小明", Role::Member).await;
    let v: Value = admin.get("/v1/mail").await.unwrap();
    assert_eq!(v["configured"], false);
    admin.request(Method::PUT, "/v1/mail", Some(json!({"smtp_host": "smtp.example.com", "smtp_port": 465, "username": "bot@example.com", "from": ""}))).await.unwrap();
    admin.request(Method::PUT, "/v1/secrets/smtp_password", Some(json!({"value": "pw-123"}))).await.unwrap();
    let v: Value = admin.get("/v1/mail").await.unwrap();
    assert_eq!((v["configured"].as_bool(), v["settings"]["smtp_port"].as_u64()), (Some(true), Some(465)));
    assert!(!v.to_string().contains("pw-123"));
    assert!(member.request(Method::PUT, "/v1/mail", Some(json!({"smtp_host": "x", "smtp_port": 25, "username": "u", "from": ""}))).await.is_err(), "成員不能改寄信設定");
}
