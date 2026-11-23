//! Server 收 LINE 訊息的端到端測試：設定走 API、webhook 走真的 HTTP、題目從清單 API 讀出。
use aiterm_intake_core::api::Role;
use aiterm_intake_core::config::Config;
use aiterm_intake_core::store::IntakeItem;
use aiterm_intake_server::channels;
use aiterm_intake_server::testkit::TestServer;
use base64::Engine;
use hmac::{Hmac, Mac};
use reqwest::Method;
use serde_json::json;
use sha2::Sha256;

const SECRET: &str = "s3cret";
const GROUP: &str = "Cb345e5d492c48c02a3eaa279924c52d2";
const GROUP2: &str = "Cb345e5d492c48c02a3eaa279924c52d3";
const CUSTOMER: &str = "U2222222222222222222222222222222b";

fn sign(body: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(body.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn group_text(group: &str, id: &str, text: &str, mention: bool, age_secs: i64) -> String {
    let m = if mention { r#","mention":{"mentionees":[{"index":0,"length":3,"type":"user","isSelf":true}]}"# } else { "" };
    let ts = (chrono::Utc::now().timestamp() - age_secs) * 1000;
    format!(r#"{{"events":[{{"type":"message","timestamp":{ts},"source":{{"type":"group","groupId":"{group}","userId":"{CUSTOMER}"}},"message":{{"type":"text","id":"{id}","text":"{text}"{m}}}}}]}}"#)
}

async fn post_hook(port: u16, body: &str, sig: Option<&str>) -> u16 {
    let mut req = reqwest::Client::new().post(format!("http://127.0.0.1:{port}/line/webhook")).body(body.to_string());
    if let Some(s) = sig {
        req = req.header("x-line-signature", s);
    }
    req.send().await.unwrap().status().as_u16()
}

/// 起一台已設好 LINE 的 server，回傳（server、管理員 Client、專案 id、webhook 埠）。
async fn line_server(groups: &[&str]) -> (TestServer, aiterm_intake_client::api::Api, String, u16) {
    let srv = TestServer::start().await;
    let admin = srv.paired("小華", Role::Admin).await;
    let pid = srv.project("範例科技").await;
    let port = free_port();
    admin.request(Method::PUT, "/v1/secrets/line_channel_secret", Some(json!({"value": SECRET}))).await.unwrap();
    let cfg = Config { line_enabled: true, line_port: port, ..Config::default() };
    admin.request(Method::PUT, &format!("/v1/projects/{pid}/config"), Some(json!({"config": cfg, "telegram_default": false}))).await.unwrap();
    for g in groups {
        let s = json!({"address": format!("line:{g}"), "level": "external", "skip_auth": false, "label": "客服群"});
        admin.request(Method::PUT, &format!("/v1/projects/{pid}/senders"), Some(s)).await.unwrap();
    }
    (srv, admin, pid, port)
}

async fn items(admin: &aiterm_intake_client::api::Api, pid: &str) -> Vec<IntakeItem> {
    admin.get(&format!("/v1/projects/{pid}/items")).await.unwrap()
}

#[tokio::test]
async fn a_signed_mention_in_a_whitelisted_group_opens_an_item_visible_to_every_member() {
    let (srv, admin, pid, port) = line_server(&[GROUP]).await;
    let status: serde_json::Value = admin.get("/v1/status").await.unwrap();
    assert_eq!(status["line_listening"], true, "設定好祕密與啟用之後 webhook 就在聽");

    let body = group_text(GROUP, "m1", "@bot 電腦開不了機", true, 0);
    assert_eq!(post_hook(port, &body, None).await, 401, "沒有簽章不收");
    assert_eq!(post_hook(port, &body, Some("bad")).await, 401);
    assert_eq!(post_hook(port, &body, Some(&sign(&body))).await, 200);

    let member = srv.paired("小明", Role::Member).await;
    let list = items(&member, &pid).await;
    assert_eq!(list.len(), 1, "另一位成員也看得到同一題");
    assert_eq!((list[0].source.as_str(), list[0].project_id.as_str()), ("line", pid.as_str()));
    assert!(list[0].body.contains("電腦開不了機"));
}

#[tokio::test]
async fn a_group_that_is_not_whitelisted_creates_nothing() {
    let (_srv, admin, pid, port) = line_server(&[GROUP]).await;
    let body = group_text(GROUP2, "m1", "@bot 幫幫我", true, 0);
    assert_eq!(post_hook(port, &body, Some(&sign(&body))).await, 200, "LINE 要的是 2xx，不能因為沒授權就回錯誤讓它重送");
    assert!(items(&admin, &pid).await.is_empty());
    let unknown: Vec<serde_json::Value> = admin.get("/v1/line/unknown").await.unwrap();
    assert_eq!(unknown.len(), 1, "只記下群組編號，管理員可以決定要不要加入白名單");
}

#[tokio::test]
async fn a_quiet_customer_message_without_a_mention_opens_an_item_on_the_next_tick() {
    let (srv, admin, pid, port) = line_server(&[GROUP]).await;
    let body = group_text(GROUP, "m1", "印表機又卡紙了", false, 600);
    assert_eq!(post_hook(port, &body, Some(&sign(&body))).await, 200);
    assert!(items(&admin, &pid).await.is_empty(), "沒 @ 不會立刻開題");
    // AI 沒設定：照預設當成新問題，寧可多開不漏客戶。
    assert!(channels::tick_once(srv.state()).await, "安靜夠久，這一輪掃描就開題");
    let list = items(&admin, &pid).await;
    assert_eq!(list.len(), 1);
    assert!(list[0].body.contains("印表機又卡紙了"));
    assert!(!channels::tick_once(srv.state()).await, "同一批訊息不會重複開題");
}

#[tokio::test]
async fn when_the_ai_says_it_is_chit_chat_no_item_is_opened() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let ai = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"choices":[{"message":{"content":"{\"new_problem\": false}"}}]})))
        .expect(1)
        .mount(&ai)
        .await;
    let (srv, admin, pid, port) = line_server(&[GROUP]).await;
    admin.request(Method::PUT, "/v1/ai", Some(json!({"provider":"openai_compat","model":"m","base_url": ai.uri()}))).await.unwrap();
    admin.request(Method::PUT, "/v1/secrets/ai_api_key", Some(json!({"value":"K"}))).await.unwrap();
    let body = group_text(GROUP, "m1", "好的謝謝", false, 600);
    post_hook(port, &body, Some(&sign(&body))).await;
    assert!(!channels::tick_once(srv.state()).await);
    assert!(items(&admin, &pid).await.is_empty(), "AI 判斷不是新問題，就不開題");
}

#[tokio::test]
async fn disabling_line_or_removing_the_secret_stops_the_webhook() {
    let (_srv, admin, pid, port) = line_server(&[GROUP]).await;
    let body = group_text(GROUP, "m1", "@bot x", true, 0);
    assert_eq!(post_hook(port, &body, Some(&sign(&body))).await, 200);
    admin.request(Method::DELETE, "/v1/secrets/line_channel_secret", None).await.unwrap();
    let status: serde_json::Value = admin.get("/v1/status").await.unwrap();
    assert_eq!(status["line_listening"], false);
    assert!(reqwest::Client::new().post(format!("http://127.0.0.1:{port}/line/webhook")).body(body).send().await.is_err(), "埠已關閉");
    let _ = pid;
}
