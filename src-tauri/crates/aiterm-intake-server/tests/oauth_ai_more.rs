//! Server 的 ChatGPT（Codex）與 Gemini（Antigravity）訂閱登入：呼叫、續期、各家差異與切換供應商。
//! 供應商用 wiremock 假的伺服器代替；真實帳號的登入由實機驗證。
use aiterm_ai_oauth::{antigravity, codex};
use aiterm_intake_core::api::Role;
use aiterm_intake_core::host::Secrets;
use aiterm_intake_server::testkit::TestServer;
use reqwest::Method;
use serde_json::{json, Value};
use wiremock::matchers::{body_partial_json, body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

type Admin = aiterm_intake_client::api::Api;

async fn setup(up: &MockServer) -> (TestServer, Admin) {
    let srv = TestServer::start().await;
    srv.state().ai.set_codex_endpoints(codex::Endpoints { token_url: format!("{}/oauth/token", up.uri()), api_base: up.uri() });
    srv.state().ai.set_antigravity_endpoints(antigravity::Endpoints { token_url: format!("{}/token", up.uri()), api_base: up.uri() });
    let admin = srv.paired("小華", Role::Admin).await;
    (srv, admin)
}

async fn login(admin: &Admin, provider: &str, extra: Value, expires_in: u64) -> Result<(), String> {
    let mut body = json!({"provider": provider, "access": "ACCESS", "refresh": "REFRESH", "expires_in": expires_in});
    body.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    admin.request(Method::PUT, "/v1/ai/oauth", Some(body)).await.map(|_| ()).map_err(|e| format!("{e:?}"))?;
    admin.request(Method::PUT, "/v1/ai", Some(json!({"provider": provider, "model": "m-1", "base_url": ""}))).await.map(|_| ()).map_err(|e| format!("{e:?}"))
}

async fn test_call(admin: &Admin, provider: &str) -> Value {
    admin.typed(Method::POST, "/v1/ai/test", Some(json!({"provider": provider, "model": "m-1"}))).await.unwrap()
}

fn codex_stream(text: &str) -> ResponseTemplate {
    let body = format!("data: {}\n\ndata: {}\n\n", json!({"type": "response.output_text.delta", "delta": text}), json!({"type": "response.completed", "response": {}}));
    ResponseTemplate::new(200).insert_header("content-type", "text/event-stream").set_body_string(body)
}

fn gemini_stream(text: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_string(format!("data: {}\n\n", json!({"response": {"candidates": [{"content": {"parts": [{"text": text}]}, "finishReason": "STOP"}]}})))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_codex_login_calls_chatgpt_with_the_account_id_and_the_codex_identity() {
    let up = MockServer::start().await;
    Mock::given(method("POST")).and(path("/backend-api/codex/responses")).and(header("authorization", "Bearer ACCESS")).and(header("chatgpt-account-id", "acct-9")).and(header("originator", "codex_cli_rs")).respond_with(codex_stream("OK")).mount(&up).await;
    let (srv, admin) = setup(&up).await;
    login(&admin, "codex_oauth", json!({"account_id": "acct-9"}), 3600).await.unwrap();

    let r = test_call(&admin, "codex_oauth").await;
    assert_eq!((r["ok"].as_bool(), r["message"].as_str()), (Some(true), Some("OK")), "{r}");
    let view: Value = admin.get("/v1/ai").await.unwrap();
    assert_eq!((view["configured"].as_bool(), view["login"].as_str(), view["settings"]["provider"].as_str()), (Some(true), Some("ok"), Some("codex_oauth")));
    assert!(!view.to_string().contains("acct-9") && !view.to_string().contains("REFRESH"), "憑證與帳號 id 不會被讀回");
    use aiterm_intake_core::host::AiComplete;
    assert_eq!(srv.state().ai.complete("sys", "hi").await.unwrap(), "OK", "群組分類走的 AiComplete 也一樣");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_codex_refresh_uses_the_one_time_refresh_token_and_stores_the_rotated_one() {
    let up = MockServer::start().await;
    Mock::given(method("POST")).and(path("/oauth/token")).and(body_string_contains("refresh_token=REFRESH")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": "NEW-ACCESS", "refresh_token": "NEW-REFRESH", "expires_in": 3600}))).mount(&up).await;
    Mock::given(method("POST")).and(path("/backend-api/codex/responses")).and(header("authorization", "Bearer NEW-ACCESS")).respond_with(codex_stream("OK")).mount(&up).await;
    let (srv, admin) = setup(&up).await;
    login(&admin, "codex_oauth", json!({}), 10).await.unwrap();

    assert_eq!(test_call(&admin, "codex_oauth").await["ok"], true);
    assert_eq!(srv.state().secrets.get("ai_oauth_refresh").as_deref(), Some("NEW-REFRESH"));
    assert_eq!(srv.state().secrets.get("ai_oauth_access").as_deref(), Some("NEW-ACCESS"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_gemini_login_needs_a_project_id_and_calls_with_it() {
    let up = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1internal:streamGenerateContent")).and(header("authorization", "Bearer ACCESS")).and(body_partial_json(json!({"project": "proj-7", "model": "m-1"}))).respond_with(gemini_stream("OK")).mount(&up).await;
    let (_srv, admin) = setup(&up).await;
    assert!(login(&admin, "antigravity_oauth", json!({}), 3600).await.is_err(), "沒有專案 id 的登入呼叫不會成功，要在登入時就拒絕");
    login(&admin, "antigravity_oauth", json!({"project_id": "proj-7"}), 3600).await.unwrap();
    let r = test_call(&admin, "antigravity_oauth").await;
    assert_eq!((r["ok"].as_bool(), r["message"].as_str()), (Some(true), Some("OK")), "{r}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn google_tokens_are_renewed_fifteen_minutes_early_and_the_refresh_token_is_kept() {
    let up = MockServer::start().await;
    Mock::given(method("POST")).and(path("/token")).and(body_string_contains("grant_type=refresh_token")).and(body_string_contains("refresh_token=REFRESH")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": "G2", "expires_in": 3599}))).mount(&up).await;
    Mock::given(method("POST")).and(path("/v1internal:streamGenerateContent")).and(header("authorization", "Bearer G2")).respond_with(gemini_stream("OK")).mount(&up).await;
    let (srv, admin) = setup(&up).await;
    login(&admin, "antigravity_oauth", json!({"project_id": "p"}), 600).await.unwrap(); // 還有 10 分鐘：Claude 不會續期，Google 要

    assert_eq!(test_call(&admin, "antigravity_oauth").await["ok"], true);
    assert_eq!(srv.state().secrets.get("ai_oauth_access").as_deref(), Some("G2"));
    assert_eq!(srv.state().secrets.get("ai_oauth_refresh").as_deref(), Some("REFRESH"), "Google 不輪替，續期憑證原樣保留");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_lists_come_from_each_subscription_and_fall_back_to_a_builtin_list() {
    let up = MockServer::start().await;
    Mock::given(method("GET")).and(path("/backend-api/codex/models")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [{"slug": "gpt-b"}, {"slug": "gpt-a"}]}))).mount(&up).await;
    Mock::given(method("POST")).and(path("/v1internal:fetchAvailableModels")).respond_with(ResponseTemplate::new(500)).mount(&up).await;
    let (_srv, admin) = setup(&up).await;
    login(&admin, "codex_oauth", json!({}), 3600).await.unwrap();
    let models: Vec<String> = admin.typed(Method::POST, "/v1/ai/models", Some(json!({"provider": "codex_oauth"}))).await.unwrap();
    assert_eq!(models, vec!["gpt-a", "gpt-b"]);
    login(&admin, "antigravity_oauth", json!({"project_id": "p"}), 3600).await.unwrap();
    let models: Vec<String> = admin.typed(Method::POST, "/v1/ai/models", Some(json!({"provider": "antigravity_oauth"}))).await.unwrap();
    assert_eq!(models, antigravity::FALLBACK_MODELS);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn switching_provider_never_sends_one_vendors_credentials_to_another() {
    let up = MockServer::start().await;
    Mock::given(method("POST")).and(path("/backend-api/codex/responses")).respond_with(codex_stream("OK")).mount(&up).await;
    let (srv, admin) = setup(&up).await;
    login(&admin, "antigravity_oauth", json!({"project_id": "p"}), 3600).await.unwrap();
    // 只改供應商、沒有為 codex 登入。
    admin.request(Method::PUT, "/v1/ai", Some(json!({"provider": "codex_oauth", "model": "m-1", "base_url": ""}))).await.unwrap();

    let view: Value = admin.get("/v1/ai").await.unwrap();
    assert_eq!((view["configured"].as_bool(), view["login"].as_str()), (Some(false), Some("none")), "別家的憑證不算已登入");
    let r = test_call(&admin, "codex_oauth").await;
    assert_eq!(r["ok"], false);
    assert!(r["message"].as_str().unwrap().contains("登入"), "{r}");
    assert!(up.received_requests().await.unwrap().iter().all(|q| q.url.path() != "/backend-api/codex/responses"), "不能拿 Google 的憑證去打 OpenAI");

    // 切回去原本登入的那家就又能用，且上一家的附加資料不會殘留。
    admin.request(Method::PUT, "/v1/ai", Some(json!({"provider": "antigravity_oauth", "model": "m-1", "base_url": ""}))).await.unwrap();
    assert_eq!(admin.get::<Value>("/v1/ai").await.unwrap()["login"], "ok");
    login(&admin, "codex_oauth", json!({"account_id": "a"}), 3600).await.unwrap();
    assert!(srv.state().secrets.get("ai_oauth_project_id").is_none(), "換成 Codex 登入後，Gemini 的專案 id 要清掉");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn logout_removes_the_extra_login_data_too() {
    let up = MockServer::start().await;
    let (srv, admin) = setup(&up).await;
    login(&admin, "antigravity_oauth", json!({"project_id": "p"}), 3600).await.unwrap();
    admin.request(Method::DELETE, "/v1/ai/oauth", None).await.unwrap();
    for id in ["ai_oauth_access", "ai_oauth_refresh", "ai_oauth_expires_at", "ai_oauth_kind", "ai_oauth_project_id", "ai_oauth_account_id"] {
        assert!(srv.state().secrets.get(id).is_none(), "{id}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn logging_in_to_a_different_provider_clears_the_other_vendors_model_but_relogin_keeps_it() {
    let up = MockServer::start().await;
    let (_srv, admin) = setup(&up).await;
    login(&admin, "antigravity_oauth", json!({"project_id": "p"}), 3600).await.unwrap(); // 模型 m-1
    // 同一家重新登入：模型保留。
    admin.request(Method::PUT, "/v1/ai/oauth", Some(json!({"provider": "antigravity_oauth", "access": "A2", "refresh": "R2", "project_id": "p"}))).await.unwrap();
    assert_eq!(admin.get::<Value>("/v1/ai").await.unwrap()["settings"]["model"], "m-1");
    // 換成 Codex 登入：上一家的模型名稱對 OpenAI 沒有意義，要清掉，不能讓畫面顯示「已設定」卻其實會 404。
    admin.request(Method::PUT, "/v1/ai/oauth", Some(json!({"provider": "codex_oauth", "access": "A", "refresh": "R"}))).await.unwrap();
    let view: Value = admin.get("/v1/ai").await.unwrap();
    assert_eq!((view["settings"]["provider"].as_str(), view["settings"]["model"].as_str(), view["configured"].as_bool()), (Some("codex_oauth"), Some(""), Some(false)), "{view}");
}
