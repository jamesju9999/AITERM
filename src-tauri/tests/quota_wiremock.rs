//! 三個配額 adapter 對假伺服器的整合測試。
//!
//! 單元測試已覆蓋解析邏輯；這裡驗證的是 HTTP 層：狀態碼分流、header、
//! 逾時。回應主體一律取自探勘 dump 的真實形狀。

use aiterm_lib::ai::AiError;
use aiterm_lib::usage::quota::anthropic::AnthropicQuota;
use aiterm_lib::usage::quota::{codex::CodexQuota, copilot::CopilotQuota, QuotaSource};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const CODEX_BODY: &str = r#"{"plan_type":"free","rate_limit":{
    "primary_window":{"used_percent":12,"limit_window_seconds":18000,"reset_at":1789029443},
    "secondary_window":null}}"#;

#[tokio::test]
async fn codex_quota_sends_auth_and_account_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .and(header("authorization", "Bearer tok"))
        .and(header("chatgpt-account-id", "acc-1"))
        .and(header("originator", "codex_cli_rs"))
        .respond_with(ResponseTemplate::new(200).set_body_string(CODEX_BODY))
        .mount(&server)
        .await;

    let q = CodexQuota::new("GPT5.6".into(), "tok".into(), Some("acc-1".into()))
        .with_url(server.uri());
    let quota = q.fetch().await.expect("fetch");
    assert_eq!(quota.windows.len(), 1);
    assert_eq!(quota.windows[0].label, "5h");
}

#[tokio::test]
async fn unauthorized_maps_to_auth_failed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401).set_body_string("{}"))
        .mount(&server)
        .await;

    let q = CodexQuota::new("p".into(), "tok".into(), None).with_url(server.uri());
    let err = q.fetch().await.expect_err("應該失敗");
    assert!(matches!(err, AiError::AuthFailed), "得到 {err:?}");
}

#[tokio::test]
async fn not_found_is_an_error_not_a_panic() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .mount(&server)
        .await;

    let q = CodexQuota::new("p".into(), "tok".into(), None).with_url(server.uri());
    assert!(q.fetch().await.is_err());
}

#[tokio::test]
async fn html_body_on_200_is_a_parse_error_not_a_panic() {
    // Cloudflare 擋下來時會回 200 + HTML。必須降級成錯誤，不能 panic。
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>challenge</html>"))
        .mount(&server)
        .await;

    let q = CodexQuota::new("p".into(), "tok".into(), None).with_url(server.uri());
    assert!(q.fetch().await.is_err());
}

#[tokio::test]
async fn timeout_maps_to_network_error() {
    // 逾時用可注入的 100ms，不要讓 CI 每次真的等 5 秒。
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(CODEX_BODY)
                .set_delay(std::time::Duration::from_secs(2)),
        )
        .mount(&server)
        .await;

    let q = CodexQuota::new("p".into(), "tok".into(), None)
        .with_url(server.uri())
        .with_timeout(std::time::Duration::from_millis(100));
    let err = q.fetch().await.expect_err("應該逾時");
    assert!(matches!(err, AiError::Network { .. }), "得到 {err:?}");
}

#[tokio::test]
async fn copilot_uses_token_scheme_not_bearer() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header("authorization", "token ghp_x"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"access_type_sku":"x","quota_snapshots":{"premium_interactions":
               {"unlimited":false,"percent_remaining":47.5,"entitlement":300,"remaining":142}}}"#,
        ))
        .mount(&server)
        .await;

    let q = CopilotQuota::new("gh".into(), "ghp_x".into()).with_url(server.uri());
    let quota = q.fetch().await.expect("fetch");
    assert!((quota.windows[0].used_percent - 52.5).abs() < 1e-9);
}

#[tokio::test]
async fn anthropic_quota_hits_oauth_usage_path() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/oauth/usage"))
        .and(header("x-app", "cli"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"five_hour":{"utilization":7.0,"resets_at":null},
                "seven_day":null,"limits":[]}"#,
        ))
        .mount(&server)
        .await;

    let q = AnthropicQuota::new("anthropic-pro".into(), "tok".into(), server.uri());
    let quota = q.fetch().await.expect("fetch");
    assert!((quota.windows[0].used_percent - 7.0).abs() < 1e-9);
}
