# Claude Code 橋接 M2（Codex）實作計畫

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓 Claude Code CLI 透過 AITerm 橋接使用 Codex（ChatGPT 訂閱）。

**Architecture:** 新增 `bridge/upstream/codex/`，把 Anthropic Messages 請求翻譯成 Codex Responses API，並把它的 SSE 解析成既有的中立 `UpstreamEvent`。輸出端（`SseEncoder` / `MessageAggregator`）零改動。

**Tech Stack:** Rust / reqwest / serde_json；前端只動設定頁的支援矩陣。

**設計文件：** `docs/superpowers/specs/2026-08-07-claude-code-bridge-design.md`

---

## 這份計畫的事實基礎

**所有協定細節都來自實測**（`src-tauri/tests/codex_probe.rs`，commit `8672c48`），不是從 OpenAI 公開的 Responses API 文件推導的。原因：`https://chatgpt.com/backend-api/codex/responses` 是逆向的無文件私有端點，而 M1 的 Gemini 經驗證明推導不可靠 —— 錯誤的規則被上游的寬容掩蓋了好幾天。

dump 檔在 `/private/tmp/claude-501/-Users-jamesju-Documents-GitHub-AITERM/e08874e0-db22-4a64-aec9-86efc165d3c5/scratchpad/codex_probe_*`。若這些檔案已被清掉，重跑：

```bash
cd src-tauri && cargo test --test codex_probe -- --ignored --nocapture
```

### 實測確立的事實

**請求 body**（`build_request_body` 既有形狀 + `tools`）：

```json
{
  "model": "gpt-5.6-luna",
  "instructions": "<系統提示，後端必填>",
  "input": [ ... ],
  "stream": true,
  "store": false,
  "tools": [{"type":"function","name":"get_weather","description":"…","parameters":{…}}]
}
```

`tools` 是**扁平**的（`name` 直接在物件上），不是 chat.completions 的巢狀 `function` 物件。

**工具呼叫的事件序列與精確欄位**：

| 事件 | 關鍵欄位 |
|---|---|
| `response.output_item.added` | `item: {id:"fc_…", type:"function_call", status, arguments:"", call_id:"call_…", name}` |
| `response.function_call_arguments.delta` | `{delta, item_id, output_index, sequence_number, obfuscation}` |
| `response.function_call_arguments.done` | `{arguments, item_id, output_index, sequence_number}` |
| `response.output_item.done` | `item`（同 added 但 status=completed、arguments 完整） |
| `response.completed` | `response.usage.{input_tokens, output_tokens, total_tokens}` |

**reasoning 事件**（需要請求帶 `reasoning.summary: "auto"`）：

| 事件 | 關鍵欄位 |
|---|---|
| `response.output_item.added`（`item.type == "reasoning"`） | — |
| `response.reasoning_summary_text.delta` | `{delta, item_id, summary_index, output_index}` |
| `response.reasoning_summary_text.done` | — |
| `response.output_item.done` | `item.{summary:[{type:"summary_text",text}], encrypted_content:"gAAAA…"}` |

### 四個會讓「照文件寫」出錯的地方

1. **`response.completed.response.output` 是空陣列 `[]`**，即使呼叫了工具。官方文件說它帶完整 output。權威記錄只在串流過程的 `response.output_item.done`。
2. **`call_id` 與 `name` 在 `item` 物件上**，不在 delta 事件裡。
3. **往返用的是 `call_id`（`call_…`）不是 `id`（`fc_…`）。** 實測 A/B/C 三組對照確認：重建一個只有 `type`/`call_id`/`name`/`arguments` 的 `function_call` item 即可（200），完全不帶則 400（`No tool call found for function call output`）。
4. **arguments 分片與否跟長度有關。** 長內容會有多個字元級 delta，短內容一次到齊。**不能假設一定收得到多個 delta。**

### 已排除的風險

- **沒有必須原樣回送的不透明欄位。** reasoning item 連同一大包 `encrypted_content` 整個不回送也回 200。M1 為 Gemini 建的 `tool_meta` 快取在這條路徑用不到。
- **並行工具呼叫的事件不交錯**（call1 全跑完才開始 call2），`item_id` 與 `output_index` 都能歸屬且一致。多輪回送的排列（成對相鄰 vs 先全部 call 再全部 output）兩種都接受。

### 仍未觀察到的（實作時要按「可能發生」處理）

- 長 reasoning summary 是否分片（這次只有 1 個 delta）
- 兩個並行呼叫的 delta 是否真的能交錯（這次 arguments 太短，各只有 1 個 delta）
- 3 個以上並行呼叫
- reasoning 與並行工具呼叫同時出現時的交錯

**因此：累加一律以 `item_id` 分桶，不要假設順序。** 這在「不交錯」與「假設交錯」兩種情況下都正確。

---

## 比 OpenAI 路徑簡單的地方

`response.output_item.added` 就同時帶了 `call_id` 與 `name`，所以 **`ToolUseStart` 可以立刻發**，不需要 OpenAI 路徑那種「緩衝到參數到達才確定名稱」的延後邏輯（見 `bridge/upstream/openai/tool_calls.rs` 的模組註解）。這條路徑不需要 `ToolCallAccumulator`。

---

## 檔案結構

```
src-tauri/src/bridge/upstream/codex/
  mod.rs        pub mod request; pub mod stream; pub mod client;
  request.rs    MessagesRequest → Responses API body（純函式）
  stream.rs     Codex SSE → UpstreamEvent（純狀態機，逐行餵入）
  client.rs     HTTP + 串流組裝，實作 BridgeUpstream
```

修改：
- `src-tauri/src/bridge/upstream/mod.rs` — 加 `pub mod codex;`
- `src-tauri/src/bridge/factory.rs` — `UpstreamKind::Codex`、`kind_for`、`build`、`Upstream` enum
- `src/components/Settings/ClaudeBridgePage.tsx` — 支援矩陣加入 codex
- `src-tauri/tests/bridge_codex_upstream.rs`（新）
- `src-tauri/tests/bridge_end_to_end.rs` — 加 Codex 案例

