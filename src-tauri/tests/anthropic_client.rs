//! Contract test for `AnthropicClient` against a wiremock fake of the
//! Anthropic Messages API. Covers the happy path (SSE streaming),
//! 401 → AuthFailed, 429 → RateLimit, and 529 → Network (overloaded).

use aiterm_lib::ai::{
    anthropic::AnthropicClient, AiError, AiProvider, ChatMessage, EnvSnapshot,
    GenerateChunk, GenerateRequest, QueryMode,
};
use std::path::PathBuf;
use tokio::sync::mpsc;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn req(text: &str) -> GenerateRequest {
    GenerateRequest {
        system_prompt: "sys".into(),
        messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!(text) }],
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

/// Build a multi-chunk Anthropic SSE response that yields `JSON_OUTPUT`.
const JSON_OUTPUT: &str = r#"{"explanation":"list","command":"ls","risk_level":"safe"}"#;

fn sse_happy_path() -> String {
    let part1 = &JSON_OUTPUT[..20];
    let part2 = &JSON_OUTPUT[20..];

    // Anthropic SSE format:
    //   event: content_block_delta
    //   data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"..."}}
    //
    //   event: message_stop
    //   data: {"type":"message_stop"}
    format!(
        "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":{part1_json}}}}}\n\n\
         event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":{part2_json}}}}}\n\n\
         event: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n",
        part1_json = serde_json::Value::String(part1.to_string()),
        part2_json = serde_json::Value::String(part2.to_string()),
    )
}

#[tokio::test]
async fn happy_path_streams_and_assembles_output() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "test-anthropic-key"))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_happy_path()),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = AnthropicClient::with_base_url(
        "test-anthropic-key".into(),
        "claude-sonnet-4-5".into(),
        server.uri(),
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

    let client =
        AnthropicClient::with_base_url("bad-key".into(), "claude-sonnet-4-5".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);
    let err = client.generate(req("x"), tx).await.unwrap_err();
    assert!(matches!(err, AiError::AuthFailed), "expected AuthFailed, got {err:?}");
}

#[tokio::test]
async fn returns_rate_limit_on_429() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "60")
                .set_body_string("rate limited"),
        )
        .mount(&server)
        .await;

    let client =
        AnthropicClient::with_base_url("k".into(), "claude-sonnet-4-5".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);
    let err = client.generate(req("x"), tx).await.unwrap_err();
    match err {
        AiError::RateLimit { retry_after } => {
            assert_eq!(retry_after.as_deref(), Some("60"));
        }
        other => panic!("expected RateLimit, got {other:?}"),
    }
}

#[tokio::test]
async fn returns_network_on_529_overloaded() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(529).set_body_string("overloaded"))
        .mount(&server)
        .await;

    let client =
        AnthropicClient::with_base_url("k".into(), "claude-sonnet-4-5".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);
    let err = client.generate(req("x"), tx).await.unwrap_err();
    match err {
        AiError::Network { message } => {
            assert!(
                message.to_lowercase().contains("overload"),
                "expected 'overloaded' in message, got: {message}"
            );
        }
        other => panic!("expected Network, got {other:?}"),
    }
}
