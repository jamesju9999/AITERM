//! Contract test for `OpenAiClient` against a wiremock fake of the OpenAI
//! chat completions endpoint. Covers the happy path, 401, 429 with
//! retry-after, and 500.

use aiterm_lib::ai::{
    openai::OpenAiClient, AiError, AiProvider, ChatMessage, EnvSnapshot, GenerateChunk,
    GenerateRequest, GenerateWithToolsResult, QueryMode,
};
use std::path::PathBuf;
use tokio::sync::mpsc;
use wiremock::matchers::{bearer_token, header, method, path};
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

    let client = OpenAiClient::with_base_url("test-key".into(), "gpt-4o-mini".into(), server.uri());
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

    let client = OpenAiClient::with_base_url("bad".into(), "gpt-4o-mini".into(), server.uri());
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

    let client = OpenAiClient::with_base_url("k".into(), "gpt-4o-mini".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);
    let err = client.generate(req("x"), tx).await.unwrap_err();
    match err {
        AiError::RateLimit { retry_after, .. } => {
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

    let client = OpenAiClient::with_base_url("k".into(), "gpt-4o-mini".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);
    let err = client.generate(req("x"), tx).await.unwrap_err();
    match err {
        AiError::Network { message } => assert!(message.contains("500")),
        other => panic!("expected Network, got {other:?}"),
    }
}

#[tokio::test]
async fn multipart_content_reaches_endpoint_as_array() {
    let server = MockServer::start().await;

    let mock = Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(bearer_token("test-key"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_response_happy_path()),
        )
        .mount_as_scoped(&server)
        .await;

    let client = OpenAiClient::with_base_url(
        "test-key".into(),
        "gpt-4o".into(),
        server.uri(),
    );
    let (tx, _rx) = mpsc::channel(32);
    let multipart_req = GenerateRequest {
        system_prompt: "sys".into(),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: serde_json::json!([
                {"type": "text", "text": "describe this"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,abc"}}
            ]),
            tool_call_id: None,
            tool_calls: None,
        }],
        context: EnvSnapshot {
            os: "linux".into(), shell: "bash".into(), cwd: PathBuf::from("/"), ..Default::default()
        },
        mode: QueryMode::Chat,
        max_tokens: Some(256),
    };
    client.generate(multipart_req, tx).await.unwrap();

    let received = &mock.received_requests().await[0];
    let body: serde_json::Value = serde_json::from_slice(&received.body).unwrap();
    let user_content = &body["messages"][1]["content"];
    assert!(user_content.is_array(), "content should be an array, got: {user_content}");
    assert_eq!(user_content[0]["type"], "text");
    assert_eq!(user_content[1]["type"], "image_url");
}

#[tokio::test]
async fn tool_call_history_reaches_endpoint_with_ids() {
    let server = MockServer::start().await;

    let response_body = r#"{"choices":[{"finish_reason":"stop","message":{"content":"done"}}]}"#;

    let mock = Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(response_body),
        )
        .mount_as_scoped(&server)
        .await;

    let client = OpenAiClient::with_base_url("test-key".into(), "gpt-4o-mini".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);

    let req = GenerateRequest {
        system_prompt: "sys".into(),
        messages: vec![
            ChatMessage {
                role: "assistant".into(),
                content: serde_json::Value::Null,
                tool_call_id: None,
                tool_calls: Some(serde_json::json!([
                    { "id": "call_1", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"a.txt\"}" } }
                ])),
            },
            ChatMessage {
                role: "tool".into(),
                content: serde_json::json!("file contents"),
                tool_call_id: Some("call_1".into()),
                tool_calls: None,
            },
        ],
        context: EnvSnapshot { os: "linux".into(), shell: "bash".into(), cwd: PathBuf::from("/"), ..Default::default() },
        mode: QueryMode::Chat,
        max_tokens: Some(256),
    };

    client.generate_with_tools(req, vec![], tx).await.expect("generate_with_tools ok");

    let received = &mock.received_requests().await[0];
    let body: serde_json::Value = serde_json::from_slice(&received.body).unwrap();
    let messages = body["messages"].as_array().unwrap();
    // messages[0] is the injected system message; our two messages follow.
    let assistant_msg = &messages[1];
    assert_eq!(assistant_msg["role"], "assistant");
    assert!(assistant_msg["content"].is_null(), "content must be null when tool_calls present");
    assert_eq!(assistant_msg["tool_calls"][0]["id"], "call_1");

    let tool_msg = &messages[2];
    assert_eq!(tool_msg["role"], "tool");
    assert_eq!(tool_msg["tool_call_id"], "call_1");
    assert_eq!(tool_msg["content"], "file contents");
}

#[tokio::test]
async fn generate_with_tools_populates_raw_tool_calls() {
    let server = MockServer::start().await;

    let body = r#"{"choices":[{"finish_reason":"tool_calls","message":{"tool_calls":[{"id":"call_abc","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"a.txt\"}"}}]}}]}"#;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(body),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = OpenAiClient::with_base_url("test-key".into(), "gpt-4o-mini".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);

    let req = GenerateRequest {
        system_prompt: "sys".into(),
        messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!("read a.txt"), tool_call_id: None, tool_calls: None }],
        context: EnvSnapshot { os: "linux".into(), shell: "bash".into(), cwd: PathBuf::from("/"), ..Default::default() },
        mode: QueryMode::Chat,
        max_tokens: Some(256),
    };

    let result = client.generate_with_tools(req, vec![], tx).await.expect("ok");
    match result {
        GenerateWithToolsResult::ToolCalls { calls, raw } => {
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].id, "call_abc");
            let raw = raw.expect("raw should be populated");
            assert_eq!(raw[0]["id"], "call_abc");
            assert_eq!(raw[0]["function"]["name"], "read_file");
        }
        _ => panic!("expected ToolCalls"),
    }
}
