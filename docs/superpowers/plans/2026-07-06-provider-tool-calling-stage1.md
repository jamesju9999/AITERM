# Provider Tool-Calling 序列化修正 Stage 1（OpenAI + Ollama）實作計畫

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修正 OpenAI 與 Ollama provider 在多輪 tool-calling 時遺失 `tool_calls`/`tool_call_id` 的序列化 bug，並修復既有的測試套件編譯錯誤。

**Architecture:** `openai.rs` 比照 `compatible.rs` 已驗證正確的邏輯，序列化時轉發 `tool_call_id`/`tool_calls`、解析回應時填入 `raw`。`ollama.rs` 需要雙向格式轉換（系統統一格式的字串化 `arguments` ↔ Ollama 原生的物件 `arguments`），並在回應解析時合成 OpenAI 形狀的 `raw` 以保持下一輪轉換的自洽性。

**Tech Stack:** Rust、serde_json、wiremock（HTTP mock 測試）、tokio::test。

**Spec:** `docs/superpowers/specs/2026-07-06-provider-tool-calling-stage1-design.md`

---

## 檔案結構

| 檔案 | 動作 | 職責 |
|------|------|------|
| `src-tauri/src/ai/mod.rs` | 修改（1 處） | 修正測試中過時的 `ChatMessage` 建構 |
| `src-tauri/tests/anthropic_client.rs` | 修改（2 處） | 修正過時的 `ChatMessage`/`RateLimit` 用法 |
| `src-tauri/tests/openai_client.rs` | 修改（3 處既有 + 新增 2 個測試） | 修正過時用法 + 新增 tool-calling 序列化測試 |
| `src-tauri/tests/ai_chat_command.rs` | 修改（2 處） | 修正過時的 `ChatMessage` 建構 |
| `src-tauri/tests/compatible_client.rs` | 修改（1 處） | 修正過時的 `ChatMessage` 建構 |
| `src-tauri/tests/ai_query_command.rs` | 修改（1 處） | 修正過時的 `ChatMessage` 建構 |
| `src-tauri/tests/ollama_client.rs` | 修改（既有測試 + 新增測試） | 修正過時用法 + 新增雙向轉換測試 |
| `src-tauri/src/ai/openai.rs` | 修改 | `generate_with_tools` 序列化/解析修正 |
| `src-tauri/src/ai/ollama.rs` | 修改 | 雙向格式轉換 + `raw` 合成 |

---

### Task 1: 修復既有編譯錯誤（12 處，機械式）

**Files:**
- Modify: `src-tauri/src/ai/mod.rs:313`
- Modify: `src-tauri/tests/anthropic_client.rs:17,123`
- Modify: `src-tauri/tests/openai_client.rs:17,110,157`
- Modify: `src-tauri/tests/ai_chat_command.rs:120,124`
- Modify: `src-tauri/tests/compatible_client.rs:20`
- Modify: `src-tauri/tests/ai_query_command.rs:77`
- Modify: `src-tauri/tests/ollama_client.rs:17,160`

- [ ] **Step 1: 修正 `src-tauri/src/ai/mod.rs:313`**

Find:
```rust
        let msg = ChatMessage {
            role: "user".into(),
            content: serde_json::json!([
                {"type": "text", "text": "hello"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,abc"}}
            ]),
        };
```

Replace with:
```rust
        let msg = ChatMessage {
            role: "user".into(),
            content: serde_json::json!([
                {"type": "text", "text": "hello"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,abc"}}
            ]),
            tool_call_id: None,
            tool_calls: None,
        };
```

- [ ] **Step 2: 修正 `src-tauri/tests/anthropic_client.rs`**

Find (line 17):
```rust
        messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!(text) }],
```

Replace with:
```rust
        messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!(text), tool_call_id: None, tool_calls: None }],
```

Find (line 123):
```rust
        AiError::RateLimit { retry_after } => {
```

Replace with:
```rust
        AiError::RateLimit { retry_after, .. } => {
```

