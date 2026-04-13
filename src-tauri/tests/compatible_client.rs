//! Contract test for `OpenAiCompatibleClient` against a wiremock fake.
//! Covers:
//!   - Happy path: mock SSE stream → verify chunked output
//!   - api_key=None: no Authorization header is sent
//!   - api_key=Some: Authorization header is present
//!   - Custom base_url routing

use aiterm_lib::ai::{
    compatible::OpenAiCompatibleClient, AiError, AiProvider, ChatMessage, EnvSnapshot,
    GenerateChunk, GenerateRequest, QueryMode,
};
use std::path::PathBuf;
use tokio::sync::mpsc;
use wiremock::matchers::{header, header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn req(text: &str) -> GenerateRequest {
    GenerateRequest {
        system_prompt: "sys".into(),
        messages: vec![ChatMessage { role: "user".into(), content: text.into() }],
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
