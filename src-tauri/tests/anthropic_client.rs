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

#[tokio::test]
async fn tool_message_without_tool_call_id_sends_empty_string() {
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
                role: "tool".into(),
                content: serde_json::json!("some result"),
                tool_call_id: None,
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
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[0]["content"][0]["tool_use_id"], "");
}

#[tokio::test]
async fn empty_message_history_produces_empty_messages_array() {
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
        messages: vec![],
        context: EnvSnapshot { os: "linux".into(), shell: "bash".into(), cwd: PathBuf::from("/"), ..Default::default() },
        mode: QueryMode::Chat,
        max_tokens: Some(256),
    };

    client.generate_with_tools(req, vec![], tx).await.expect("ok");

    let received = &mock.received_requests().await[0];
    let body: serde_json::Value = serde_json::from_slice(&received.body).unwrap();
    assert_eq!(body["messages"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn leading_system_message_is_extracted_not_sent_as_a_message() {
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
        system_prompt: String::new(),
        messages: vec![
            ChatMessage {
                role: "system".into(),
                content: serde_json::json!("You are an orchestrator."),
                tool_call_id: None,
                tool_calls: None,
            },
            ChatMessage {
                role: "user".into(),
                content: serde_json::json!("go"),
                tool_call_id: None,
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
    assert_eq!(body["system"], "You are an orchestrator.");
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1, "the system message must not appear in the messages array");
    assert_eq!(messages[0]["role"], "user");
    for m in messages {
        assert_ne!(m["role"], "system", "system must never appear inside the messages array");
    }
}

#[tokio::test]
async fn generate_extracts_system_role_message_via_plain_streaming_path() {
    let server = MockServer::start().await;

    let sse_body = "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";

    let mock = Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body),
        )
        .mount_as_scoped(&server)
        .await;

    let client = AnthropicClient::with_base_url("test-key".into(), "claude-sonnet-4-5".into(), server.uri());
    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);

    let req = GenerateRequest {
        system_prompt: String::new(),
        messages: vec![
            ChatMessage {
                role: "system".into(),
                content: serde_json::json!("You are the orchestrator."),
                tool_call_id: None,
                tool_calls: None,
            },
            ChatMessage {
                role: "user".into(),
                content: serde_json::json!("go"),
                tool_call_id: None,
                tool_calls: None,
            },
        ],
        context: EnvSnapshot { os: "linux".into(), shell: "bash".into(), cwd: PathBuf::from("/"), ..Default::default() },
        mode: QueryMode::Chat,
        max_tokens: Some(256),
    };

    client.generate(req, tx).await.expect("generate ok");
    while let Some(chunk) = rx.recv().await {
        if chunk.done { break; }
    }

    let received = &mock.received_requests().await[0];
    let body: serde_json::Value = serde_json::from_slice(&received.body).unwrap();
    assert_eq!(body["system"], "You are the orchestrator.");
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    for m in messages {
        assert_ne!(m["role"], "system");
    }
}

// ── generate_with_tools：串流 ────────────────────────────────────────────────
//
// Anthropic 的工具參數是 `input_json_delta` 分片來的，要按 content block 的
// index 對位累積，收齊才 parse 成 JSON。文字則是 `text_delta`，逐段送出。

fn tool_req() -> GenerateRequest {
    GenerateRequest {
        system_prompt: "sys".into(),
        messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!("read a.txt"), tool_call_id: None, tool_calls: None }],
        context: EnvSnapshot { os: "linux".into(), shell: "bash".into(), cwd: PathBuf::from("/"), ..Default::default() },
        mode: QueryMode::Chat,
        max_tokens: Some(256),
    }
}

#[tokio::test]
async fn generate_with_tools_streams_text_and_assembles_input_json_deltas() {
    let server = MockServer::start().await;
    let sse = concat!(
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"我看一下\"}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"那個檔案\"}}\n\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_9\",\"name\":\"read_file\",\"input\":{}}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"pa\"}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"th\\\":\\\"a.txt\\\"}\"}}\n\n",
        "data: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"}}\n\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    );

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = AnthropicClient::with_base_url("k".into(), "claude-sonnet-4-5".into(), server.uri());
    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(32);
    let result = client.generate_with_tools(tool_req(), vec![], tx).await.expect("ok");

    let mut deltas = Vec::new();
    while let Some(c) = rx.recv().await {
        if !c.delta.is_empty() { deltas.push(c.delta); }
        if c.done { break; }
    }
    assert!(deltas.len() >= 2, "文字應逐段送出，實際 {} 段：{deltas:?}", deltas.len());
    assert_eq!(deltas.concat(), "我看一下那個檔案");

    match result {
        GenerateWithToolsResult::ToolCalls { calls, raw } => {
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].id, "toolu_9");
            assert_eq!(calls[0].tool_name, "read_file");
            assert_eq!(calls[0].args["path"], "a.txt", "input_json_delta 的片段要拼回物件");
            // 既有契約：raw 要含文字區塊與工具區塊兩者。
            let blocks = raw.expect("raw").as_array().expect("array").clone();
            assert_eq!(blocks.len(), 2);
            assert_eq!(blocks[0]["type"], "text");
            assert_eq!(blocks[0]["text"], "我看一下那個檔案");
            assert_eq!(blocks[1]["type"], "tool_use");
            assert_eq!(blocks[1]["id"], "toolu_9");
        }
        _ => panic!("expected ToolCalls"),
    }
}