- [ ] **Step 3: 修正 `src-tauri/tests/openai_client.rs`**

Find (line 17):
```rust
        messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!(text) }],
```

Replace with:
```rust
        messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!(text), tool_call_id: None, tool_calls: None }],
```

Find (line 110):
```rust
        AiError::RateLimit { retry_after } => {
```

Replace with:
```rust
        AiError::RateLimit { retry_after, .. } => {
```

Find (lines 157-163):
```rust
        messages: vec![ChatMessage {
            role: "user".into(),
            content: serde_json::json!([
                {"type": "text", "text": "describe this"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,abc"}}
            ]),
        }],
```

Replace with:
```rust
        messages: vec![ChatMessage {
            role: "user".into(),
            content: serde_json::json!([
                {"type": "text", "text": "describe this"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,abc"}}
            ]),
            tool_call_id: None,
            tool_calls: None,
        }],
```

- [ ] **Step 4: 修正 `src-tauri/tests/ai_chat_command.rs`**

Find (lines 119-125):
```rust
fn user(text: &str) -> ChatMessage {
    ChatMessage { role: "user".into(), content: serde_json::json!(text) }
}

fn assistant(text: &str) -> ChatMessage {
    ChatMessage { role: "assistant".into(), content: serde_json::json!(text) }
}
```

Replace with:
```rust
fn user(text: &str) -> ChatMessage {
    ChatMessage { role: "user".into(), content: serde_json::json!(text), tool_call_id: None, tool_calls: None }
}

fn assistant(text: &str) -> ChatMessage {
    ChatMessage { role: "assistant".into(), content: serde_json::json!(text), tool_call_id: None, tool_calls: None }
}
```

- [ ] **Step 5: 修正 `src-tauri/tests/compatible_client.rs:20`**

Find:
```rust
        messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!(text) }],
```

Replace with:
```rust
        messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!(text), tool_call_id: None, tool_calls: None }],
```

- [ ] **Step 6: 修正 `src-tauri/tests/ai_query_command.rs:77`**

Find:
```rust
        messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!("list files") }],
```

Replace with:
```rust
        messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!("list files"), tool_call_id: None, tool_calls: None }],
```

- [ ] **Step 7: 修正 `src-tauri/tests/ollama_client.rs`**

Find (line 17):
```rust
        messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!(text) }],
```

Replace with:
```rust
        messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!(text), tool_call_id: None, tool_calls: None }],
```

Find (lines 159-166):
```rust
    match result {
        aiterm_lib::ai::GenerateWithToolsResult::ToolCalls(calls) => {
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].tool_name, "brave__search");
            assert_eq!(calls[0].args["query"], "WWDC 2026");
        }
        _ => panic!("expected ToolCalls, got something else"),
    }
```

Replace with:
```rust
    match result {
        aiterm_lib::ai::GenerateWithToolsResult::ToolCalls { calls, .. } => {
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].tool_name, "brave__search");
            assert_eq!(calls[0].args["query"], "WWDC 2026");
        }
        _ => panic!("expected ToolCalls, got something else"),
    }
```

- [ ] **Step 8: 驗證編譯通過**

Run: `cd src-tauri && cargo build --tests 2>&1 | tail -40`
Expected: 無編譯錯誤（可能仍有既有 warning，那是正常的）

- [ ] **Step 9: 跑一次完整測試套件確認基準線**

Run: `cd src-tauri && cargo test 2>&1 | tail -60`
Expected: 所有既有測試 PASS（這是我們接下來兩個 Task 的基準線）

- [ ] **Step 10: Commit**

```bash
git add src-tauri/src/ai/mod.rs src-tauri/tests/anthropic_client.rs src-tauri/tests/openai_client.rs src-tauri/tests/ai_chat_command.rs src-tauri/tests/compatible_client.rs src-tauri/tests/ai_query_command.rs src-tauri/tests/ollama_client.rs
git commit -m "fix(ai): update stale ChatMessage/RateLimit/GenerateWithToolsResult usages in tests

Test files predated the tool_call_id/tool_calls/body fields and the
ToolCalls struct-variant change, blocking cargo test --lib entirely."
```

