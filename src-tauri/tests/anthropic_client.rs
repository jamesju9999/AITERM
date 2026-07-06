//! Contract test for `AnthropicClient` against a wiremock fake of the
//! Anthropic Messages API. Covers the happy path (SSE streaming),
//! 401 → AuthFailed, 429 → RateLimit, and 529 → Network (overloaded).

use aiterm_lib::ai::{
    anthropic::AnthropicClient, AiError, AiProvider, ChatMessage, EnvSnapshot,
    GenerateChunk, GenerateRequest, GenerateWithToolsResult, QueryMode,
};
use std::path::PathBuf;
use tokio::sync::mpsc;
use wiremock::matchers::{header, method, path};
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
        AiError::RateLimit { retry_after, .. } => {
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

#[tokio::test]
async fn tool_call_round_trip_uses_content_blocks() {
    let server = MockServer::start().await;

    let response_body = r#"{"stop_reason":"end_turn","content":[{"type":"text","text":"done"}]}"#;

    let mock = Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(response_body),
        )
        .mount_as_scoped(&server)
        .await;

    let client = AnthropicClient::with_base_url("test-key".into(), "claude-sonnet-4-5".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);

    let req = GenerateRequest {
        system_prompt: "sys".into(),
        messages: vec![
            ChatMessage {
                role: "assistant".into(),
                content: serde_json::Value::Null,
                tool_call_id: None,
                tool_calls: Some(serde_json::json!([
                    { "type": "tool_use", "id": "toolu_1", "name": "read_file", "input": { "path": "a.txt" } }
                ])),
            },
            ChatMessage {
                role: "tool".into(),
                content: serde_json::json!("file contents"),
                tool_call_id: Some("toolu_1".into()),
                tool_calls: None,
            },
        ],
        context: EnvSnapshot { os: "linux".into(), shell: "bash".into(), cwd: PathBuf::from("/"), ..Default::default() },
        mode: QueryMode::Chat,
        max_tokens: Some(256),
    };

    client.generate_with_tools(req, vec![], tx).await.expect("ok");

    let received = &mock.received_requests().await[0];
    let body: serde_json::Value = serde_json::from_slice(&received.body).unwrap();
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2, "no system message in the array (Anthropic puts it top-level)");

    let assistant_msg = &messages[0];
    assert_eq!(assistant_msg["role"], "assistant");
    assert_eq!(assistant_msg["content"][0]["type"], "tool_use");
    assert_eq!(assistant_msg["content"][0]["id"], "toolu_1");

    let tool_result_msg = &messages[1];
    assert_eq!(tool_result_msg["role"], "user");
    assert_eq!(tool_result_msg["content"][0]["type"], "tool_result");
    assert_eq!(tool_result_msg["content"][0]["tool_use_id"], "toolu_1");
    assert_eq!(tool_result_msg["content"][0]["content"], "file contents");
}

#[tokio::test]
async fn parallel_tool_results_are_merged_into_one_user_turn() {
    let server = MockServer::start().await;

    let response_body = r#"{"stop_reason":"end_turn","content":[{"type":"text","text":"done"}]}"#;

    let mock = Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(response_body),
        )
        .mount_as_scoped(&server)
        .await;

    let client = AnthropicClient::with_base_url("test-key".into(), "claude-sonnet-4-5".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);

    let req = GenerateRequest {
        system_prompt: "sys".into(),
        messages: vec![
            ChatMessage {
                role: "assistant".into(),
                content: serde_json::Value::Null,
                tool_call_id: None,
                tool_calls: Some(serde_json::json!([
                    { "type": "tool_use", "id": "toolu_1", "name": "read_file", "input": { "path": "a.txt" } },
                    { "type": "tool_use", "id": "toolu_2", "name": "read_file", "input": { "path": "b.txt" } }
                ])),
            },
            ChatMessage {
                role: "tool".into(),
                content: serde_json::json!("a contents"),
                tool_call_id: Some("toolu_1".into()),
                tool_calls: None,
            },
            ChatMessage {
                role: "tool".into(),
                content: serde_json::json!("b contents"),
                tool_call_id: Some("toolu_2".into()),
                tool_calls: None,
            },
        ],
        context: EnvSnapshot { os: "linux".into(), shell: "bash".into(), cwd: PathBuf::from("/"), ..Default::default() },
        mode: QueryMode::Chat,
        max_tokens: Some(256),
    };

    client.generate_with_tools(req, vec![], tx).await.expect("ok");

    let received = &mock.received_requests().await[0];
    let body: serde_json::Value = serde_json::from_slice(&received.body).unwrap();
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2, "the two tool results must merge into a single user turn");

    let assistant_msg = &messages[0];
    assert_eq!(assistant_msg["content"].as_array().unwrap().len(), 2);

    let tool_result_msg = &messages[1];
    assert_eq!(tool_result_msg["role"], "user");
    let blocks = tool_result_msg["content"].as_array().unwrap();
    assert_eq!(blocks.len(), 2, "both tool results must be in one user message");
    assert_eq!(blocks[0]["tool_use_id"], "toolu_1");
    assert_eq!(blocks[1]["tool_use_id"], "toolu_2");
}