---

## 階段總覽

| 階段 | 任務 | 產出 |
|---|---|---|
| A | 1–2 | 請求翻譯、SSE 解析（純函式，可單獨測） |
| B | 3–4 | adapter 接線、工廠與 UI 支援矩陣 |
| C | 5–6 | 端到端測試、手動驗收 |

---

## 階段 A：純函式

### Task 1: Anthropic → Codex Responses 請求翻譯

**Files:**
- Create: `src-tauri/src/bridge/upstream/codex/request.rs`
- Create: `src-tauri/src/bridge/upstream/codex/mod.rs`
- Modify: `src-tauri/src/bridge/upstream/mod.rs`

- [ ] **Step 1: 寫失敗測試**

建立 `src-tauri/src/bridge/upstream/codex/request.rs`，**先只寫測試區塊**：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn req(v: serde_json::Value) -> MessagesRequest {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn system_becomes_top_level_instructions() {
        // Codex 後端必填 instructions（見 ai/codex.rs:81 的註解）。
        let body = build_body(
            &req(json!({
                "model": "aiterm:sonnet", "system": "你是助手",
                "messages": [{"role": "user", "content": "hi"}]
            })),
            "gpt-5.6",
        );
        assert_eq!(body["instructions"], "你是助手");
        assert_eq!(body["model"], "gpt-5.6");
        assert_eq!(body["stream"], true);
        assert_eq!(body["store"], false);
    }

    #[test]
    fn empty_system_still_sends_instructions() {
        // 後端拒絕沒有 instructions 的請求，所以空字串也要送。
        let body = build_body(
            &req(json!({"model": "m", "messages": [{"role":"user","content":"hi"}]})),
            "m",
        );
        assert!(body.get("instructions").is_some());
    }

    #[test]
    fn user_text_becomes_input_text_message() {
        let body = build_body(
            &req(json!({"model":"m","messages":[{"role":"user","content":"hi"}]})),
            "m",
        );
        let item = &body["input"][0];
        assert_eq!(item["type"], "message");
        assert_eq!(item["role"], "user");
        assert_eq!(item["content"][0]["type"], "input_text");
        assert_eq!(item["content"][0]["text"], "hi");
    }

    #[test]
    fn assistant_text_uses_output_text() {
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"user","content":"a"},
                {"role":"assistant","content":"b"}
            ]})),
            "m",
        );
        assert_eq!(body["input"][1]["content"][0]["type"], "output_text");
    }

    #[test]
    fn tool_use_becomes_a_function_call_item() {
        // 實測：往返只需要 type/call_id/name/arguments，不需要 fc_ 開頭的 id。
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"assistant","content":[
                    {"type":"tool_use","id":"call_abc","name":"Read","input":{"p":1}}
                ]}
            ]})),
            "m",
        );
        let item = &body["input"][0];
        assert_eq!(item["type"], "function_call");
        assert_eq!(item["call_id"], "call_abc");
        assert_eq!(item["name"], "Read");
        // arguments 是 JSON 字串，不是物件。
        assert_eq!(item["arguments"], "{\"p\":1}");
        assert!(item.get("id").is_none(), "不該送 fc_ 開頭的 id");
    }

    #[test]
    fn tool_result_becomes_function_call_output() {
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"user","content":[
                    {"type":"tool_result","tool_use_id":"call_abc",
                     "content":[{"type":"text","text":"檔案內容"}]}
                ]}
            ]})),
            "m",
        );
        let item = &body["input"][0];
        assert_eq!(item["type"], "function_call_output");
        assert_eq!(item["call_id"], "call_abc");
        assert_eq!(item["output"], "檔案內容");
    }

    #[test]
    fn a_turn_with_two_tool_results_produces_two_items() {
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"user","content":[
                    {"type":"tool_result","tool_use_id":"c1","content":"a"},
                    {"type":"tool_result","tool_use_id":"c2","content":"b"}
                ]}
            ]})),
            "m",
        );
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 2);
        assert_eq!(input[0]["call_id"], "c1");
        assert_eq!(input[1]["call_id"], "c2");
    }

    #[test]
    fn tools_are_flat_not_nested() {
        // Responses API 的 tool 格式跟 chat.completions 不同：name 直接在
        // 物件上，沒有巢狀的 function 物件。
        let body = build_body(
            &req(json!({
                "model":"m","messages":[{"role":"user","content":"x"}],
                "tools":[{"name":"Read","description":"讀檔",
                          "input_schema":{"type":"object","properties":{}}}]
            })),
            "m",
        );
        let t = &body["tools"][0];
        assert_eq!(t["type"], "function");
        assert_eq!(t["name"], "Read");
        assert_eq!(t["description"], "讀檔");
        assert_eq!(t["parameters"]["type"], "object");
        assert!(t.get("function").is_none(), "不該有巢狀的 function 物件");
    }

    #[test]
    fn no_tools_means_no_tools_field() {
        let body = build_body(
            &req(json!({"model":"m","messages":[{"role":"user","content":"x"}]})),
            "m",
        );
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn thinking_request_enables_reasoning_summary() {
        // 實測：不設 summary 就完全沒有 reasoning 事件（第一輪探勘的結果）。
        let body = build_body(
            &req(json!({
                "model":"m","messages":[{"role":"user","content":"x"}],
                "thinking":{"type":"enabled","budget_tokens":8000}
            })),
            "m",
        );
        assert_eq!(body["reasoning"]["summary"], "auto");
        assert_eq!(body["reasoning"]["effort"], "medium");
    }

    #[test]
    fn no_thinking_means_no_reasoning_field() {
        let body = build_body(
            &req(json!({"model":"m","messages":[{"role":"user","content":"x"}]})),
            "m",
        );
        assert!(body.get("reasoning").is_none());
    }

    #[test]
    fn thinking_blocks_in_history_are_dropped() {
        // 實測：reasoning item 連同 encrypted_content 整個不回送也回 200，
        // 而我們產生的 thinking 區塊沒有 encrypted_content，送回去只是雜訊。
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"assistant","content":[
                    {"type":"thinking","thinking":"嗯"},
                    {"type":"text","text":"答案"}
                ]}
            ]})),
            "m",
        );
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["content"][0]["text"], "答案");
    }
}
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --lib bridge::upstream::codex::request`
Expected: 編譯失敗，`cannot find function 'build_body'`。**必須真的看到失敗**，回報時貼訊息。

- [ ] **Step 3: 實作**

在測試區塊**之前**加入：

```rust
//! Anthropic Messages 請求 → Codex Responses API 請求。
//!
//! 形狀全部來自實測（`tests/codex_probe.rs`），不是從 Responses API 公開
//! 文件推導的 —— `chatgpt.com/backend-api/codex` 是逆向的無文件端點。

