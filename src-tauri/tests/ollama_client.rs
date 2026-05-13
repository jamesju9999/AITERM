//! Contract test for `OllamaClient` against a wiremock fake of the Ollama
//! local API. Covers the happy path (NDJSON streaming), non-2xx HTTP error
//! → Network, and health check (/api/tags) success / failure.

use aiterm_lib::ai::{
    ollama::OllamaClient, AiError, AiProvider, ChatMessage, EnvSnapshot,
    GenerateChunk, GenerateRequest, QueryMode,
};
use std::path::PathBuf;
use tokio::sync::mpsc;
use wiremock::matchers::{method, path};
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
        max_tokens: None,
    }
}

/// The JSON the AI is expected to produce, split across two NDJSON lines.
const JSON_OUTPUT: &str = r#"{"explanation":"list","command":"ls","risk_level":"safe"}"#;

fn ndjson_happy_path() -> String {
    let part1 = &JSON_OUTPUT[..20];
    let part2 = &JSON_OUTPUT[20..];
    // Ollama NDJSON format: one JSON object per line, last one has "done":true.
    format!(
        "{{\"model\":\"llama3\",\"message\":{{\"role\":\"assistant\",\"content\":{p1}}},\"done\":false}}\n\
         {{\"model\":\"llama3\",\"message\":{{\"role\":\"assistant\",\"content\":{p2}}},\"done\":false}}\n\
         {{\"model\":\"llama3\",\"message\":{{\"role\":\"assistant\",\"content\":\"\"}},\"done\":true}}\n",
        p1 = serde_json::Value::String(part1.to_string()),
        p2 = serde_json::Value::String(part2.to_string()),
    )
}

#[tokio::test]
async fn happy_path_streams_and_assembles_ndjson() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/x-ndjson")
                .set_body_string(ndjson_happy_path()),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = OllamaClient::with_base_url("llama3".into(), server.uri());
    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);

    client.generate(req("list files"), tx).await.expect("generate ok");

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
async fn returns_network_on_http_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal server error"))
        .mount(&server)
        .await;

    let client = OllamaClient::with_base_url("llama3".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);
    let err = client.generate(req("x"), tx).await.unwrap_err();
    match err {
        AiError::Network { message } => {
            assert!(message.contains("500"), "expected '500' in message, got: {message}");
        }
        other => panic!("expected Network, got {other:?}"),
    }
}

#[tokio::test]
async fn health_check_succeeds_when_tags_returns_200() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"models":[{"name":"llama3"}]}"#),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = OllamaClient::with_base_url("llama3".into(), server.uri());
    client.health_check().await.expect("health check should succeed");
}

#[tokio::test]
async fn health_check_fails_when_tags_returns_error() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(503).set_body_string("service unavailable"))
        .expect(1)
        .mount(&server)
        .await;

    let client = OllamaClient::with_base_url("llama3".into(), server.uri());
    let err = client.health_check().await.unwrap_err();
    assert!(matches!(err, AiError::Network { .. }), "expected Network, got {err:?}");
}

#[tokio::test]
async fn multipart_content_puts_images_in_images_field() {
    let server = MockServer::start().await;
    let mock = Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/x-ndjson")
                .set_body_string("{\"message\":{\"content\":\"ok\"},\"done\":true}\n"),
        )
        .mount_as_scoped(&server)
        .await;

    let client = OllamaClient::with_base_url("llava".into(), server.uri());
    let (tx, _rx) = mpsc::channel(32);
    let req = GenerateRequest {
        system_prompt: "sys".into(),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: serde_json::json!([
                {"type": "text", "text": "describe"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,abc123"}}
            ]),
        }],
        context: EnvSnapshot {
            os: "linux".into(), shell: "bash".into(),
            cwd: std::path::PathBuf::from("/"), ..Default::default()
        },
        mode: QueryMode::Chat,
        max_tokens: None,
    };
    client.generate(req, tx).await.unwrap();

    let received = &mock.received_requests().await[0];
    let body: serde_json::Value = serde_json::from_slice(&received.body).unwrap();
    // User message is at index 1 (after system at index 0)
    let user_msg = &body["messages"][1];
    assert_eq!(user_msg["content"], "describe");
    assert_eq!(user_msg["images"][0], "abc123");
}