#[tokio::test]
async fn openai_shaped_tool_calls_convert_to_tool_use_blocks() {
    let server = MockServer::start().await;

    let response_body = r#"{"stop_reason":"end_turn","content":[{"type":"text","text":"done"}]}"#;

    let mock = Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(response_body),
        )
        .mount_as_scoped(&server)
        .await;

    let client = AnthropicClient::with_base_url("test-key".into(), "claude-sonnet-4-5".into(), server.uri());
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

    client.generate_with_tools(req, vec![], tx).await.expect("ok");

    let received = &mock.received_requests().await[0];
    let body: serde_json::Value = serde_json::from_slice(&received.body).unwrap();
    let messages = body["messages"].as_array().unwrap();
    let assistant_msg = &messages[0];
    let block = &assistant_msg["content"][0];
    assert_eq!(block["type"], "tool_use");
    assert_eq!(block["id"], "call_1");
    assert_eq!(block["name"], "read_file");
    assert_eq!(block["input"]["path"], "a.txt", "arguments string must be parsed into an object");
}

#[tokio::test]
async fn generate_with_tools_raw_preserves_text_and_tool_use_blocks() {
    let server = MockServer::start().await;

    let body = r#"{"stop_reason":"tool_use","content":[{"type":"text","text":"Let me check that file"},{"type":"tool_use","id":"toolu_1","name":"read_file","input":{"path":"a.txt"}}]}"#;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(body),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = AnthropicClient::with_base_url("test-key".into(), "claude-sonnet-4-5".into(), server.uri());
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
            assert_eq!(calls[0].id, "toolu_1");
            let raw = raw.expect("raw should be populated");
            let blocks = raw.as_array().expect("raw should be an array");
            assert_eq!(blocks.len(), 2, "raw must include both the text block and the tool_use block");
            assert_eq!(blocks[0]["type"], "text");
            assert_eq!(blocks[1]["type"], "tool_use");
        }
        _ => panic!("expected ToolCalls"),
    }
}

#[tokio::test]
async fn mixed_raw_content_round_trips_without_misdetection() {
    let server = MockServer::start().await;

    let response_body = r#"{"stop_reason":"end_turn","content":[{"type":"text","text":"done"}]}"#;

    let mock = Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(response_body),
        )
        .mount_as_scoped(&server)
        .await;

    let client = AnthropicClient::with_base_url("test-key".into(), "claude-sonnet-4-5".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);

    // Simulates a prior turn's `raw` (text block + tool_use block) echoed back
    // verbatim as this ChatMessage's tool_calls. The first element is "text",
    // not "tool_use" — this must NOT be misdetected as OpenAI-shaped.
    let req = GenerateRequest {
        system_prompt: "sys".into(),
        messages: vec![
            ChatMessage {
                role: "assistant".into(),
                content: serde_json::Value::Null,
                tool_call_id: None,
                tool_calls: Some(serde_json::json!([
                    { "type": "text", "text": "Let me check that file" },
                    { "type": "tool_use", "id": "toolu_1", "name": "read_file", "input": { "path": "a.txt" } }
                ])),
            },
            ChatMessage {
                role: "tool".into(),
                content: serde_json::json!("file contents"),
                tool_call_id: Some("toolu_1".into()),
                tool_calls: None,
            },
        ],
        context: EnvSnapshot { os: "linux".into(), shell: "bash".into(), cwd: PathBuf::from("/"), ..Default::default() },
        mode: QueryMode::Chat,
        max_tokens: Some(256),
    };

    client.generate_with_tools(req, vec![], tx).await.expect("ok");

    let received = &mock.received_requests().await[0];
    let body: serde_json::Value = serde_json::from_slice(&received.body).unwrap();
    let messages = body["messages"].as_array().unwrap();
    let assistant_msg = &messages[0];
    let blocks = assistant_msg["content"].as_array().unwrap();
    assert_eq!(blocks.len(), 2, "both the text block and tool_use block must survive unchanged");
    assert_eq!(blocks[0]["type"], "text");
    assert_eq!(blocks[0]["text"], "Let me check that file");
    assert_eq!(blocks[1]["type"], "tool_use");
    assert_eq!(blocks[1]["id"], "toolu_1");
}