---

### Task 2: OpenAI — 序列化 tool_calls/tool_call_id + 填入 raw

**Files:**
- Modify: `src-tauri/src/ai/openai.rs:65-138`（`generate_with_tools`）
- Test: `src-tauri/tests/openai_client.rs`

- [ ] **Step 1: 寫失敗測試**

在 `src-tauri/tests/openai_client.rs` 檔案開頭的 import 加入 `GenerateWithToolsResult`：

```rust
use aiterm_lib::ai::{
    openai::OpenAiClient, AiError, AiProvider, ChatMessage, EnvSnapshot, GenerateChunk,
    GenerateRequest, GenerateWithToolsResult, QueryMode,
};
```

在檔案末尾（`multipart_content_reaches_endpoint_as_array` 測試之後）新增：

```rust
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
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --test openai_client -- tool_call_history_reaches_endpoint_with_ids generate_with_tools_populates_raw_tool_calls`
Expected: `tool_call_history_reaches_endpoint_with_ids` FAIL（assistant_msg content 不是 null，且沒有 `tool_calls`/`tool_call_id` 欄位）；`generate_with_tools_populates_raw_tool_calls` FAIL（`raw` 是 `None`）

- [ ] **Step 3: 修改 `src-tauri/src/ai/openai.rs` 的 `generate_with_tools`**

Find (約第 82-86 行):
```rust
        let mut messages: Vec<serde_json::Value> = Vec::with_capacity(req.messages.len() + 1);
        messages.push(serde_json::json!({"role": "system", "content": req.system_prompt}));
        for m in &req.messages {
            messages.push(serde_json::json!({"role": m.role, "content": m.content}));
        }
```

Replace with:
```rust
        let mut messages: Vec<serde_json::Value> = Vec::with_capacity(req.messages.len() + 1);
        messages.push(serde_json::json!({"role": "system", "content": req.system_prompt}));
        for m in &req.messages {
            let mut msg = serde_json::json!({"role": m.role, "content": m.content});
            if let Some(id) = &m.tool_call_id {
                msg["tool_call_id"] = serde_json::Value::String(id.clone());
            }
            if let Some(tool_calls) = &m.tool_calls {
                msg["tool_calls"] = tool_calls.clone();
                msg["content"] = serde_json::Value::Null;
            }
            messages.push(msg);
        }
```

Find (約第 132 行):
```rust
            return Ok(GenerateWithToolsResult::ToolCalls { calls, raw: None });
```

Replace with:
```rust
            return Ok(GenerateWithToolsResult::ToolCalls { calls, raw: Some(choice["message"]["tool_calls"].clone()) });
```

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test --test openai_client`
Expected: 全部 PASS（含既有測試與新增的 2 個）

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/ai/openai.rs src-tauri/tests/openai_client.rs
git commit -m "fix(ai): forward tool_call_id/tool_calls and populate raw in OpenAI provider"
```

---

### Task 3: Ollama — 雙向格式轉換 + raw 合成

**Files:**
- Modify: `src-tauri/src/ai/ollama.rs`
- Test: `src-tauri/tests/ollama_client.rs`

- [ ] **Step 1: 寫失敗測試**

在 `src-tauri/tests/ollama_client.rs` 的既有 `generate_with_tools_returns_tool_calls` 測試（約第 133-167 行），把 match 區塊：

Find:
```rust
    let result = client.generate_with_tools(req("search WWDC"), tools, tx).await.unwrap();
    match result {
        aiterm_lib::ai::GenerateWithToolsResult::ToolCalls { calls, .. } => {
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].tool_name, "brave__search");
            assert_eq!(calls[0].args["query"], "WWDC 2026");
        }
        _ => panic!("expected ToolCalls, got something else"),
    }
}
```

