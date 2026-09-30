//! OpenAI 相容 server 的整合測試：真的 TCP listener、真的 HTTP、
//! wiremock 扮演上游。CORS 走 tower oneshot（不綁 0.0.0.0，避免 macOS 防火牆對話框）。

use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aiterm_lib::bridge::openai_server::server::{router, AppState};
use aiterm_lib::bridge::openai_server::OpenAiServerState;
use aiterm_lib::bridge::tool_meta::ToolMetaCache;
use aiterm_lib::config::types::{ModelAlias, ProviderConfig, ProviderType};
use aiterm_lib::config::ConfigStore;
use aiterm_lib::secret::SecretStore;

const KEY: &str = "sk-aiterm-test";

fn provider(id: &str, ty: ProviderType, base: &str) -> ProviderConfig {
    ProviderConfig {
        id: id.into(),
        display_name: id.into(),
        provider_type: ty,
        base_url: Some(base.to_string()),
        oauth_client_id: None,
        model: "m".into(),
        supports_json_mode: true,
        auth_method: None,
    }
}

fn alias(a: &str, p: &str, m: &str) -> ModelAlias {
    ModelAlias { alias: a.into(), provider_id: p.into(), model: m.into() }
}

fn config(dir: &tempfile::TempDir, upstream: &str) -> Arc<ConfigStore> {
    let config = ConfigStore::new_at(dir.path().join("config.toml"));
    config
        .update(|c| {
            c.providers.push(provider("wm-openai", ProviderType::Openai, upstream));
            c.providers.push(provider("wm-anthropic", ProviderType::AnthropicCompatible, upstream));
            c.openai_server.aliases = vec![
                alias("gpt-4o", "wm-openai", "gpt-4o-mini"),
                alias("claude-x", "wm-anthropic", "claude-real"),
            ];
        })
        .unwrap();
    Arc::new(config)
}

async fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let p = l.local_addr().unwrap().port();
    drop(l);
    p
}

async fn start(dir: &tempfile::TempDir, upstream: &str) -> (OpenAiServerState, String) {
    let server = OpenAiServerState::new();
    let port = free_port().await;
    server
        .start(config(dir, upstream), Arc::new(SecretStore::new()), KEY.into(), port, false)
        .await
        .unwrap();
    (server, format!("http://127.0.0.1:{port}"))
}

async fn chat(base: &str, key: Option<&str>, body: &Value) -> reqwest::Response {
    let mut r = reqwest::Client::new().post(format!("{base}/v1/chat/completions")).json(body);
    if let Some(k) = key {
        r = r.bearer_auth(k);
    }
    r.send().await.unwrap()
}

fn user_request(model: &str, stream: bool) -> Value {
    json!({"model": model, "stream": stream, "messages": [{"role": "user", "content": "hi"}]})
}

/// 解析串流 body：`data: {...}` 逐幀成 JSON，`[DONE]` 以字串表示。
fn frames(body: &str) -> Vec<Value> {
    body.split("\n\n")
        .filter_map(|f| f.trim().strip_prefix("data: "))
        .map(|p| if p == "[DONE]" { json!("[DONE]") } else { serde_json::from_str(p).unwrap() })
        .collect()
}

const OPENAI_TEXT_SSE: &str = concat!(
    "data: {\"choices\":[{\"delta\":{\"content\":\"你\"}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{\"content\":\"好\"}}]}\n\n",
    "data: {\"choices\":[{\"finish_reason\":\"stop\"}]}\n\n",
    "data: [DONE]\n\n",
);

async fn mount_openai(server: &MockServer, sse: &'static str) {
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
        .mount(server)
        .await;
}

// ── 授權與清單 ──────────────────────────────────────────────────────────

#[tokio::test]
async fn rejects_missing_and_wrong_key_but_health_is_open() {
    let wm = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (server, base) = start(&dir, &wm.uri()).await;

    for key in [None, Some("nope")] {
        let resp = chat(&base, key, &user_request("gpt-4o", false)).await;
        assert_eq!(resp.status(), 401);
        let body: Value = resp.json().await.unwrap();
        assert_eq!(body["error"]["code"], "invalid_api_key");
    }
    let models = reqwest::get(format!("{base}/v1/models")).await.unwrap();
    assert_eq!(models.status(), 401);
    let health = reqwest::get(format!("{base}/health")).await.unwrap();
    assert_eq!(health.status(), 200);
    server.stop();
}

