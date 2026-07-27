//! Contract test for `CodexClient` against a wiremock fake of the ChatGPT
//! Codex Responses API. Covers the happy path (SSE streaming + required
//! client-identity headers) and 401 → AuthFailed.

use aiterm_lib::ai::{
    codex::CodexClient, AiError, AiProvider, ChatMessage, EnvSnapshot, GenerateChunk,
    GenerateRequest, QueryMode,
};
use std::path::PathBuf;
use tokio::sync::mpsc;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn req() -> GenerateRequest {
    GenerateRequest {
        system_prompt: "You are a terminal assistant.".into(),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: serde_json::json!("list files"),
            tool_call_id: None,
            tool_calls: None,
        }],
        context: EnvSnapshot {
            os: "linux".into(),
            shell: "bash".into(),
            cwd: PathBuf::from("/"),
            ..Default::default()
        },
        mode: QueryMode::Chat,
        max_tokens: Some(256),
    }
}

#[tokio::test]
async fn happy_path_streams_and_sends_required_headers() {
    let server = MockServer::start().await;

    let sse_body = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n\
                     data: {\"type\":\"response.output_text.delta\",\"delta\":\" world\"}\n\n\
                     data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":2}}}\n\n";

    Mock::given(method("POST"))
        .and(path("/backend-api/codex/responses"))
        .and(header("authorization", "Bearer test-token"))
        .and(header("originator", "codex_cli_rs"))
        .and(header("chatgpt-account-id", "acct-123"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body),
        )
        .expect(1)
        .mount(&server)
        .await;

    // Production always talks to the real chatgpt.com host — `with_base_url`
    // is a test-only hook to point at the wiremock server.
    let client = CodexClient::with_base_url(
        "test-token".into(),
        "gpt-5.1-codex".into(),
        Some("acct-123".into()),
        server.uri(),
    );
    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    client.generate(req(), tx).await.expect("generate ok");

    let mut buf = String::new();
    let mut saw_done = false;
    while let Some(chunk) = rx.recv().await {
        buf.push_str(&chunk.delta);
        if chunk.done {
            saw_done = true;
            break;
        }
    }
    assert!(saw_done, "expected a done chunk");
    assert_eq!(buf, "Hello world");
}

#[tokio::test]
async fn returns_auth_failed_on_401() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .mount(&server)
        .await;

    let client =
        CodexClient::with_base_url("bad-token".into(), "gpt-5.1-codex".into(), None, server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);
    let err = client.generate(req(), tx).await.unwrap_err();
    assert!(matches!(err, AiError::AuthFailed), "expected AuthFailed, got {err:?}");
}

#[tokio::test]
async fn returns_model_error_on_response_failed_event() {
    let server = MockServer::start().await;

    let sse_body =
        "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"message\":\"something broke\"}}}\n\n";

    Mock::given(method("POST"))
        .and(path("/backend-api/codex/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body),
        )
        .mount(&server)
        .await;

    let client =
        CodexClient::with_base_url("test-token".into(), "gpt-5.1-codex".into(), None, server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);
    let err = client.generate(req(), tx).await.unwrap_err();
    assert!(matches!(err, AiError::ModelError { .. }), "expected ModelError, got {err:?}");
}
