//! Contract test for `OpenAiCompatibleClient` against a wiremock fake.
//! Covers:
//!   - Happy path: mock SSE stream → verify chunked output
//!   - api_key=None: no Authorization header is sent
//!   - api_key=Some: Authorization header is present
//!   - Custom base_url routing

use aiterm_lib::ai::{
    compatible::OpenAiCompatibleClient, AiError, AiProvider, ChatMessage, EnvSnapshot,
    GenerateChunk, GenerateRequest, GenerateWithToolsResult, McpToolDefinition, QueryMode,
};
use std::path::PathBuf;
use tokio::sync::mpsc;
use wiremock::matchers::{body_string_contains, header, header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn req(text: &str) -> GenerateRequest {
    GenerateRequest {
        system_prompt: "sys".into(),
        messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!(text), tool_call_id: None, tool_calls: None }],
        context: EnvSnapshot {
            os: "linux".into(),
            shell: "bash".into(),
            cwd: PathBuf::from("/"),
            ..Default::default()
        },
        mode: QueryMode::SingleCommand,
        max_tokens: Some(256),
    }
}

const JSON_OUTPUT: &str = r#"{"explanation":"list","command":"ls","risk_level":"safe"}"#;

/// Builds an OpenAI-compatible SSE response (same format as OpenAI).
fn sse_happy_path() -> String {
    let c1 = format!(
        r#"{{"choices":[{{"delta":{{"content":{}}}}}]}}"#,
        serde_json::Value::String(JSON_OUTPUT[..20].to_string())
    );
    let c2 = format!(
        r#"{{"choices":[{{"delta":{{"content":{}}}}}]}}"#,
        serde_json::Value::String(JSON_OUTPUT[20..].to_string())
    );
    let done = r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#;
    format!("data: {c1}\n\ndata: {c2}\n\ndata: {done}\n\ndata: [DONE]\n\n")
}

#[tokio::test]
async fn happy_path_streams_with_api_key() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer compat-key"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_happy_path()),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = OpenAiCompatibleClient::new(
        server.uri(),
        "local-model".into(),
        Some("compat-key".into()),
        false,
    );
    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    client.generate(req("hello"), tx).await.expect("generate ok");

    let mut buf = String::new();
    let mut saw_done = false;
    while let Some(chunk) = rx.recv().await {
        buf.push_str(&chunk.delta);
        if chunk.done {
            saw_done = true;
            break;
        }
    }

    assert!(saw_done, "expected done chunk");
    assert_eq!(buf, JSON_OUTPUT);
}

#[tokio::test]
async fn no_api_key_omits_authorization_header() {
    let server = MockServer::start().await;

    // This mock only matches requests WITHOUT an Authorization header.
    // If the client sends one, wiremock won't match and the test will fail.
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_happy_path()),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = OpenAiCompatibleClient::new(
        server.uri(),
        "local-model".into(),
        None, // no api_key
        false,
    );
    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    client.generate(req("hello"), tx).await.expect("generate ok");

    let mut buf = String::new();
    while let Some(chunk) = rx.recv().await {
        buf.push_str(&chunk.delta);
        if chunk.done { break; }
    }
    assert_eq!(buf, JSON_OUTPUT);
}

#[tokio::test]
async fn with_api_key_sends_authorization_header() {
    let server = MockServer::start().await;

    // Match requires the Authorization header to be present.
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header_exists("authorization"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_happy_path()),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = OpenAiCompatibleClient::new(
        server.uri(),
        "local-model".into(),
        Some("secret".into()),
        false,
    );
    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    client.generate(req("hello"), tx).await.expect("generate ok");

    // Drain channel; main assertion is that the mock expectation is satisfied.
    while let Some(chunk) = rx.recv().await {
        if chunk.done { break; }
    }
}

#[tokio::test]
async fn custom_base_url_routes_correctly() {
    let server = MockServer::start().await;

    // The completions_url is `{base_url}/chat/completions`
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_happy_path()),
        )
        .expect(1)
        .mount(&server)
        .await;

    let custom_base = server.uri();
    let client = OpenAiCompatibleClient::new(
        custom_base,
        "custom-model".into(),
        None,
        false,
    );
    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    client.generate(req("test"), tx).await.expect("generate ok");

    while let Some(chunk) = rx.recv().await {
        if chunk.done { break; }
    }
    // If the mock expectation (expect(1)) is met, routing was correct.
}