use serde_json::{json, Map, Value};

use crate::bridge::anthropic::request::{
    parse_content, system_text, ContentBlock, MessagesRequest,
};

pub fn build_body(req: &MessagesRequest, model: &str) -> Value {
    let mut input: Vec<Value> = Vec::new();
    for m in &req.messages {
        push_message(&mut input, &m.role, &parse_content(&m.content));
    }

    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    // 後端拒絕沒有 instructions 的請求（見 ai/codex.rs 的註解），
    // 所以即使是空字串也要送。
    body.insert("instructions".into(), json!(system_text(req.system.as_ref())));
    body.insert("input".into(), Value::Array(input));
    body.insert("stream".into(), json!(true));
    body.insert("store".into(), json!(false));

    if let Some(tools) = &req.tools {
        if !tools.is_empty() {
            // Responses API 的 tool 是扁平的：name 直接在物件上，
            // 沒有 chat.completions 那層巢狀的 function 物件。
            let defs: Vec<Value> = tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "name": t.name,
                        "description": t.description.clone().unwrap_or_default(),
                        "parameters": t.input_schema,
                    })
                })
                .collect();
            body.insert("tools".into(), Value::Array(defs));
        }
    }

    if let Some(t) = &req.thinking {
        // 實測：不設 summary 就一個 reasoning 事件都不會出現。
        body.insert(
            "reasoning".into(),
            json!({"effort": reasoning_effort(t.budget_tokens.unwrap_or(0)), "summary": "auto"}),
        );
    }

    Value::Object(body)
}

fn reasoning_effort(budget: u32) -> &'static str {
    if budget < 4096 {
        "low"
    } else if budget < 16384 {
        "medium"
    } else {
        "high"
    }
}

/// 把一個 Anthropic turn 攤成一或多個 Responses API 的 input item。
fn push_message(out: &mut Vec<Value>, role: &str, blocks: &[ContentBlock]) {
    let mut text_parts: Vec<String> = Vec::new();

    for b in blocks {
        match b {
            ContentBlock::Text(t) => text_parts.push(t.clone()),
            // 我們產生的 thinking 區塊沒有 Codex 的 encrypted_content，
            // 送回去只是雜訊。實測整個不回送也回 200。
            ContentBlock::Thinking(_) => {}
            // 圖片：Responses API 有 input_image，但探勘沒有驗證過這個端點
            // 接不接受，所以先丟棄而不是送一個可能被拒的欄位。
            // 見 spec 的「已知限制」。
            ContentBlock::Image { .. } => {}
            ContentBlock::ToolUse { id, name, input } => out.push(json!({
                "type": "function_call",
                // 往返用 call_id（實測：fc_ 開頭的 id 不需要送）。
                "call_id": id,
                "name": name,
                "arguments": serde_json::to_string(input).unwrap_or_else(|_| "{}".into()),
            })),
            ContentBlock::ToolResult { tool_use_id, content, .. } => {
                let text: Vec<String> = content
                    .iter()
                    .filter_map(|inner| match inner {
                        ContentBlock::Text(t) => Some(t.clone()),
                        _ => None,
                    })
                    .collect();
                out.push(json!({
                    "type": "function_call_output",
                    "call_id": tool_use_id,
                    "output": text.join("\n"),
                }));
            }
        }
    }

    if !text_parts.is_empty() {
        // assistant 的文字用 output_text，其餘用 input_text
        // （沿用 ai/codex.rs:111 的既有判斷）。
        let content_type = if role == "assistant" { "output_text" } else { "input_text" };
        out.push(json!({
            "type": "message",
            "role": role,
            "content": [{"type": content_type, "text": text_parts.join("\n")}],
        }));
    }
}
```

建立 `src-tauri/src/bridge/upstream/codex/mod.rs`：

```rust
//! Codex（ChatGPT 訂閱）上游 adapter。
//!
//! 協定細節全部來自 `tests/codex_probe.rs` 的實測，見該檔與
//! `docs/superpowers/plans/2026-08-08-claude-code-bridge-m2-codex.md`。

pub mod request;
```

在 `src-tauri/src/bridge/upstream/mod.rs` 加入 `pub mod codex;`（維持字母排序，放在 `anthropic` 之後）。

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test --lib bridge::upstream::codex::request`
Expected: 12 passed。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/bridge/upstream/
git commit -m "feat(bridge): Anthropic → Codex Responses API 請求翻譯

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Codex SSE → UpstreamEvent

**Files:**
- Create: `src-tauri/src/bridge/upstream/codex/stream.rs`
- Modify: `src-tauri/src/bridge/upstream/codex/mod.rs`

- [ ] **Step 1: 寫失敗測試**