#[tokio::test]
async fn generate_with_tools_streams_plain_text_answer() {
    let server = MockServer::start().await;
    let sse = concat!(
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"這個\"}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"專案\"}}\n\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = AnthropicClient::with_base_url("k".into(), "claude-sonnet-4-5".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(32);
    let result = client.generate_with_tools(tool_req(), vec![], tx).await.expect("ok");
    match result {
        GenerateWithToolsResult::Text(t) => assert_eq!(t, "這個專案"),
        _ => panic!("expected Text"),
    }
}

// ── 訂閱 OAuth + 工具呼叫的計費落差 ──────────────────────────────────────────
//
// 實測（Claude Pro 訂閱、credits 餘額 $0）：請求只要帶 `tools`，Anthropic 就回
// 400「You're out of extra usage」——那些請求被算到 API credits 那個桶，不是訂閱
// 額度（訂閱用量頁顯示 0%）。拿掉 tools 的同一組請求則正常走訂閱。
//
// 變數已用三次 A/B 切乾淨：串流與 max_tokens 都無關，唯一因素是 tools 的有無。
//
// 這等於「這個憑證在此情境下無法使用原生工具呼叫」，正是 ToolCallingUnsupported
// 的語意——回這個錯誤，上層現成的「工具描述注入系統提示」fallback 就會接手，
// 而 fallback 送出的請求不帶 tools，因此回到訂閱計費。

const OUT_OF_USAGE_BODY: &str = r#"{"type":"error","error":{"type":"invalid_request_error","message":"You're out of extra usage. Add more at claude.ai/settings/usage and keep going."},"request_id":"req_x"}"#;

#[tokio::test]
async fn oauth_out_of_extra_usage_with_tools_maps_to_tool_calling_unsupported() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(400).set_body_string(OUT_OF_USAGE_BODY))
        .expect(1)
        .mount(&server)
        .await;

    let client = AnthropicClient::with_oauth("oat-token".into(), "claude-sonnet-4-5".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);
    let err = match client.generate_with_tools(tool_req(), vec![], tx).await {
        Err(e) => e,
        Ok(_) => panic!("expected an error"),
    };

    assert!(
        matches!(err, AiError::ToolCallingUnsupported),
        "應對映成 ToolCallingUnsupported 以觸發既有的 fallback，實際是 {err:?}"
    );
}

// API key（非訂閱）用戶付的是 API 額度，這個訊息對他們就是真的餘額不足——
// 不能一併吞掉，否則會用一個假的「不支援工具」蓋掉真正的計費問題。
#[tokio::test]
async fn api_key_out_of_usage_stays_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(400).set_body_string(OUT_OF_USAGE_BODY))
        .expect(1)
        .mount(&server)
        .await;

    let client = AnthropicClient::with_base_url("sk-ant-key".into(), "claude-sonnet-4-5".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);
    let err = match client.generate_with_tools(tool_req(), vec![], tx).await {
        Err(e) => e,
        Ok(_) => panic!("expected an error"),
    };

    assert!(
        !matches!(err, AiError::ToolCallingUnsupported),
        "API key 用戶的餘額不足是真錯誤，不可以偽裝成不支援工具"
    );
}

// 其他 400（例如請求真的畸形）必須照原樣是錯誤，不能被這條規則誤吞。
#[tokio::test]
async fn oauth_other_400_is_not_swallowed() {
    let server = MockServer::start().await;
    let body = r#"{"type":"error","error":{"type":"invalid_request_error","message":"messages: at least one message is required"}}"#;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(400).set_body_string(body))
        .expect(1)
        .mount(&server)
        .await;

    let client = AnthropicClient::with_oauth("oat-token".into(), "claude-sonnet-4-5".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);
    let err = match client.generate_with_tools(tool_req(), vec![], tx).await {
        Err(e) => e,
        Ok(_) => panic!("expected an error"),
    };

    assert!(!matches!(err, AiError::ToolCallingUnsupported), "不相干的 400 不該被當成不支援工具");
}
