//! 直接對 axum router 發請求，驗證授權與錯誤路徑。
//! 不啟動真的 TcpListener，用 tower 的 oneshot。

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use aiterm_lib::bridge::server::{router, AppState};
use aiterm_lib::bridge::tool_meta::ToolMetaCache;
use aiterm_lib::config::ConfigStore;
use aiterm_lib::secret::SecretStore;

fn state(dir: &tempfile::TempDir) -> AppState {
    AppState {
        config: Arc::new(ConfigStore::new_at(dir.path().join("config.toml"))),
        secrets: Arc::new(SecretStore::new()),
        token: Arc::new("t0ken".into()),
        tool_meta: Arc::new(ToolMetaCache::new(512)),
    }
}

fn post(uri: &str, token: Option<&str>, body: serde_json::Value) -> Request<Body> {
    let mut b = Request::builder().method("POST").uri(uri).header("content-type", "application/json");
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    b.body(Body::from(body.to_string())).unwrap()
}

#[tokio::test]
async fn rejects_missing_token() {
    let dir = tempfile::tempdir().unwrap();
    let resp = router(state(&dir))
        .oneshot(post("/v1/messages", None, serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn rejects_wrong_token() {
    let dir = tempfile::tempdir().unwrap();
    let resp = router(state(&dir))
        .oneshot(post("/v1/messages", Some("nope"), serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn accepts_non_streaming_request_and_fails_only_on_unmapped_tier() {
    // 曾經：`stream != true` 在這一步就被直接拒絕（400 +「只支援串流」）。
    // 非串流路徑補上之後，這個請求應該走到下一關（層級映射），而不是被
    // stream 欄位擋下——用「錯誤訊息指向設定頁」而非「隨便一個 400」，
    // 證明擋下它的是這個乾淨的 config 沒映射 sonnet 層，不是 stream:false。
    let dir = tempfile::tempdir().unwrap();
    let resp = router(state(&dir))
        .oneshot(post(
            "/v1/messages",
            Some("t0ken"),
            serde_json::json!({"model": "aiterm:sonnet", "messages": [], "stream": false}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("設定"), "訊息要指向設定頁（未映射層級）：{text}");
    assert!(!text.contains("只支援串流"), "非串流請求不該再被直接拒絕：{text}");
}

#[tokio::test]
async fn unmapped_tier_returns_400_pointing_at_settings() {
    let dir = tempfile::tempdir().unwrap();
    let resp = router(state(&dir))
        .oneshot(post(
            "/v1/messages",
            Some("t0ken"),
            serde_json::json!({"model": "aiterm:sonnet", "messages": [], "stream": true}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("設定"), "訊息要指向設定頁：{text}");
}

#[tokio::test]
async fn count_tokens_returns_an_estimate() {
    let dir = tempfile::tempdir().unwrap();
    let resp = router(state(&dir))
        .oneshot(post(
            "/v1/messages/count_tokens",
            Some("t0ken"),
            serde_json::json!({"model": "m", "messages": [{"role":"user","content":"12345678"}]}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["input_tokens"], 2);
}