建立 `src-tauri/src/bridge/upstream/codex/stream.rs`，**先只寫測試**：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// 把幾個 SSE 資料行餵進解析器，收集全部事件。
    fn run(lines: &[&str]) -> Vec<UpstreamEvent> {
        let mut p = StreamParser::default();
        let mut out = Vec::new();
        for l in lines {
            out.extend(p.feed_line(l));
        }
        out.extend(p.finish());
        out
    }

    fn added_fn(item_id: &str, call_id: &str, name: &str) -> String {
        format!(
            r#"data: {{"type":"response.output_item.added","output_index":0,"item":{{"id":"{item_id}","type":"function_call","status":"in_progress","arguments":"","call_id":"{call_id}","name":"{name}"}}}}"#
        )
    }

    #[test]
    fn text_deltas_are_forwarded() {
        let ev = run(&[
            r#"data: {"type":"response.output_text.delta","delta":"你"}"#,
            r#"data: {"type":"response.output_text.delta","delta":"好"}"#,
            r#"data: {"type":"response.completed","response":{"usage":{"input_tokens":5,"output_tokens":2}}}"#,
        ]);
        assert_eq!(ev[0], UpstreamEvent::TextDelta("你".into()));
        assert_eq!(ev[1], UpstreamEvent::TextDelta("好".into()));
    }

    #[test]
    fn reasoning_summary_becomes_thinking_delta() {
        // 實測的事件名稱，不是從文件推的。
        let ev = run(&[
            r#"data: {"type":"response.reasoning_summary_text.delta","delta":"嗯","item_id":"rs_1"}"#,
            r#"data: {"type":"response.completed","response":{"usage":{}}}"#,
        ]);
        assert_eq!(ev[0], UpstreamEvent::ThinkingDelta("嗯".into()));
    }

    #[test]
    fn tool_use_starts_immediately_on_output_item_added() {
        // 比 OpenAI 路徑簡單的地方：added 就同時帶了 call_id 與 name，
        // 不需要緩衝到參數到達才發 ToolUseStart。
        let ev = run(&[&added_fn("fc_1", "call_1", "Read")]);
        assert_eq!(
            ev[0],
            UpstreamEvent::ToolUseStart { id: "call_1".into(), name: "Read".into() }
        );
    }

    #[test]
    fn tool_use_id_is_the_call_id_not_the_item_id() {
        // 往返用 call_id；fc_ 開頭的 id 是內部識別碼。
        let ev = run(&[&added_fn("fc_1", "call_1", "Read")]);
        match &ev[0] {
            UpstreamEvent::ToolUseStart { id, .. } => assert_eq!(id, "call_1"),
            other => panic!("預期 ToolUseStart，實際 {other:?}"),
        }
    }

    #[test]
    fn argument_fragments_are_forwarded_verbatim() {
        let ev = run(&[
            &added_fn("fc_1", "call_1", "Read"),
            r#"data: {"type":"response.function_call_arguments.delta","delta":"{\"a\"","item_id":"fc_1","output_index":0}"#,
            r#"data: {"type":"response.function_call_arguments.delta","delta":":1}","item_id":"fc_1","output_index":0}"#,
            r#"data: {"type":"response.function_call_arguments.done","arguments":"{\"a\":1}","item_id":"fc_1","output_index":0}"#,
        ]);
        assert_eq!(ev[1], UpstreamEvent::ToolInputDelta("{\"a\"".into()));
        assert_eq!(ev[2], UpstreamEvent::ToolInputDelta(":1}".into()));
        assert_eq!(ev[3], UpstreamEvent::ToolUseEnd);
    }

    #[test]
    fn done_supplies_arguments_when_no_delta_arrived() {
        // 實測：arguments 分片與否跟長度有關，短內容可能一次到齊。
        // 若完全沒有 delta，必須用 done 的完整值補上，否則工具呼叫會沒有參數。
        let ev = run(&[
            &added_fn("fc_1", "call_1", "Read"),
            r#"data: {"type":"response.function_call_arguments.done","arguments":"{\"a\":1}","item_id":"fc_1","output_index":0}"#,
        ]);
        assert_eq!(ev[1], UpstreamEvent::ToolInputDelta("{\"a\":1}".into()));
        assert_eq!(ev[2], UpstreamEvent::ToolUseEnd);
    }

    #[test]
    fn done_does_not_duplicate_already_streamed_arguments() {
        let ev = run(&[
            &added_fn("fc_1", "call_1", "Read"),
            r#"data: {"type":"response.function_call_arguments.delta","delta":"{}","item_id":"fc_1","output_index":0}"#,
            r#"data: {"type":"response.function_call_arguments.done","arguments":"{}","item_id":"fc_1","output_index":0}"#,
        ]);
        let deltas = ev.iter().filter(|e| matches!(e, UpstreamEvent::ToolInputDelta(_))).count();
        assert_eq!(deltas, 1, "done 不能重複送一次參數：{ev:?}");
    }

    #[test]
    fn two_parallel_tool_calls_are_bucketed_by_item_id() {
        // 實測時兩個呼叫沒有交錯，但探勘明確標註「沒觀察到 ≠ 不會發生」，
        // 所以用 item_id 分桶，交錯與否都正確。
        let ev = run(&[
            &added_fn("fc_1", "call_1", "A"),
            &added_fn("fc_2", "call_2", "B"),
            r#"data: {"type":"response.function_call_arguments.delta","delta":"{\"x\":1}","item_id":"fc_2","output_index":1}"#,
            r#"data: {"type":"response.function_call_arguments.delta","delta":"{\"y\":2}","item_id":"fc_1","output_index":0}"#,
            r#"data: {"type":"response.function_call_arguments.done","arguments":"{\"y\":2}","item_id":"fc_1","output_index":0}"#,
            r#"data: {"type":"response.function_call_arguments.done","arguments":"{\"x\":1}","item_id":"fc_2","output_index":1}"#,
        ]);
        // 兩個工具各自開一個區塊，參數不能串到對方身上。
        let starts: Vec<_> = ev.iter().filter_map(|e| match e {
            UpstreamEvent::ToolUseStart { id, name } => Some((id.clone(), name.clone())),
            _ => None,
        }).collect();
        assert_eq!(starts.len(), 2);
        assert!(starts.contains(&("call_1".to_string(), "A".to_string())));
        assert!(starts.contains(&("call_2".to_string(), "B".to_string())));
    }

    #[test]
    fn reasoning_output_item_does_not_start_a_tool() {
        // output_item.added 也用於 reasoning，必須靠 item.type 分辨。
        let ev = run(&[
            r#"data: {"type":"response.output_item.added","output_index":0,"item":{"id":"rs_1","type":"reasoning","summary":[]}}"#,
        ]);
        assert!(
            !ev.iter().any(|e| matches!(e, UpstreamEvent::ToolUseStart { .. })),
            "reasoning item 不該被當成工具呼叫：{ev:?}"
        );
    }

    #[test]
    fn stop_reason_is_tool_use_when_a_tool_was_called() {
        let ev = run(&[
            &added_fn("fc_1", "call_1", "Read"),
            r#"data: {"type":"response.function_call_arguments.done","arguments":"{}","item_id":"fc_1","output_index":0}"#,
            r#"data: {"type":"response.completed","response":{"usage":{"input_tokens":5,"output_tokens":2}}}"#,
        ]);
        match ev.last().unwrap() {
            UpstreamEvent::Done { stop_reason, usage } => {
                assert_eq!(*stop_reason, StopReason::ToolUse);
                assert_eq!(usage.input_tokens, 5);
                assert_eq!(usage.output_tokens, 2);
            }
            other => panic!("預期 Done，實際 {other:?}"),
        }
    }

    #[test]
    fn stop_reason_is_end_turn_for_plain_text() {
        let ev = run(&[
            r#"data: {"type":"response.output_text.delta","delta":"hi"}"#,
            r#"data: {"type":"response.completed","response":{"usage":{}}}"#,
        ]);
        assert!(matches!(
            ev.last().unwrap(),
            UpstreamEvent::Done { stop_reason: StopReason::EndTurn, .. }
        ));
    }

    #[test]
    fn response_failed_surfaces_an_error() {
        let mut p = StreamParser::default();
        let err = p.take_error(
            r#"data: {"type":"response.failed","response":{"error":{"message":"boom"}}}"#,
        );
        assert!(err.is_some(), "response.failed 必須產生錯誤");
    }

    #[test]
    fn unknown_events_are_ignored() {
        let ev = run(&[
            r#"data: {"type":"response.created"}"#,
            r#"data: {"type":"response.in_progress"}"#,
            r#"data: {"type":"response.content_part.added"}"#,
            r#"data: {"type":"response.output_text.delta","delta":"ok"}"#,
            r#"data: {"type":"response.completed","response":{"usage":{}}}"#,
        ]);
        assert_eq!(ev[0], UpstreamEvent::TextDelta("ok".into()));
    }

    #[test]
    fn malformed_json_is_skipped_not_fatal() {
        let ev = run(&[
            "data: {not json",
            r#"data: {"type":"response.output_text.delta","delta":"ok"}"#,
            r#"data: {"type":"response.completed","response":{"usage":{}}}"#,
        ]);
        assert_eq!(ev[0], UpstreamEvent::TextDelta("ok".into()));
    }

    #[test]
    fn stream_ending_without_completed_still_emits_done() {
        // 實測沒遇到，但串流中斷是常態，不能讓客戶端永遠等下去。
        let ev = run(&[r#"data: {"type":"response.output_text.delta","delta":"a"}"#]);
        assert!(matches!(ev.last(), Some(UpstreamEvent::Done { .. })));
    }
}
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --lib bridge::upstream::codex::stream`
Expected: 編譯失敗，`cannot find type 'StreamParser'`。

- [ ] **Step 3: 實作**

在測試區塊之前加入：

```rust
//! Codex Responses API SSE → [`UpstreamEvent`]。
//!
//! 事件名稱與欄位位置全部來自實測（`tests/codex_probe.rs`）。三個跟公開
//! 文件不同、會讓「照文件寫」出錯的地方：
//!
//! 1. `response.completed.response.output` 是**空陣列**，即使呼叫了工具。
//!    權威記錄只在串流過程的 `response.output_item.done`。
//! 2. `call_id` 與 `name` 在 `item` 物件上，不在 delta 事件裡。
//! 3. arguments 分片與否跟長度有關 —— 短內容一次到齊，**不能假設一定
//!    收得到 delta**，所以 `.done` 要能補位。

use std::collections::BTreeMap;

use serde_json::Value;

use crate::ai::AiError;
use crate::bridge::upstream::{StopReason, UpstreamEvent, Usage};

#[derive(Debug, Default)]
struct Slot {
    call_id: String,
    /// 是否已經收過至少一個 arguments delta。沒收過的話 `.done` 要補位。
    saw_delta: bool,
    closed: bool,
}

#[derive(Default)]
pub struct StreamParser {
    /// 以 `item_id` 分桶。實測時並行呼叫沒有交錯，但探勘明確標註
    /// 「沒觀察到 ≠ 不會發生」，分桶在兩種情況下都正確。
    slots: BTreeMap<String, Slot>,
    saw_tool: bool,
    usage: Usage,
    done_sent: bool,
}

impl StreamParser {
    pub fn feed_line(&mut self, line: &str) -> Vec<UpstreamEvent> {
        let Some(v) = parse_data_line(line) else { return Vec::new() };
        let Some(ty) = v.get("type").and_then(Value::as_str) else { return Vec::new() };

        match ty {
            "response.output_text.delta" => {
                str_field(&v, "delta").map(|t| vec![UpstreamEvent::TextDelta(t)]).unwrap_or_default()
            }
            "response.reasoning_summary_text.delta" => str_field(&v, "delta")
                .map(|t| vec![UpstreamEvent::ThinkingDelta(t)])
                .unwrap_or_default(),
            "response.output_item.added" => self.on_item_added(&v),
            "response.function_call_arguments.delta" => self.on_args_delta(&v),
            "response.function_call_arguments.done" => self.on_args_done(&v),
            "response.completed" => {
                if let Some(u) = v.get("response").and_then(|r| r.get("usage")) {
                    self.usage = Usage {
                        input_tokens: u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
                        output_tokens: u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
                    };
                }
                self.finish()
            }
            // response.created / in_progress / content_part.* / output_item.done /
            // reasoning_summary_part.* 等：不影響輸出，忽略。
            _ => Vec::new(),
        }
    }

    /// 若這一行是 `response.failed`，回傳對應的錯誤。
    pub fn take_error(&mut self, line: &str) -> Option<AiError> {
        let v = parse_data_line(line)?;
        if v.get("type").and_then(Value::as_str) != Some("response.failed") {
            return None;
        }
        let raw = v.get("response").map(|r| r.to_string()).unwrap_or_default();
        let reason = v
            .get("response")
            .and_then(|r| r.get("error"))
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("Codex 回報請求失敗")
            .to_string();
        Some(AiError::ModelError { reason, raw })
    }

    /// 串流結束時呼叫。冪等。
    pub fn finish(&mut self) -> Vec<UpstreamEvent> {
        if self.done_sent {
            return Vec::new();
        }
        self.done_sent = true;
        let mut out = Vec::new();
        // 關掉還開著的工具區塊，避免客戶端等一個永遠不來的收尾。
        let open: Vec<String> = self
            .slots
            .iter()
            .filter(|(_, s)| !s.closed)
            .map(|(k, _)| k.clone())
            .collect();
        for k in open {
            if let Some(s) = self.slots.get_mut(&k) {
                s.closed = true;
            }
            out.push(UpstreamEvent::ToolUseEnd);
        }
        let stop_reason = if self.saw_tool { StopReason::ToolUse } else { StopReason::EndTurn };
        out.push(UpstreamEvent::Done { stop_reason, usage: self.usage });
        out
    }

    fn on_item_added(&mut self, v: &Value) -> Vec<UpstreamEvent> {
        let Some(item) = v.get("item") else { return Vec::new() };
        // output_item.added 也用於 reasoning，必須靠 item.type 分辨。
        if item.get("type").and_then(Value::as_str) != Some("function_call") {
            return Vec::new();
        }
        let Some(item_id) = item.get("id").and_then(Value::as_str) else { return Vec::new() };
        let call_id = item.get("call_id").and_then(Value::as_str).unwrap_or_default().to_string();
        let name = item.get("name").and_then(Value::as_str).unwrap_or_default().to_string();

        self.saw_tool = true;
        self.slots.insert(
            item_id.to_string(),
            Slot { call_id: call_id.clone(), saw_delta: false, closed: false },
        );
        // added 就同時帶了 call_id 與 name，可以立刻開區塊 —— 不需要
        // OpenAI 路徑那種延後邏輯。往返用 call_id 不是 item id。
        vec![UpstreamEvent::ToolUseStart { id: call_id, name }]
    }

    fn on_args_delta(&mut self, v: &Value) -> Vec<UpstreamEvent> {
        let Some(item_id) = v.get("item_id").and_then(Value::as_str) else { return Vec::new() };
        let Some(delta) = str_field(v, "delta") else { return Vec::new() };
        if let Some(s) = self.slots.get_mut(item_id) {
            s.saw_delta = true;
        }
        vec![UpstreamEvent::ToolInputDelta(delta)]
    }

    fn on_args_done(&mut self, v: &Value) -> Vec<UpstreamEvent> {
        let Some(item_id) = v.get("item_id").and_then(Value::as_str) else { return Vec::new() };
        let Some(s) = self.slots.get_mut(item_id) else { return Vec::new() };
        if s.closed {
            return Vec::new();
        }
        let mut out = Vec::new();
        // 短參數可能完全沒有 delta，這時 done 的完整值就是唯一來源。
        if !s.saw_delta {
            if let Some(args) = str_field(v, "arguments") {
                out.push(UpstreamEvent::ToolInputDelta(args));
            }
        }
        s.closed = true;
        out.push(UpstreamEvent::ToolUseEnd);
        out
    }
}

fn parse_data_line(line: &str) -> Option<Value> {
    let payload = line.trim().strip_prefix("data:")?.trim();
    if payload.is_empty() || payload == "[DONE]" {
        return None;
    }
    // 壞掉的一行不該終止整個串流。
    serde_json::from_str(payload).ok()
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}
```

`Slot.call_id` 目前只在 `on_item_added` 用到；若編譯器警告未使用，**不要刪它** —— 保留是為了讓 `finish()` 的收尾在未來需要按 call 區分時有依據。加 `#[allow(dead_code)]` 並註明原因，或在 `finish()` 裡實際用到它。實作者自行判斷，並在回報裡說明選了哪個。

在 `src-tauri/src/bridge/upstream/codex/mod.rs` 加入 `pub mod stream;`。

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test --lib bridge::upstream::codex::stream`
Expected: 15 passed。

- [ ] **Step 5: 確認沒弄壞既有功能**

Run: `cd src-tauri && cargo test --lib`
Expected: 全綠（基準 611 + 12 + 15 = 638）。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/bridge/upstream/codex/
git commit -m "feat(bridge): Codex SSE 解析成中立上游事件

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## 階段 B：接線

### Task 3: Codex adapter

**Files:**
- Create: `src-tauri/src/bridge/upstream/codex/client.rs`
- Modify: `src-tauri/src/bridge/upstream/codex/mod.rs`
- Create: `src-tauri/tests/bridge_codex_upstream.rs`

- [ ] **Step 1: 寫失敗的整合測試**

建立 `src-tauri/tests/bridge_codex_upstream.rs`。**先讀 `src-tauri/tests/bridge_openai_upstream.rs`**，照它的結構寫（`collect`、`req` 等 helper 可以照抄）。

測試內容：

```rust
//! Codex 上游 adapter 的端到端測試（假上游）。

use futures_util::StreamExt;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aiterm_lib::bridge::anthropic::request::MessagesRequest;
use aiterm_lib::bridge::upstream::codex::client::CodexUpstream;
use aiterm_lib::bridge::upstream::{BridgeUpstream, StopReason, UpstreamEvent, UpstreamResponse};

fn req(v: serde_json::Value) -> MessagesRequest {
    serde_json::from_value(v).unwrap()
}

async fn collect(resp: UpstreamResponse) -> Vec<UpstreamEvent> {
    match resp {
        UpstreamResponse::Events(mut s) => {
            let mut out = Vec::new();
            while let Some(item) = s.next().await {
                out.push(item.expect("串流不該出錯"));
            }
            out
        }
        UpstreamResponse::Passthrough(_) => panic!("Codex 路徑不應回 Passthrough"),
    }
}

#[tokio::test]
async fn streams_text_and_tool_calls() {
    let server = MockServer::start().await;
    let sse = concat!(
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"開始\"}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\",\"status\":\"in_progress\",\"arguments\":\"\",\"call_id\":\"call_1\",\"name\":\"Read\"}}\n\n",
        "data: {\"type\":\"response.function_call_arguments.delta\",\"delta\":\"{}\",\"item_id\":\"fc_1\",\"output_index\":0}\n\n",
        "data: {\"type\":\"response.function_call_arguments.done\",\"arguments\":\"{}\",\"item_id\":\"fc_1\",\"output_index\":0}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":7,\"output_tokens\":3},\"output\":[]}}\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/backend-api/codex/responses"))
        .and(header("authorization", "Bearer tok"))
        // 探勘確認的必要欄位。
        .and(body_partial_json(serde_json::json!({"stream": true, "store": false})))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
        .mount(&server)
        .await;

    let up = CodexUpstream::new(server.uri(), "tok".into(), None);
    let resp = up
        .send(
            &req(serde_json::json!({
                "model": "aiterm:sonnet",
                "messages": [{"role": "user", "content": "hi"}]
            })),
            "gpt-5.6",
        )
        .await
        .unwrap();

    let ev = collect(resp).await;
    assert_eq!(ev[0], UpstreamEvent::TextDelta("開始".into()));
    assert_eq!(ev[1], UpstreamEvent::ToolUseStart { id: "call_1".into(), name: "Read".into() });
    assert_eq!(ev[2], UpstreamEvent::ToolInputDelta("{}".into()));
    assert_eq!(ev[3], UpstreamEvent::ToolUseEnd);
    assert!(matches!(ev[4], UpstreamEvent::Done { stop_reason: StopReason::ToolUse, .. }));
}