Replace with:
```rust
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
```

(這一步的修改本身在 Task 1 已經把 pattern 從 tuple 改成 struct 且用 `..` 忽略 `raw` — 這裡是把 `..` 換成具名綁定 `raw` 並加斷言，屬於這個 Task 的失敗測試修改，不是重複勞動。)

在檔案末尾新增：

```rust
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
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --test ollama_client -- generate_with_tools_returns_tool_calls tool_calls_round_trip_converts_arguments_to_object_shape`
Expected: 兩個測試都 FAIL（`raw` 目前是 `None`；`OllamaMessage` 目前沒有 `tool_calls` 欄位，assistant_msg 送出時不會帶有 tool_calls）

- [ ] **Step 3: 修改 `src-tauri/src/ai/ollama.rs`**

Find（`OllamaMessage` 定義，約第 211-215 行）:
```rust
#[derive(Serialize)]
struct OllamaMessage {
    role: String,
    content: serde_json::Value,
}
```

Replace with:
```rust
#[derive(Serialize)]
struct OllamaMessage {
    role: String,
    content: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OllamaResponseToolCall>>,
}
```

Find（`build_messages`，約第 217-230 行）:
```rust
fn build_messages(req: &GenerateRequest) -> Vec<OllamaMessage> {
    let mut messages: Vec<OllamaMessage> = Vec::with_capacity(req.messages.len() + 1);
    messages.push(OllamaMessage {
        role: "system".to_owned(),
        content: serde_json::Value::String(req.system_prompt.clone()),
    });
    for m in &req.messages {
        messages.push(OllamaMessage {
            role: m.role.clone(),
            content: m.content.clone(),
        });
    }
    messages
}
```

Replace with:
```rust
fn build_messages(req: &GenerateRequest) -> Vec<OllamaMessage> {
    let mut messages: Vec<OllamaMessage> = Vec::with_capacity(req.messages.len() + 1);
    messages.push(OllamaMessage {
        role: "system".to_owned(),
        content: serde_json::Value::String(req.system_prompt.clone()),
        tool_calls: None,
    });
    for m in &req.messages {
        match &m.tool_calls {
            Some(tool_calls) => {
                messages.push(OllamaMessage {
                    role: m.role.clone(),
                    content: serde_json::Value::String(String::new()),
                    tool_calls: Some(to_ollama_tool_calls(tool_calls)),
                });
            }
            None => {
                messages.push(OllamaMessage {
                    role: m.role.clone(),
                    content: m.content.clone(),
                    tool_calls: None,
                });
            }
        }
    }
    messages
}

/// Convert the system's OpenAI-shaped tool_calls (arguments as a JSON string)
/// into Ollama's native shape (arguments as a parsed JSON object).
fn to_ollama_tool_calls(tool_calls: &serde_json::Value) -> Vec<OllamaResponseToolCall> {
    tool_calls
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|c| {
                    let name = c["function"]["name"].as_str().unwrap_or("").to_string();
                    let arguments = c["function"]["arguments"]
                        .as_str()
                        .and_then(|s| serde_json::from_str(s).ok())
                        .unwrap_or(serde_json::json!({}));
                    OllamaResponseToolCall {
                        function: OllamaResponseFunction { name, arguments },
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}
```

Find（`OllamaResponseToolCall`/`OllamaResponseFunction`，約第 273-282 行）:
```rust
#[derive(Deserialize)]
struct OllamaResponseToolCall {
    function: OllamaResponseFunction,
}

#[derive(Deserialize)]
struct OllamaResponseFunction {
    name: String,
    arguments: serde_json::Value,
}
```

Replace with:
```rust
#[derive(Serialize, Deserialize)]
struct OllamaResponseToolCall {
    function: OllamaResponseFunction,
}

#[derive(Serialize, Deserialize)]
struct OllamaResponseFunction {
    name: String,
    arguments: serde_json::Value,
}
```