#[tokio::test]
async fn models_lists_only_the_configured_aliases() {
    let wm = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (server, base) = start(&dir, &wm.uri()).await;

    let body: Value = reqwest::Client::new()
        .get(format!("{base}/v1/models"))
        .bearer_auth(KEY)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ids: Vec<_> = body["data"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["gpt-4o", "claude-x"]);
    assert_eq!(body["object"], "list");
    server.stop();
}

#[tokio::test]
async fn unknown_model_is_a_404_model_not_found() {
    let wm = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (server, base) = start(&dir, &wm.uri()).await;

    let resp = chat(&base, Some(KEY), &user_request("gpt-5", false)).await;
    assert_eq!(resp.status(), 404);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["code"], "model_not_found");
    assert!(body["error"]["message"].as_str().unwrap().contains("gpt-5"));
    server.stop();
}

#[tokio::test]
async fn malformed_requests_are_400s() {
    let wm = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (server, base) = start(&dir, &wm.uri()).await;

    let not_json = reqwest::Client::new()
        .post(format!("{base}/v1/chat/completions"))
        .bearer_auth(KEY)
        .body("{oops")
        .send()
        .await
        .unwrap();
    assert_eq!(not_json.status(), 400);

    let empty = chat(&base, Some(KEY), &json!({"model": "gpt-4o", "messages": []})).await;
    assert_eq!(empty.status(), 400);
    server.stop();
}

// ── OpenAI 家族上游 ────────────────────────────────────────────────────

#[tokio::test]
async fn openai_upstream_non_streaming_returns_one_completion() {
    let wm = MockServer::start().await;
    mount_openai(&wm, OPENAI_TEXT_SSE).await;
    let dir = tempfile::tempdir().unwrap();
    let (server, base) = start(&dir, &wm.uri()).await;

    let resp = chat(&base, Some(KEY), &user_request("gpt-4o", false)).await;
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["object"], "chat.completion");
    assert_eq!(body["model"], "gpt-4o", "回給客戶端的 model 是它送的別名，不是內部型號");
    assert_eq!(body["choices"][0]["message"]["content"], "你好");
    assert_eq!(body["choices"][0]["finish_reason"], "stop");

    // 上游收到的是映射後的真實型號。
    let sent: Value = wm.received_requests().await.unwrap()[0].body_json().unwrap();
    assert_eq!(sent["model"], "gpt-4o-mini");
    server.stop();
}

#[tokio::test]
async fn openai_upstream_streaming_emits_chunks_and_done() {
    let wm = MockServer::start().await;
    mount_openai(&wm, OPENAI_TEXT_SSE).await;
    let dir = tempfile::tempdir().unwrap();
    let (server, base) = start(&dir, &wm.uri()).await;

    let resp = chat(&base, Some(KEY), &user_request("gpt-4o", true)).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-type"], "text/event-stream");
    let f = frames(&resp.text().await.unwrap());

    assert_eq!(f[0]["choices"][0]["delta"]["role"], "assistant");
    let text: String = f
        .iter()
        .filter_map(|c| c["choices"][0]["delta"]["content"].as_str())
        .collect();
    assert_eq!(text, "你好");
    assert_eq!(f[f.len() - 2]["choices"][0]["finish_reason"], "stop");
    assert_eq!(f.last().unwrap(), &json!("[DONE]"));
    assert_eq!(f.iter().filter(|x| **x == json!("[DONE]")).count(), 1);
    server.stop();
}