#[tokio::test]
async fn account_id_header_is_sent_when_present() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("chatgpt-account-id", "acct-1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw("data: {\"type\":\"response.completed\",\"response\":{\"usage\":{}}}\n\n", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let up = CodexUpstream::new(server.uri(), "tok".into(), Some("acct-1".into()));
    let resp = up
        .send(&req(serde_json::json!({"model":"m","messages":[{"role":"user","content":"x"}]})), "m")
        .await;
    assert!(resp.is_ok());
}

#[tokio::test]
async fn http_error_is_mapped_to_ai_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_string("no token"))
        .mount(&server)
        .await;

    let up = CodexUpstream::new(server.uri(), "bad".into(), None);
    let err = up
        .send(&req(serde_json::json!({"model":"m","messages":[{"role":"user","content":"x"}]})), "m")
        .await
        .unwrap_err();
    assert!(format!("{err:?}").contains("401") || format!("{err:?}").contains("Auth"), "實際：{err:?}");
}
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --test bridge_codex_upstream`
Expected: 編譯失敗，`could not find 'client' in 'codex'`。

- [ ] **Step 3: 實作**

建立 `src-tauri/src/bridge/upstream/codex/client.rs`。**照 `src-tauri/src/bridge/upstream/openai/client.rs` 的結構寫** —— 那支的 `into_events` 用 `futures_util::stream::unfold` 加 `ai::sse::{find_line_end, separator_len}` 切行，Codex 這支照抄同一個骨架，只換掉解析器與請求建構。

URL、headers 照 `src/ai/codex.rs` 的既有實作（`responses_url`、`apply_headers`）—— 那些是已經能正常運作的部分，**不要自己重編**。Task 探勘時已把它們提為 `pub`。

要點：
- `CodexUpstream::new(base_url, access_token, chatgpt_account_id: Option<String>)`
- URL：`{base_url}/backend-api/codex/responses`
- Headers：`Authorization: Bearer`、`originator: codex_cli_rs`、`User-Agent`、`Version`、`Openai-Beta: responses=experimental`、`X-Codex-Beta-Features`，以及 `chatgpt-account-id`（有值才送）
- body：`super::request::build_body(req, model)`
- 非 2xx → `crate::ai::sse::map_http_error(status, resp).await`
- 串流：每行先呼叫 `parser.take_error(line)`，有錯就結束串流並回該錯誤；否則 `parser.feed_line(line)`

在 `mod.rs` 加入 `pub mod client;`。

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test --test bridge_codex_upstream`
Expected: 3 passed。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/bridge/upstream/codex/ src-tauri/tests/bridge_codex_upstream.rs
git commit -m "feat(bridge): Codex 上游 adapter 與整合測試

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: 工廠與 UI 支援矩陣

