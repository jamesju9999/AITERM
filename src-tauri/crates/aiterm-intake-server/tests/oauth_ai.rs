//! Server 的訂閱登入（Claude）：憑證存放、續期（一次性續期憑證不能被兩個請求同時用）、呼叫與狀態。
//! 供應商用 wiremock 假的伺服器代替；真實帳號的登入由實機驗證。
use aiterm_ai_oauth::anthropic::{Endpoints, CLAUDE_CODE_SENTINEL, OAUTH_BETA_HEADER};
use aiterm_intake_core::api::Role;
use aiterm_intake_core::host::Secrets;
use aiterm_intake_server::testkit::TestServer;
use reqwest::Method;
use serde_json::{json, Value};
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn setup(up: &MockServer) -> (TestServer, aiterm_intake_client::api::Api) {
    let srv = TestServer::start().await;
    srv.state().ai.set_oauth_endpoints(Endpoints { token_url: format!("{}/oauth/token", up.uri()), api_base: up.uri() });
    let admin = srv.paired("小華", Role::Admin).await;
    (srv, admin)
}

async fn login(admin: &aiterm_intake_client::api::Api, access: &str, refresh: &str, expires_in: u64) {
    admin.request(Method::PUT, "/v1/ai/oauth", Some(json!({"provider": "anthropic_oauth", "access": access, "refresh": refresh, "expires_in": expires_in}))).await.unwrap();
    admin.request(Method::PUT, "/v1/ai", Some(json!({"provider": "anthropic_oauth", "model": "claude-x", "base_url": ""}))).await.unwrap();
}

fn messages_ok(text: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"content": [{"type": "text", "text": text}]}))
}

