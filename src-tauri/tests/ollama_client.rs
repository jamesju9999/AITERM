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
        messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!(text), tool_call_id: None, tool_calls: None }],
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
async fn generate_with_tools_returns_tool_calls() {
    let server = MockServer::start().await;

    let body = r#"{"model":"qwen2.5","message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"brave__search","arguments":{"query":"WWDC 2026"}}}]},"done":true}"#;

    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(body),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = OllamaClient::with_base_url("qwen2.5".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(4);
    let tools = vec![aiterm_lib::ai::McpToolDefinition {
        name: "brave__search".into(),
        description: "Search the web".into(),
        input_schema: serde_json::json!({ "type": "object", "properties": { "query": { "type": "string" } } }),
    }];

    let result = client.generate_with_tools(req("search WWDC"), tools, tx).await.unwrap();
    match result {
        aiterm_lib::ai::GenerateWithToolsResult::ToolCalls { calls, raw } => {
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].tool_name, "brave__search");
            assert_eq!(calls[0].args["query"], "WWDC 2026");

            let raw = raw.expect("raw should be populated for Ollama");
            assert_eq!(raw[0]["id"], "call_0");
            assert_eq!(raw[0]["function"]["name"], "brave__search");
            let args_str = raw[0]["function"]["arguments"].as_str().expect("arguments should be a JSON string");
            let parsed: serde_json::Value = serde_json::from_str(args_str).unwrap();
            assert_eq!(parsed["query"], "WWDC 2026");
        }
        _ => panic!("expected ToolCalls, got something else"),
    }
}

#[tokio::test]
async fn generate_with_tools_returns_text_when_no_tool_calls() {
    let server = MockServer::start().await;

    let body = r#"{"model":"qwen2.5","message":{"role":"assistant","content":"Hello there"},"done":true}"#;

    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(body),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = OllamaClient::with_base_url("qwen2.5".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(4);
    let tools = vec![aiterm_lib::ai::McpToolDefinition {
        name: "dummy".into(),
        description: "dummy".into(),
        input_schema: serde_json::json!({}),
    }];

    let result = client.generate_with_tools(req("hello"), tools, tx).await.unwrap();
    match result {
        aiterm_lib::ai::GenerateWithToolsResult::Text(t) => assert_eq!(t, "Hello there"),
        _ => panic!("expected Text, got something else"),
    }
}

#[tokio::test]
async fn tool_calls_round_trip_converts_arguments_to_object_shape() {
    let server = MockServer::start().await;

    let response_body = r#"{"model":"qwen2.5","message":{"role":"assistant","content":"done"},"done":true}"#;

    let mock = Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(response_body),
        )
        .mount_as_scoped(&server)
        .await;

    let client = OllamaClient::with_base_url("qwen2.5".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(4);

    let req = GenerateRequest {
        system_prompt: "sys".into(),
        messages: vec![
            ChatMessage {
                role: "assistant".into(),
                content: serde_json::Value::Null,
                tool_call_id: None,
                tool_calls: Some(serde_json::json!([
                    { "id": "call_0", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"a.txt\"}" } }
                ])),
            },
            ChatMessage {
                role: "tool".into(),
                content: serde_json::json!("file contents"),
                tool_call_id: Some("call_0".into()),
                tool_calls: None,
            },
        ],
        context: EnvSnapshot { os: "linux".into(), shell: "bash".into(), cwd: PathBuf::from("/"), ..Default::default() },
        mode: QueryMode::Chat,
        max_tokens: None,
    };

    client.generate_with_tools(req, vec![], tx).await.expect("ok");

    let received = &mock.received_requests().await[0];
    let body: serde_json::Value = serde_json::from_slice(&received.body).unwrap();
    let messages = body["messages"].as_array().unwrap();
    // messages[0] = system, messages[1] = assistant tool call, messages[2] = tool result
    let assistant_msg = &messages[1];
    assert_eq!(assistant_msg["content"], "");
    let sent_args = &assistant_msg["tool_calls"][0]["function"]["arguments"];
    assert!(sent_args.is_object(), "Ollama expects arguments as an object, got: {sent_args}");
    assert_eq!(sent_args["path"], "a.txt");

    let tool_msg = &messages[2];
    assert_eq!(tool_msg["role"], "tool");
    assert_eq!(tool_msg["content"], "file contents");
}