**Files:**
- Modify: `src-tauri/src/bridge/factory.rs`
- Modify: `src/components/Settings/ClaudeBridgePage.tsx`
- Modify: `src/components/Settings/ClaudeBridgePage.test.tsx`

- [ ] **Step 1: 寫失敗測試（Rust）**

`src-tauri/src/bridge/factory.rs` 的測試區塊裡，把既有的 `codex_is_not_supported_in_m1` 改寫成：

```rust
    #[test]
    fn codex_maps_to_codex_kind() {
        assert_eq!(
            kind_for(&provider(ProviderType::Codex, Some("oauth"))),
            Some(UpstreamKind::Codex)
        );
    }
```

（既有測試名稱與 `provider()` helper 以實際檔案為準。）

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --lib bridge::factory`
Expected: 失敗，`no variant named 'Codex'`。

- [ ] **Step 3: 實作**

在 `factory.rs`：

1. `UpstreamKind` 加 `Codex`
2. `kind_for` 的 `ProviderType::Codex => None` 改成 `Some(UpstreamKind::Codex)`
3. `Upstream` enum 加 `Codex(CodexUpstream)`
4. `build` 加分支：

```rust
        UpstreamKind::Codex => {
            // Codex 的 access token 300 秒就過期且 refresh token 會輪替，
            // 所以每個請求都重新解析（見模組頂端的註解）。這個函式會處理
            // 刷新與回存，不要自己重寫。
            let (token, account_id) =
                crate::ai::router::get_valid_codex_oauth_token(provider_id, secrets).await?;
            // base_url 固定 https://chatgpt.com（見 ai/codex.rs），
            // 使用者填的 base_url 只在測試時用得到。
            let base = p.base_url.clone().unwrap_or_else(|| "https://chatgpt.com".into());
            Ok(Upstream::Codex(CodexUpstream::new(base, token, account_id)))
        }
