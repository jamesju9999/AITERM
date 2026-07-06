# Provider Tool-Calling 序列化修正 Stage 2（Anthropic）實作計畫

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修正 Anthropic provider 在多輪 tool-calling 時的訊息格式，轉換成 Anthropic Messages API 要求的 content-block 結構（無 `tool` role，需 `tool_use`/`tool_result` block + 角色交替）。

**Architecture:** 新增 `build_anthropic_messages`（將 `ChatMessage` 歷史轉換成 Anthropic 格式，合併連續 `tool` 訊息成一則 `user` 訊息）與 `to_anthropic_content_blocks`（處理 Anthropic 原生與 OpenAI 形狀兩種 `tool_calls` 輸入）兩個純函式，取代 `generate_with_tools` 現有的單行 map；回應解析改為保留完整 `content` 陣列到 `raw`。

**Tech Stack:** Rust、serde_json、wiremock、tokio::test。

**Spec:** `docs/superpowers/specs/2026-07-09-provider-tool-calling-stage2-anthropic-design.md`

**前置條件：** 這個分支要從 `feature/provider-tool-calling-stage1`（Stage 1，PR #6，尚未 merge）切出，
不是從 `master`——因為 `tests/anthropic_client.rs` 的 `ChatMessage` 編譯修正只在
Stage 1 分支上，master 上還沒有，否則 `cargo build --tests` 會因為 Stage 1 已知的
舊編譯錯誤而失敗。

---

## 檔案結構

| 檔案 | 動作 | 職責 |
|------|------|------|
| `src-tauri/src/ai/anthropic.rs` | 修改 | 新增 `build_anthropic_messages`/`to_anthropic_content_blocks`，修改 `generate_with_tools` |
| `src-tauri/tests/anthropic_client.rs` | 修改（新增 6 個測試） | 驗證訊息轉換、角色合併、形狀容錯、raw 保留 |

---

### Task 1: Anthropic 訊息格式轉換 + raw 保留完整 content

**Files:**
- Modify: `src-tauri/src/ai/anthropic.rs`
- Test: `src-tauri/tests/anthropic_client.rs`

- [ ] **Step 1: 寫失敗測試**

在 `src-tauri/tests/anthropic_client.rs` 檔案開頭的 import，把:
```rust
use aiterm_lib::ai::{
    anthropic::AnthropicClient, AiError, AiProvider, ChatMessage, EnvSnapshot,
    GenerateChunk, GenerateRequest, QueryMode,
};
```
改為加入 `GenerateWithToolsResult`:
```rust
use aiterm_lib::ai::{
    anthropic::AnthropicClient, AiError, AiProvider, ChatMessage, EnvSnapshot,
    GenerateChunk, GenerateRequest, GenerateWithToolsResult, QueryMode,
};
```

在檔案末尾（既有測試之後）新增以下 6 個測試：

```rust
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
```

(第 6 個測試「純文字訊息不受影響」不需要新增程式碼——既有的
`request_body_puts_system_at_top_level` 單元測試涵蓋這個路徑，本 Task 完全不
修改 `build_request_body`，Step 4 的完整測試跑起來時會一併驗證它仍然通過。)

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --test anthropic_client -- tool_call_round_trip_uses_content_blocks parallel_tool_results_are_merged_into_one_user_turn openai_shaped_tool_calls_convert_to_tool_use_blocks generate_with_tools_raw_preserves_text_and_tool_use_blocks mixed_raw_content_round_trips_without_misdetection`
Expected: 全部 5 個新測試 FAIL（訊息序列化仍是 `{"role","content"}` 直通、`raw` 仍是 `None`）

- [ ] **Step 3: 修改 `src-tauri/src/ai/anthropic.rs`**

Find（`generate_with_tools` 目前建構 messages 的地方）:
```rust
        let messages: Vec<serde_json::Value> = req.messages.iter().map(|m| {
            serde_json::json!({ "role": m.role, "content": m.content })
        }).collect();
```

Replace with:
```rust
        let messages: Vec<serde_json::Value> = build_anthropic_messages(&req.messages);
```

Find（`raw: None` 的回傳）:
```rust
            return Ok(GenerateWithToolsResult::ToolCalls { calls, raw: None });
```

Replace with:
```rust
            return Ok(GenerateWithToolsResult::ToolCalls {
                calls,
                raw: Some(serde_json::Value::Array(content_blocks.clone())),
            });
```

在檔案中 `generate_with_tools` 方法**之後**（`impl AiProvider for AnthropicClient` 區塊
結束之後，`// ── Request types ──` 註解之前的任何位置都可以，建議放在
`build_request_body` 函式之前）新增這三個函式：