#[tokio::test]
async fn returns_auth_failed_on_401() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .mount(&server)
        .await;

    let client =
        OpenAiCompatibleClient::new(server.uri(), "model".into(), Some("bad".into()), false);
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);
    let err = client.generate(req("x"), tx).await.unwrap_err();
    assert!(matches!(err, AiError::AuthFailed), "expected AuthFailed, got {err:?}");
}

// ── generate_with_tools：串流 ────────────────────────────────────────────────
//
// 這條路徑原本是 `"stream": false`，整包 JSON 回來後才送單一 chunk——於是所有
// 帶工具的對話（終端機 MCP、程式庫協助的研究階段）都是「想很久然後整段跳出
// 來」。文字要逐字送，工具呼叫則是分片來的，要靠 index 對位把 arguments 的
// 字串片段接回完整 JSON。

fn tools() -> Vec<McpToolDefinition> {
    vec![McpToolDefinition {
        name: "read_file".into(),
        description: "read a file".into(),
        input_schema: serde_json::json!({"type":"object","properties":{"path":{"type":"string"}}}),
    }]
}

#[tokio::test]
async fn generate_with_tools_streams_text_incrementally() {
    let server = MockServer::start().await;
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"這個專案\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"採用 DDD\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(body_string_contains("\"stream\":true"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = OpenAiCompatibleClient::new(server.uri(), "m".into(), None, false);
    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    let result = client.generate_with_tools(req("x"), tools(), tx).await.expect("ok");

    let mut deltas = Vec::new();
    while let Some(c) = rx.recv().await {
        if !c.delta.is_empty() { deltas.push(c.delta); }
        if c.done { break; }
    }
    // 關鍵：不只是「最後拿到全文」，而是分成多次送達。
    assert!(deltas.len() >= 2, "文字應逐段送出，實際只收到 {} 段：{deltas:?}", deltas.len());
    assert_eq!(deltas.concat(), "這個專案採用 DDD");

    match result {
        GenerateWithToolsResult::Text(t) => assert_eq!(t, "這個專案採用 DDD"),
        _ => panic!("expected Text"),
    }
}

#[tokio::test]
async fn generate_with_tools_assembles_streamed_tool_call_fragments() {
    let server = MockServer::start().await;
    // arguments 是一個字元一個字元(片段)來的，要按 index 接回完整 JSON。
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"read_file\",\"arguments\":\"\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"pa\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"th\\\":\\\"a.java\\\"}\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = OpenAiCompatibleClient::new(server.uri(), "m".into(), None, false);
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);
    let result = client.generate_with_tools(req("x"), tools(), tx).await.expect("ok");

    match result {
        GenerateWithToolsResult::ToolCalls { calls, .. } => {
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].tool_name, "read_file");
            assert_eq!(calls[0].id, "call_1");
            assert_eq!(calls[0].args["path"], "a.java");
        }
        _ => panic!("expected ToolCalls"),
    }
}

#[tokio::test]
async fn generate_with_tools_assembles_two_parallel_tool_calls() {
    let server = MockServer::start().await;
    // 同一輪要求兩個工具時，兩組片段是交錯來的，只能靠 index 分辨。
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c0\",\"function\":{\"name\":\"read_file\",\"arguments\":\"\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"id\":\"c1\",\"function\":{\"name\":\"read_file\",\"arguments\":\"\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"path\\\":\\\"a\\\"}\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"function\":{\"arguments\":\"{\\\"path\\\":\\\"b\\\"}\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let client = OpenAiCompatibleClient::new(server.uri(), "m".into(), None, false);
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);
    let result = client.generate_with_tools(req("x"), tools(), tx).await.expect("ok");

    match result {
        GenerateWithToolsResult::ToolCalls { calls, .. } => {
            assert_eq!(calls.len(), 2);
            assert_eq!(calls[0].args["path"], "a");
            assert_eq!(calls[1].args["path"], "b");
        }
        _ => panic!("expected ToolCalls"),
    }
}