```

`get_valid_codex_oauth_token` 的實際簽名以 `router.rs` 為準（探勘時已提為 `pub`，回傳 `(String, Option<String>)`）。

5. 呼叫端要處理新變體。`src/bridge/stream.rs` 與 `src/bridge/server.rs` 的 `messages_non_streaming` 各有一個 `match up`，編譯器會指出來。Codex 走 `BridgeUpstream::send`，跟 `Upstream::OpenAi` 同樣處理。

- [ ] **Step 4: 前端支援矩陣**

`src/components/Settings/ClaudeBridgePage.tsx` 的 `SUPPORTED_TYPES` 加入 `"codex"`。

`ClaudeBridgePage.test.tsx` 裡有個測試斷言 Codex 選項被停用（`把不支援的供應商標示出來且不可選`），**語意變了** —— 改成斷言 Codex **可選**，並改用 `google-ai` + `auth_method: "oauth"`（M3 才支援）當作「不支援」的樣本。在回報裡說明你怎麼改的。

- [ ] **Step 5: 驗證**

```bash
cd src-tauri && cargo test && cd ..
npm run test -- --run && npx tsc -b && npm run lint
```
Expected: Rust 全綠；前端全綠；`tsc` 乾淨；lint 與基準一致（92 problems，全是既有的）。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/bridge/ src/components/Settings/
git commit -m "feat(bridge): 工廠與設定頁支援 Codex

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## 階段 C：驗證

### Task 5: 端到端測試

**Files:**
- Modify: `src-tauri/tests/bridge_end_to_end.rs`

- [ ] **Step 1: 加測試**

先讀該檔的 `start_bridge` / `post_messages` / `collect_content_frames` helper。`start_bridge` 目前只建立 OpenAI 相容的 provider 設定，要能建立 `codex` 型別的。

加一個測試：假的 Codex 上游回含工具呼叫的 SSE，透過真的 bridge server 發請求，斷言回傳的 **Anthropic** frame 序列正確（`message_start` → text 區塊 → tool_use 區塊 → `message_delta`(stop_reason=tool_use) → `message_stop`）。

⚠️ Codex 路徑需要 OAuth token，而 `factory::build` 會呼叫 `get_valid_codex_oauth_token` 碰 keychain。若無法在測試中避開，**停下來回報** —— 不要為了讓測試跑起來而寫入使用者的鑰匙圈。可行的替代是只在 `bridge_codex_upstream.rs` 那層測（已涵蓋翻譯與解析），端到端這層留給手動驗收。

- [ ] **Step 2: 驗證**

Run: `cd src-tauri && cargo test`
Expected: 全綠。

- [ ] **Step 3: Commit**

---

### Task 6: 手動端到端驗收

自動化測試證明不了這件事能用 —— Codex 是逆向的私有端點，Claude Code 是外部程式。

- [ ] **Step 1: 全套自動化驗證**

```bash
cd src-tauri && cargo test && cd .. && npm run test -- --run && npx tsc -b && npm run lint
```
全綠才往下走。

- [ ] **Step 2: 設定**

`npm run tauri:dev`，設定 → Claude Code 橋接，把某一層指向 Codex provider（使用者的是 `GPT5.6`），儲存。

⚠️ 每次重新編譯後 macOS 會要求重新授權鑰匙圈（症狀：app 活著但零 log、橋接不上線）。診斷用 `pgrep -f SecurityAgent`。

- [ ] **Step 3: 跑完整工具循環**

開一個 Claude Code 分頁，下一個需要多步工具呼叫的指令，例如：

```
讀 package.json，列出 scripts 裡有哪些指令，然後執行 npm run lint
```

Expected：Read → 回報 → Bash → 回報，全程無斷線、無 JSON 解析錯誤。

**這一步通過才算 M2 完成。**

- [ ] **Step 4: 驗證並行工具呼叫**

下一個明確需要同時查多個檔案的指令，觀察 Claude Code 是否收到多個工具呼叫且參數沒有串錯。這是探勘裡「沒觀察到 ≠ 不會發生」的那一項，只有真實使用能驗。

- [ ] **Step 5: 驗證 thinking**

若 Claude Code 有啟用 extended thinking，觀察 thinking 區塊是否正常顯示。

- [ ] **Step 6: 更新規格**

在 `docs/superpowers/specs/2026-08-07-claude-code-bridge-design.md` 記錄 M2 的實測結果，以及探勘裡仍未解的項目（長 reasoning summary 是否分片、3 個以上並行呼叫）現在有沒有被真實使用觸發到。

---

## M2 完成條件

- [ ] `cd src-tauri && cargo test` 全綠
- [ ] `npm run test` 全綠
- [ ] `npx tsc -b` 成功
- [ ] `npm run lint` 與基準一致
- [ ] Task 6 Step 3 手動驗收通過（Codex 跑完整工具循環）

---

## 給實作者的提醒

這份計畫的協定細節來自實測，但**實測涵蓋不到的地方仍是推測**。特別是：

- 圖片（`input_image`）**完全沒驗證過**，計畫選擇丟棄而非送一個可能被拒的欄位
- 長 reasoning summary 是否分片、3 個以上並行呼叫都沒觀察到

若實作中發現任何跟這份計畫描述不符的行為，**那很可能是計畫錯了不是你錯了**。用最小的修正讓它過，並明確回報你改了什麼、為什麼。不要默默改掉，也不要為了讓測試通過而扭曲實作語意。