#[tokio::test]
async fn openai_upstream_streaming_tool_call_round_trip() {
    let wm = MockServer::start().await;
    mount_openai(
        &wm,
        concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",",
            "\"function\":{\"name\":\"Bash\",\"arguments\":\"{\\\"command\\\":\\\"ls\\\"}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n",
        ),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let (server, base) = start(&dir, &wm.uri()).await;

    let mut req = user_request("gpt-4o", true);
    req["tools"] = json!([{"type": "function", "function": {
        "name": "Bash", "parameters": {"type": "object", "properties": {"command": {"type": "string"}}}}}]);
    let f = frames(&chat(&base, Some(KEY), &req).await.text().await.unwrap());

    let calls: Vec<&Value> = f
        .iter()
        .filter_map(|c| c["choices"][0]["delta"]["tool_calls"].get(0))
        .collect();
    assert_eq!(calls[0]["id"], "call_1");
    assert_eq!(calls[0]["function"]["name"], "Bash");
    let args: String = calls.iter().filter_map(|c| c["function"]["arguments"].as_str()).collect();
    assert_eq!(serde_json::from_str::<Value>(&args).unwrap(), json!({"command": "ls"}));
    assert!(f.iter().any(|c| c["choices"][0]["finish_reason"] == "tool_calls"));

    // 上游確實收到了翻譯後的工具定義。
    let sent: Value = wm.received_requests().await.unwrap()[0].body_json().unwrap();
    assert_eq!(sent["tools"][0]["function"]["name"], "Bash");
    server.stop();
}

#[tokio::test]
async fn tool_result_turn_reaches_the_upstream_as_a_tool_message() {
    let wm = MockServer::start().await;
    mount_openai(&wm, "data: [DONE]\n\n").await;
    let dir = tempfile::tempdir().unwrap();
    let (server, base) = start(&dir, &wm.uri()).await;

    let req = json!({"model": "gpt-4o", "messages": [
        {"role": "user", "content": "列目錄"},
        {"role": "assistant", "content": null, "tool_calls": [
            {"id": "call_1", "type": "function", "function": {"name": "Bash", "arguments": "{\"command\":\"ls\"}"}}]},
        {"role": "tool", "tool_call_id": "call_1", "content": "a.txt\nb.txt"}
    ]});
    assert_eq!(chat(&base, Some(KEY), &req).await.status(), 200);

    let sent: Value = wm.received_requests().await.unwrap()[0].body_json().unwrap();
    let msgs = sent["messages"].as_array().unwrap();
    let tool = msgs.iter().find(|m| m["role"] == "tool").expect("上游要收到 tool 訊息");
    assert_eq!(tool["tool_call_id"], "call_1");
    assert_eq!(tool["content"], "a.txt\nb.txt");
    server.stop();
}

// ── Anthropic 家族上游 ────────────────────────────────────────────────

const ANTHROPIC_TOOL_SSE: &str = concat!(
    "event: message_start\n",
    "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":9}}}\n\n",
    "event: content_block_start\n",
    "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
    "event: content_block_delta\n",
    "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"先列目錄\"}}\n\n",
    "event: content_block_stop\n",
    "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
    "event: content_block_start\n",
    "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"Bash\",\"input\":{}}}\n\n",
    "event: content_block_delta\n",
    "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"command\\\":\"}}\n\n",
    "event: content_block_delta\n",
    "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"ls\\\"}\"}}\n\n",
    "event: content_block_stop\n",
    "data: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
    "event: message_delta\n",
    "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":5}}\n\n",
    "event: message_stop\n",
    "data: {\"type\":\"message_stop\"}\n\n",
);

async fn mount_anthropic(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(ANTHROPIC_TOOL_SSE, "text/event-stream"))
        .mount(server)
        .await;
}

#[tokio::test]
async fn anthropic_upstream_streaming_text_and_tool_call() {
    let wm = MockServer::start().await;
    mount_anthropic(&wm).await;
    let dir = tempfile::tempdir().unwrap();
    let (server, base) = start(&dir, &wm.uri()).await;

    let f = frames(&chat(&base, Some(KEY), &user_request("claude-x", true)).await.text().await.unwrap());
    let text: String = f.iter().filter_map(|c| c["choices"][0]["delta"]["content"].as_str()).collect();
    assert_eq!(text, "先列目錄");
    let calls: Vec<&Value> = f.iter().filter_map(|c| c["choices"][0]["delta"]["tool_calls"].get(0)).collect();
    assert_eq!(calls[0]["id"], "toolu_1");
    let args: String = calls.iter().filter_map(|c| c["function"]["arguments"].as_str()).collect();
    assert_eq!(serde_json::from_str::<Value>(&args).unwrap(), json!({"command": "ls"}));
    assert!(f.iter().any(|c| c["choices"][0]["finish_reason"] == "tool_calls"));
    assert_eq!(f.last().unwrap(), &json!("[DONE]"));
    server.stop();
}

