use wiremock::matchers::{header, headers, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aiterm_lib::bridge::upstream::anthropic::AnthropicUpstream;
use aiterm_lib::bridge::upstream::UpstreamResponse;

#[tokio::test]
async fn api_key_mode_sets_x_api_key_and_passes_body_through() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "sk-ant-test"))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw("event: message_stop\ndata: {}\n\n", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let up = AnthropicUpstream::new(server.uri(), "sk-ant-test".into(), false);
    let raw = serde_json::json!({
        "model": "aiterm:sonnet", "stream": true, "messages": [],
        // 我們沒解析的欄位必須原樣送達。
        "metadata": {"user_id": "x"}
    });
    let resp = up.send_raw(&raw, "claude-sonnet-4-5").await.unwrap();
    match resp {
        UpstreamResponse::Passthrough(r) => {
            let body = r.text().await.unwrap();
            assert!(body.contains("message_stop"));
        }
        _ => panic!("Anthropic 路徑必須回 Passthrough"),
    }
}

#[tokio::test]
async fn oauth_mode_sets_bearer_and_beta_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("authorization", "Bearer sk-ant-oat-x"))
        // wiremock 的 `header()` 比對時，實際請求裡逗號分隔的值會被拆成多個
        // element 再比對，但期望值不會被拆；單一 `header()` 因此永遠比對不到
        // 這個逗號分隔的值，改用 `headers()` 帶拆好的兩個 element。
        .and(headers("anthropic-beta", vec!["claude-code-20250219", "oauth-2025-04-20"]))
        .and(header("x-app", "cli"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("data: {}\n\n", "text/event-stream"))
        .mount(&server)
        .await;

    let up = AnthropicUpstream::new(server.uri(), "sk-ant-oat-x".into(), true);
    let resp = up.send_raw(&serde_json::json!({"model": "m", "messages": []}), "m").await;
    assert!(resp.is_ok());
}