```rust
/// Convert internal ChatMessage history into Anthropic's Messages API format.
/// Anthropic has no "tool" role: tool calls live inside an assistant message's
/// `content` array as `tool_use` blocks, and tool results are wrapped in a
/// `user` message's `content` array as `tool_result` blocks. Consecutive
/// `role: "tool"` ChatMessages (parallel tool calls) are coalesced into one
/// user turn, since Anthropic requires strictly alternating roles.
fn build_anthropic_messages(messages: &[ChatMessage]) -> Vec<serde_json::Value> {
    let mut result: Vec<serde_json::Value> = Vec::with_capacity(messages.len());
    let mut pending_tool_results: Vec<serde_json::Value> = Vec::new();

    for m in messages {
        if m.role == "tool" {
            pending_tool_results.push(serde_json::json!({
                "type": "tool_result",
                "tool_use_id": m.tool_call_id.clone().unwrap_or_default(),
                "content": m.content.clone(),
            }));
            continue;
        }

        flush_tool_results(&mut result, &mut pending_tool_results);

        if m.role == "assistant" {
            if let Some(tool_calls) = &m.tool_calls {
                result.push(serde_json::json!({
                    "role": "assistant",
                    "content": to_anthropic_content_blocks(tool_calls),
                }));
                continue;
            }
        }

        result.push(serde_json::json!({ "role": m.role, "content": m.content }));
    }

    flush_tool_results(&mut result, &mut pending_tool_results);
    result
}

fn flush_tool_results(result: &mut Vec<serde_json::Value>, pending: &mut Vec<serde_json::Value>) {
    if !pending.is_empty() {
        result.push(serde_json::json!({
            "role": "user",
            "content": std::mem::take(pending),
        }));
    }
}

/// Convert a ChatMessage's `tool_calls` value into Anthropic content blocks.
/// Handles two possible shapes: Anthropic-native (already `tool_use` blocks,
/// e.g. echoed back verbatim from a prior `raw`) and OpenAI-shaped (the
/// frontend's fallback reconstruction, `function.arguments` as a JSON string).
/// Detection: OpenAI-shaped elements always have a `"function"` key; Anthropic
/// content blocks (whether `text` or `tool_use`) never do — so checking only
/// the first element would misdetect a `[text, tool_use]` raw echo.
fn to_anthropic_content_blocks(tool_calls: &serde_json::Value) -> Vec<serde_json::Value> {
    let Some(arr) = tool_calls.as_array() else {
        return vec![];
    };

    let is_openai_shaped = arr.iter().any(|el| el.get("function").is_some());
    if !is_openai_shaped {
        return arr.clone();
    }

    arr.iter()
        .filter(|el| el.get("function").is_some())
        .map(|el| {
            let arguments = el["function"]["arguments"]
                .as_str()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(serde_json::json!({}));
            serde_json::json!({
                "type": "tool_use",
                "id": el["id"],
                "name": el["function"]["name"],
                "input": arguments,
            })
        })
        .collect()
}
```

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test --test anthropic_client`
Expected: 全部 PASS（含既有 4 個測試 + 新增的 5 個）

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/ai/anthropic.rs src-tauri/tests/anthropic_client.rs
git commit -m "fix(ai): convert tool_calls to Anthropic content blocks and preserve raw content"
```

---

### Task 2: 全面驗證

- [ ] **Step 1: 完整 Rust 測試套件**

Run: `cd src-tauri && cargo test --lib -- --test-threads=1 2>&1 | tail -20`
Expected: 159 passed，15 個既有無關失敗（macOS 上跑 Windows 路徑假設的 pty 測試 +
1 個無關的 `ai_single_command` 測試）——與 Stage 1 驗證時完全相同的基準線，
本 Task 只改 `anthropic.rs`，不會影響這些。

Run: `cd src-tauri && cargo test --test anthropic_client --test openai_client --test ollama_client --test compatible_client --test ai_chat_command --test ai_query_command 2>&1 | tail -60`
Expected: 全部 PASS（Stage 1 的 openai/ollama 測試 + 本階段新增的 anthropic 測試）

- [ ] **Step 2: 確認前端不受影響**

Run: `cd /Users/jamesju/Documents/GitHub/AITERM && npx tsc --noEmit && npm run test -- --run 2>&1 | tail -10`
Expected: tsc 無錯誤；195 passed / 6 個既有無關失敗，本階段完全沒有改動前端程式碼

- [ ] **Step 3: Lint**

Run: `cd src-tauri && cargo clippy --tests 2>&1 | grep -A5 "ai/anthropic.rs\|tests/anthropic_client.rs"`
Expected: 無輸出（本次改動的檔案沒有新增 clippy warning）

---

## Self-Review 紀錄

- **Spec 覆蓋**：回應解析（raw 保留完整 content）→ Task 1 Step 3 的第二個 find/replace；
  `build_anthropic_messages`（角色合併）→ Task 1 Step 3 新函式；`to_anthropic_content_blocks`
  （形狀容錯，含自我審查抓到的「不能只看第一個元素」修正）→ Task 1 Step 3 新函式；
  6 個測試情境（單一 round-trip、平行合併、OpenAI 形狀相容、raw 保留混合內容、
  純文字不受影響、混合內容形狀偵測邊界案例）→ Task 1 Step 1（前 5 個為新增測試，
  第 6 個由既有測試 + Task 2 Step 1 的完整跑測覆蓋）。
- **Placeholder 掃描**：無 TBD/TODO，所有程式碼區塊為完整可執行內容。
- **型別一致性**：`build_anthropic_messages(messages: &[ChatMessage]) -> Vec<serde_json::Value>`、
  `to_anthropic_content_blocks(tool_calls: &serde_json::Value) -> Vec<serde_json::Value>`、
  `flush_tool_results(result: &mut Vec<serde_json::Value>, pending: &mut Vec<serde_json::Value>)`
  三者的簽名在 Task 1 Step 3 的呼叫點與定義處完全一致。
- **已知範圍外事項**（spec 中已列明，此處重申）：不建立通用角色合併邏輯；
  不修改 `generate()`/`build_request_body`/`AnthropicMessage`；不處理 Ollama 的
  既有 fast-follow 項目；不處理 `thought_signature`。