Find（`generate_with_tools` 的回應處理，約第 173-189 行）:
```rust
        if !data.message.tool_calls.is_empty() {
            let calls = data
                .message
                .tool_calls
                .into_iter()
                .enumerate()
                .map(|(i, tc)| AiToolCall {
                    id: format!("call_{}", i),
                    tool_name: tc.function.name,
                    args: tc.function.arguments,
                    thought_signature: None,
                })
                .collect();
            Ok(GenerateWithToolsResult::ToolCalls { calls, raw: None })
        } else {
            Ok(GenerateWithToolsResult::Text(data.message.content))
        }
```

Replace with:
```rust
        if !data.message.tool_calls.is_empty() {
            let raw_tool_calls: Vec<serde_json::Value> = data.message.tool_calls.iter().enumerate()
                .map(|(i, tc)| serde_json::json!({
                    "id": format!("call_{}", i),
                    "type": "function",
                    "function": {
                        "name": tc.function.name,
                        "arguments": serde_json::to_string(&tc.function.arguments).unwrap_or_default(),
                    }
                }))
                .collect();
            let raw = Some(serde_json::Value::Array(raw_tool_calls));

            let calls = data
                .message
                .tool_calls
                .into_iter()
                .enumerate()
                .map(|(i, tc)| AiToolCall {
                    id: format!("call_{}", i),
                    tool_name: tc.function.name,
                    args: tc.function.arguments,
                    thought_signature: None,
                })
                .collect();
            Ok(GenerateWithToolsResult::ToolCalls { calls, raw })
        } else {
            Ok(GenerateWithToolsResult::Text(data.message.content))
        }
```

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test --test ollama_client`
Expected: 全部 PASS（含既有測試、修改過的 `generate_with_tools_returns_tool_calls`、新增的 round-trip 測試）

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/ai/ollama.rs src-tauri/tests/ollama_client.rs
git commit -m "fix(ai): convert tool_calls between OpenAI-shaped and Ollama-native argument formats"
```

---

### Task 4: 全面驗證

- [ ] **Step 1: 完整 Rust 測試套件**

Run: `cd src-tauri && cargo test 2>&1 | tail -80`
Expected: 全部 PASS（含 Task 1-3 新增/修改的測試，以及既有的 PTY、`agent_exec`、db 等測試）

- [ ] **Step 2: 確認前端不受影響**

Run: `npx tsc --noEmit && npm run test -- --run 2>&1 | tail -10`
Expected: tsc 無錯誤；測試數與修改前一致（195 passed / 6 個既有無關失敗），本階段完全沒有改動前端程式碼

- [ ] **Step 3: Lint**

Run: `cd src-tauri && cargo clippy --tests 2>&1 | tail -40`
Expected: 無新增 warning／error（既有 warning 如 `unused variable: finish_reason` 不是本次改動範圍，不需處理）

---

## Self-Review 紀錄

- **Spec 覆蓋**：附帶修復（12 處編譯錯誤）→ Task 1；OpenAI 序列化 + raw → Task 2；
  Ollama 雙向轉換 + raw 合成 + `OllamaMessage`/`build_messages` 更新提醒 → Task 3；
  測試與整體驗證 → Task 4。spec 中所有小節都有對應任務。
- **Placeholder 掃描**：無 TBD/TODO，所有程式碼區塊皆為完整可執行內容。
- **型別一致性**：`OllamaResponseToolCall`/`OllamaResponseFunction` 在 Task 3 內前後一致
  （新增 `Serialize` derive 後，型別名稱、欄位名稱在 `build_messages`、
  `to_ollama_tool_calls`、`generate_with_tools` 三處呼叫點完全一致）。
  `OpenAiClient`/`OllamaClient` 的 `generate_with_tools` 簽名未變動，只改函式內部邏輯。
- **已知範圍外事項**（於 spec 中列明，此處重申）：Anthropic 的訊息格式轉換為 Stage 2，
  不在本計畫範圍內；`compatible.rs`／`copilot.rs`／`router.rs`／前端型別皆不變動。
