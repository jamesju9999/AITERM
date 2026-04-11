//! Contract test for `OpenAiClient` against a wiremock fake of the OpenAI
//! chat completions endpoint. Covers the happy path, 401, 429 with
//! retry-after, and 500.

use aiterm_lib::ai::{
    openai::OpenAiClient, AiError, AiProvider, ChatMessage, EnvSnapshot, GenerateChunk,
    GenerateRequest, QueryMode,
};
use std::path::PathBuf;
use tokio::sync::mpsc;
use wiremock::matchers::{bearer_token, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn req(text: &str) -> GenerateRequest {
    GenerateRequest {
        system_prompt: "sys".into(),
        messages: vec![ChatMessage { role: "user".into(), content: text.into() }],
        context: EnvSnapshot {
            os: "linux".into(),
            shell: "bash".into(),
            cwd: PathBuf::from("/"),
        },
        mode: QueryMode::SingleCommand,
        max_tokens: Some(256),
    }
}

const JSON_OUTPUT: &str =
    r#"{"explanation":"list","command":"ls","risk_level":"safe"}"#;

fn sse_response_happy_path() -> String {
    // Three SSE events: two with content, one terminator.
    let c1 = format!(
        r#"{{"choices":[{{"delta":{{"content":{}}}}}]}}"#,
        serde_json::Value::String(JSON_OUTPUT[..20].to_string())
    );
    let c2 = format!(
        r#"{{"choices":[{{"delta":{{"content":{}}}}}]}}"#,
        serde_json::Value::String(JSON_OUTPUT[20..].to_string())
    );
    let done = r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#;
    format!(
        "data: {c1}\n\ndata: {c2}\n\ndata: {done}\n\ndata: [DONE]\n\n"
    )
}

#[tokio::test]
async fn happy_path_streams_and_parses() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(bearer_token("test-key"))
        .and(header("content-type", "application/json"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_response_happy_path()),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = OpenAiClient::with_base_url("test-key".into(), server.uri());
    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);

    client.generate(req("hello"), tx).await.expect("generate ok");

    let mut buf = String::new();
    let mut saw_done = false;
    while let Some(chunk) = rx.recv().await {
        buf.push_str(&chunk.delta);
        if chunk.done { saw_done = true; break; }
    }
    assert!(saw_done, "expected a done chunk");
    assert_eq!(buf, JSON_OUTPUT);
}

#[tokio::test]
async fn returns_auth_failed_on_401() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .mount(&server)
        .await;

    let client = OpenAiClient::with_base_url("bad".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);
    let err = client.generate(req("x"), tx).await.unwrap_err();
    assert!(matches!(err, AiError::AuthFailed), "got {err:?}");
}

#[tokio::test]
async fn returns_rate_limit_with_retry_after() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "30")
                .set_body_string("slow down"),
        )
        .mount(&server)
        .await;

    let client = OpenAiClient::with_base_url("k".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);
    let err = client.generate(req("x"), tx).await.unwrap_err();
    match err {
        AiError::RateLimit { retry_after } => {
            assert_eq!(retry_after.as_deref(), Some("30"));
        }
        other => panic!("expected RateLimit, got {other:?}"),
    }
}

#[tokio::test]
async fn returns_network_on_500() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal"))
        .mount(&server)
        .await;

    let client = OpenAiClient::with_base_url("k".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);
    let err = client.generate(req("x"), tx).await.unwrap_err();
    match err {
        AiError::Network { message } => assert!(message.contains("500")),
        other => panic!("expected Network, got {other:?}"),
    }
}