async fn test_call(admin: &aiterm_intake_client::api::Api) -> Value {
    admin.typed(Method::POST, "/v1/ai/test", Some(json!({"provider": "anthropic_oauth", "model": "claude-x"}))).await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_logged_in_server_calls_claude_with_the_subscription_token_and_the_claude_code_identity() {
    let up = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/messages")).and(header("authorization", "Bearer ACCESS")).and(body_partial_json(json!({"model": "claude-x", "system": [{"type": "text", "text": CLAUDE_CODE_SENTINEL}]}))).respond_with(messages_ok("OK")).mount(&up).await;
    let (srv, admin) = setup(&up).await;
    login(&admin, "ACCESS", "REFRESH", 3600).await;

    let r = test_call(&admin).await;
    assert_eq!((r["ok"].as_bool(), r["message"].as_str()), (Some(true), Some("OK")), "{r}");
    let sent = &up.received_requests().await.unwrap()[0];
    assert_eq!(sent.headers.get("anthropic-beta").unwrap().to_str().unwrap(), OAUTH_BETA_HEADER);
    let view: Value = admin.get("/v1/ai").await.unwrap();
    assert_eq!((view["configured"].as_bool(), view["login"].as_str()), (Some(true), Some("ok")));
    assert!(!view.to_string().contains("ACCESS") && !view.to_string().contains("REFRESH"), "憑證不會被讀回");
    // 實際走 AiComplete（群組分類用的那條路）也一樣。
    use aiterm_intake_core::host::AiComplete;
    assert_eq!(srv.state().ai.complete("sys", "hi").await.unwrap(), "OK");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_expiring_token_is_refreshed_first_and_the_rotated_refresh_token_is_what_gets_stored() {
    let up = MockServer::start().await;
    Mock::given(method("POST")).and(path("/oauth/token")).and(body_partial_json(json!({"grant_type": "refresh_token", "refresh_token": "OLD"}))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": "NEW-ACCESS", "refresh_token": "NEW-REFRESH", "expires_in": 28800}))).mount(&up).await;
    Mock::given(method("POST")).and(path("/v1/messages")).and(header("authorization", "Bearer NEW-ACCESS")).respond_with(messages_ok("OK")).mount(&up).await;
    let (srv, admin) = setup(&up).await;
    login(&admin, "OLD-ACCESS", "OLD", 10).await; // 10 秒後到期，落在提早續期的範圍內

    assert_eq!(test_call(&admin).await["ok"], true);
    assert_eq!(srv.state().secrets.get("ai_oauth_refresh").as_deref(), Some("NEW-REFRESH"), "一次性的續期憑證用過就失效，新的必須存下來");
    assert_eq!(srv.state().secrets.get("ai_oauth_access").as_deref(), Some("NEW-ACCESS"));
    assert_eq!(test_call(&admin).await["ok"], true);
    let refreshes = up.received_requests().await.unwrap().iter().filter(|r| r.url.path() == "/oauth/token").count();
    assert_eq!(refreshes, 1, "新的憑證有效，第二次不該再續期");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn many_simultaneous_calls_share_one_refresh_because_the_refresh_token_is_single_use() {
    let up = MockServer::start().await;
    Mock::given(method("POST")).and(path("/oauth/token")).and(body_partial_json(json!({"refresh_token": "OLD"}))).respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_millis(200)).set_body_json(json!({"access_token": "A2", "refresh_token": "R2", "expires_in": 28800}))).mount(&up).await;
    // 一次性：拿舊的再來一次就被拒絕（真實的供應商就是這樣）。
    Mock::given(method("POST")).and(path("/oauth/token")).respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"}))).mount(&up).await;
    Mock::given(method("POST")).and(path("/v1/messages")).respond_with(messages_ok("OK")).mount(&up).await;
    let (srv, admin) = setup(&up).await;
    login(&admin, "OLD-ACCESS", "OLD", 5).await;

    let calls: Vec<_> = (0..8).map(|_| srv.state().ai.access_token()).collect();
    let results = futures_util::future::join_all(calls).await;
    assert!(results.iter().all(|r| r.as_deref() == Ok("A2")), "{results:?}");
    let refreshes = up.received_requests().await.unwrap().iter().filter(|r| r.url.path() == "/oauth/token").count();
    assert_eq!(refreshes, 1, "同時進來的請求要排隊共用同一次續期");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rejected_refresh_on_an_expired_token_asks_for_a_new_login_and_a_new_login_fixes_it() {
    let up = MockServer::start().await;
    Mock::given(method("POST")).and(path("/oauth/token")).respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"}))).mount(&up).await;
    Mock::given(method("POST")).and(path("/v1/messages")).and(header("authorization", "Bearer FRESH")).respond_with(messages_ok("OK")).mount(&up).await;
    let (_srv, admin) = setup(&up).await;
    login(&admin, "DEAD", "DEAD-R", 0).await; // 已經過期

    let r = test_call(&admin).await;
    assert_eq!(r["ok"], false);
    assert!(r["message"].as_str().unwrap().contains("重新"), "{r}");
    let view: Value = admin.get("/v1/ai").await.unwrap();
    assert_eq!(view["login"], "needs_login");
    assert!(view["login_error"].as_str().unwrap().contains("400"));

    login(&admin, "FRESH", "FRESH-R", 3600).await;
    assert_eq!(test_call(&admin).await["ok"], true);
    let view: Value = admin.get("/v1/ai").await.unwrap();
    assert_eq!(view["login"], "ok");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_refresh_does_not_log_out_a_token_that_has_not_actually_expired() {
    let up = MockServer::start().await;
    Mock::given(method("POST")).and(path("/oauth/token")).respond_with(ResponseTemplate::new(500)).mount(&up).await;
    Mock::given(method("POST")).and(path("/v1/messages")).and(header("authorization", "Bearer STILL-GOOD")).respond_with(messages_ok("OK")).mount(&up).await;
    let (_srv, admin) = setup(&up).await;
    login(&admin, "STILL-GOOD", "R", 100).await; // 還有 100 秒：在提早續期範圍內，但沒過期
    assert_eq!(test_call(&admin).await["ok"], true, "續期暫時失敗時先用還沒過期的憑證");
    let view: Value = admin.get("/v1/ai").await.unwrap();
    assert_eq!(view["login"], "ok");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_list_uses_the_subscription_token_and_logout_removes_everything() {
    let up = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/models")).and(header("authorization", "Bearer ACCESS")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"id": "claude-b"}, {"id": "claude-a"}]}))).mount(&up).await;
    let (srv, admin) = setup(&up).await;
    login(&admin, "ACCESS", "R", 3600).await;
    let models: Vec<String> = admin.typed(Method::POST, "/v1/ai/models", Some(json!({"provider": "anthropic_oauth"}))).await.unwrap();
    assert_eq!(models, vec!["claude-a", "claude-b"]);

    admin.request(Method::DELETE, "/v1/ai/oauth", None).await.unwrap();
    for id in ["ai_oauth_access", "ai_oauth_refresh", "ai_oauth_expires_at"] {
        assert!(srv.state().secrets.get(id).is_none(), "{id} 要被刪掉");
    }
    let view: Value = admin.get("/v1/ai").await.unwrap();
    assert_eq!((view["configured"].as_bool(), view["login"].as_str()), (Some(false), Some("none")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_admins_can_log_the_server_in_and_incomplete_credentials_are_refused() {
    let up = MockServer::start().await;
    let (_srv, admin) = setup(&up).await;
    let member = _srv.paired("小明", Role::Member).await;
    let body = json!({"provider": "anthropic_oauth", "access": "A", "refresh": "R", "expires_in": 60});
    assert!(member.request(Method::PUT, "/v1/ai/oauth", Some(body.clone())).await.is_err());
    assert!(member.request(Method::DELETE, "/v1/ai/oauth", None).await.is_err());
    assert!(admin.request(Method::PUT, "/v1/ai/oauth", Some(json!({"provider": "anthropic_oauth", "access": "A", "refresh": "", "expires_in": 60}))).await.is_err(), "沒有續期憑證就撐不過一小時，要拒絕");
    assert!(admin.request(Method::PUT, "/v1/ai/oauth", Some(json!({"provider": "openai", "access": "A", "refresh": "R"}))).await.is_err());
    assert!(admin.request(Method::PUT, "/v1/ai/oauth", Some(body)).await.is_ok());
}
