//! Contract test for `AntigravityClient` against a wiremock fake of the
//! Antigravity/Cloud Code Assist streamGenerateContent endpoint. Covers the
//! happy path (SSE streaming + required headers) and 401 → AuthFailed.

use aiterm_lib::ai::{
    antigravity::AntigravityClient, AiError, AiProvider, ChatMessage, EnvSnapshot, GenerateChunk,
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

    let sse_body = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}]}}]}\n\n\
                     data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\" world\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":2}}\n\n";

    Mock::given(method("POST"))
        .and(path("/v1internal:streamGenerateContent"))
        .and(header("authorization", "Bearer test-token"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body),
        )
        .expect(1)
        .mount(&server)
        .await;

    // Production always talks to the real cloudcode-pa.googleapis.com host —
    // `with_base_url` is a test-only hook to point at the wiremock server.
    let client = AntigravityClient::with_base_url(
        "test-token".into(),
        "proj-123".into(),
        "gemini-2.5-pro".into(),
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

    let client = AntigravityClient::with_base_url(
        "bad-token".into(),
        "proj-123".into(),
        "gemini-2.5-pro".into(),
        server.uri(),
    );
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);
    let err = client.generate(req(), tx).await.unwrap_err();
    assert!(matches!(err, AiError::AuthFailed), "expected AuthFailed, got {err:?}");
}

#[tokio::test]
async fn returns_model_error_on_blocked_prompt_feedback() {
    let server = MockServer::start().await;

    // Gemini's documented shape for a safety-blocked prompt: empty
    // `candidates` plus a `promptFeedback.blockReason` — not an HTTP error
    // and not a recognizably-different top-level SSE event type.
    let sse_body = "data: {\"candidates\":[],\"promptFeedback\":{\"blockReason\":\"SAFETY\"}}\n\n";

    Mock::given(method("POST"))
        .and(path("/v1internal:streamGenerateContent"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = AntigravityClient::with_base_url(
        "test-token".into(),
        "proj-123".into(),
        "gemini-2.5-pro".into(),
        server.uri(),
    );
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);
    let err = client.generate(req(), tx).await.unwrap_err();
    assert!(matches!(err, AiError::ModelError { .. }), "expected ModelError, got {err:?}");
}

/// `health_check` must not drain the SSE stream to validate a token+project
/// id pair — it should return as soon as the response status/headers arrive.
/// A wiremock `MockServer` sends its whole (buffered) body essentially
/// instantly, so it can't distinguish "waited for body" from "didn't" on
/// timing alone; this uses a raw TCP listener to send headers immediately
/// and then stall for several seconds before writing any body bytes, proving
/// `health_check` doesn't need the body to complete.
#[tokio::test]
async fn health_check_returns_without_waiting_for_response_body() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let _ = socket.read(&mut buf).await; // drain the request, content unused

        let headers = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
        socket.write_all(headers.as_bytes()).await.unwrap();
        socket.flush().await.unwrap();

        // Stall well past the test's timeout before sending any body bytes.
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        let _ = socket.write_all(b"data: {\"candidates\":[]}\n\n").await;
    });

    let client = AntigravityClient::with_base_url(
        "test-token".into(),
        "proj-123".into(),
        "gemini-2.5-pro".into(),
        format!("http://{addr}"),
    );

    let result =
        tokio::time::timeout(std::time::Duration::from_secs(1), client.health_check()).await;
    assert!(
        result.is_ok(),
        "health_check should return once headers arrive, without waiting on the response body"
    );
    assert!(result.unwrap().is_ok(), "health_check should treat a 200 status as success");
}
