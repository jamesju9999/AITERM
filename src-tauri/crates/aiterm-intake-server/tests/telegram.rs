//! Server 的 Telegram：長輪詢收訊息、回覆、設定。Telegram 用 wiremock 假的伺服器代替。
use aiterm_intake_core::api::Role;
use aiterm_intake_core::config::Config;
use aiterm_intake_core::store::IntakeItem;
use aiterm_intake_server::testkit::TestServer;
use reqwest::Method;
use serde_json::{json, Value};
use std::time::Duration;
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "123:ABC";

fn updates(list: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "ok": true, "result": list }))
}

async fn eventually<T, F: std::future::Future<Output = Option<T>>>(mut f: impl FnMut() -> F) -> T {
    for _ in 0..100 {
        if let Some(v) = f().await {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("等不到結果");
}

/// 一台設好 Telegram 的 server：專案啟用 Telegram，管理員的聊天室 id 是 111。
async fn setup(tg: &MockServer) -> (TestServer, aiterm_intake_client::api::Api, String) {
    let srv = TestServer::start().await;
    *srv.state().telegram_api_base.lock() = tg.uri();
    let admin = srv.paired("小華", Role::Admin).await;
    let pid = srv.project("範例科技").await;
    let cfg = Config { enabled: true, telegram_enabled: true, ..Config::default() };
    admin.request(Method::PUT, &format!("/v1/projects/{pid}/config"), Some(json!({"config": cfg, "telegram_default": false}))).await.unwrap();
    admin.request(Method::PUT, "/v1/telegram", Some(json!({"owner_chat_id": "111"}))).await.unwrap();
    (srv, admin, pid)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_owner_asks_with_a_slash_command_and_a_listed_chat_asks_directly_while_strangers_are_ignored() {
    let tg = MockServer::start().await;
    let (_srv, admin, pid) = setup(&tg).await;
    let s = json!({"address": "tg:222", "level": "external", "skip_auth": false, "label": "客服群"});
    admin.request(Method::PUT, &format!("/v1/projects/{pid}/senders"), Some(s)).await.unwrap();
    let msg = |id: i64, chat: i64, text: &str| json!({"update_id": id, "message": {"message_id": id, "chat": {"id": chat}, "from": {"first_name": "小明"}, "text": text}});
    Mock::given(method("GET")).and(path(format!("/bot{TOKEN}/getUpdates"))).respond_with(updates(json!([msg(1, 111, "/ask 印表機卡紙"), msg(2, 222, "網路很慢"), msg(3, 333, "陌生人的訊息"), msg(4, 111, "沒有指令的一般對話")]))).up_to_n_times(1).mount(&tg).await;
    Mock::given(method("GET")).and(path(format!("/bot{TOKEN}/getUpdates"))).respond_with(updates(json!([]))).mount(&tg).await;
    admin.request(Method::PUT, "/v1/secrets/telegram_bot_token", Some(json!({"value": TOKEN}))).await.unwrap();

    let items: Vec<IntakeItem> = eventually(|| async {
        let v: Vec<IntakeItem> = admin.get(&format!("/v1/projects/{pid}/items")).await.unwrap();
        (v.len() >= 2).then_some(v)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let items: Vec<IntakeItem> = admin.get(&format!("/v1/projects/{pid}/items")).await.unwrap_or(items);
    let mut got: Vec<(String, String, String)> = items.iter().map(|i| (i.source.clone(), i.sender.clone(), i.body.clone())).collect();
    got.sort();
    assert_eq!(got, vec![("telegram".to_string(), "tg:111".to_string(), "印表機卡紙".to_string()), ("telegram".to_string(), "tg:222".to_string(), "網路很慢".to_string())], "主人的 /ask 與白名單聊天室進來；陌生人與沒有指令的一般對話不進來");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_telegram_question_is_answered_in_its_chat_as_a_reply_to_the_original_message() {
    let tg = MockServer::start().await;
    let (srv, admin, pid) = setup(&tg).await;
    Mock::given(method("POST")).and(path(format!("/bot{TOKEN}/sendMessage"))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true}))).mount(&tg).await;
    admin.request(Method::PUT, "/v1/secrets/telegram_bot_token", Some(json!({"value": TOKEN}))).await.unwrap();
    sqlx::query("INSERT INTO intake_items (id,source,sender,sender_level,subject,body,message_id,status,reply_draft,created_at,updated_at,kind,project_id) VALUES ('t1','telegram','tg:222','external','網路很慢','body','tg:222:77','draft','請先重開路由器。',1,1,'question',?)")
        .bind(&pid)
        .execute(&srv.state().pools.intake)
        .await
        .unwrap();

    admin.request(Method::POST, "/v1/items/t1/reply", None).await.unwrap();
    let sent = tg.received_requests().await.unwrap().into_iter().find(|r| r.url.path().ends_with("/sendMessage")).expect("沒有送出");
    let body: Value = serde_json::from_slice(&sent.body).unwrap();
    assert_eq!((body["chat_id"].as_i64(), body["text"].as_str(), body["reply_parameters"]["message_id"].as_i64()), (Some(222), Some("請先重開路由器。"), Some(77)));
    let item: IntakeItem = admin.get("/v1/items/t1").await.unwrap();
    assert_eq!(item.status, "done");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn another_program_polling_the_same_bot_is_reported_not_hidden() {
    let tg = MockServer::start().await;
    let (_srv, admin, _pid) = setup(&tg).await;
    Mock::given(method("GET")).and(path_regex(r"/getUpdates$")).respond_with(ResponseTemplate::new(409).set_body_json(json!({"ok": false}))).mount(&tg).await;
    admin.request(Method::PUT, "/v1/secrets/telegram_bot_token", Some(json!({"value": TOKEN}))).await.unwrap();
    let v: Value = eventually(|| async {
        let v: Value = admin.get("/v1/telegram").await.unwrap();
        v["error"].is_string().then_some(v)
    })
    .await;
    assert!(v["error"].as_str().unwrap().contains("409"), "{v}");
    assert_eq!((v["has_token"].as_bool(), v["listening"].as_bool()), (Some(true), Some(false)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn settings_are_validated_and_only_admins_can_change_them_and_the_token_is_never_returned() {
    let tg = MockServer::start().await;
    let (_srv, admin, _pid) = setup(&tg).await;
    let member = _srv.paired("小明", Role::Member).await;
    assert!(admin.request(Method::PUT, "/v1/telegram", Some(json!({"owner_chat_id": "不是數字"}))).await.is_err());
    assert!(member.request(Method::PUT, "/v1/telegram", Some(json!({"owner_chat_id": "5"}))).await.is_err());
    admin.request(Method::PUT, "/v1/secrets/telegram_bot_token", Some(json!({"value": "secret-token"}))).await.unwrap();
    let v: Value = member.get("/v1/telegram").await.unwrap();
    assert!(!v.to_string().contains("secret-token"));
    assert_eq!((v["settings"]["owner_chat_id"].as_str(), v["has_token"].as_bool()), (Some("111"), Some(true)));
}