#[tokio::test]
async fn anthropic_upstream_is_always_called_with_stream_true_even_for_non_streaming_clients() {
    let wm = MockServer::start().await;
    mount_anthropic(&wm).await;
    let dir = tempfile::tempdir().unwrap();
    let (server, base) = start(&dir, &wm.uri()).await;

    let resp = chat(&base, Some(KEY), &user_request("claude-x", false)).await;
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let m = &body["choices"][0]["message"];
    assert_eq!(m["content"], "先列目錄");
    assert_eq!(m["tool_calls"][0]["function"]["arguments"], "{\"command\":\"ls\"}");
    assert_eq!(body["usage"], json!({"prompt_tokens": 9, "completion_tokens": 5, "total_tokens": 14}));

    let sent: Value = wm.received_requests().await.unwrap()[0].body_json().unwrap();
    assert_eq!(sent["stream"], true);
    assert_eq!(sent["model"], "claude-real");
    server.stop();
}

// ── 上游錯誤 ────────────────────────────────────────────────────────────

#[tokio::test]
async fn upstream_usage_limit_surfaces_as_429_with_plan_details() {
    let wm = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({"error": {
            "type": "usage_limit_reached", "message": "The usage limit has been reached",
            "plan_type": "free", "resets_in_seconds": 1460795}})))
        .mount(&wm)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let (server, base) = start(&dir, &wm.uri()).await;

    let resp = chat(&base, Some(KEY), &user_request("gpt-4o", false)).await;
    assert_eq!(resp.status(), 429);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["code"], "rate_limit_exceeded");
    let msg = body["error"]["message"].as_str().unwrap();
    assert!(msg.contains("plan: free") && msg.contains("16d"), "{msg}");
    server.stop();
}

#[tokio::test]
async fn upstream_401_is_reported_as_an_upstream_auth_error_not_a_client_key_error() {
    let wm = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
        .mount(&wm)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let (server, base) = start(&dir, &wm.uri()).await;

    for stream in [false, true] {
        let resp = chat(&base, Some(KEY), &user_request("gpt-4o", stream)).await;
        assert_eq!(resp.status(), 401, "串流也要是真正的 HTTP 狀態碼，不是 200 + error frame");
        let body: Value = resp.json().await.unwrap();
        assert_eq!(body["error"]["code"], "upstream_auth_failed");
        assert!(body["error"]["message"].as_str().unwrap().contains("上游"));
    }
    server.stop();
}

// ── CORS ────────────────────────────────────────────────────────────────

fn oneshot_state(dir: &tempfile::TempDir) -> AppState {
    AppState {
        config: config(dir, "http://127.0.0.1:1"),
        secrets: Arc::new(SecretStore::new()),
        token: Arc::new(KEY.into()),
        tool_meta: Arc::new(ToolMetaCache::new(16)),
    }
}

fn preflight() -> Request<Body> {
    Request::builder()
        .method("OPTIONS")
        .uri("/v1/chat/completions")
        .header("origin", "http://evil.example")
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn loopback_mode_sends_no_cors_headers() {
    let dir = tempfile::tempdir().unwrap();
    let resp = router(oneshot_state(&dir), false).oneshot(preflight()).await.unwrap();
    assert_eq!(resp.status(), 204);
    assert!(resp.headers().get("access-control-allow-origin").is_none());
}

#[tokio::test]
async fn lan_mode_answers_preflight_with_cors_headers() {
    let dir = tempfile::tempdir().unwrap();
    let resp = router(oneshot_state(&dir), true).oneshot(preflight()).await.unwrap();
    assert_eq!(resp.status(), 204);
    assert_eq!(resp.headers()["access-control-allow-origin"], "*");
    assert!(resp.headers()["access-control-allow-headers"].to_str().unwrap().contains("authorization"));
}
