# Claude Code 橋接 M3（Antigravity）實作計畫

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓 Claude Code CLI 透過 AITerm 橋接使用 Antigravity（Google 訂閱制 OAuth 的 Gemini）。

**Architecture:** 新增 `bridge/upstream/antigravity/`，把 Anthropic Messages 請求翻譯成 Gemini 原生格式，並把 `v1internal:streamGenerateContent` 的回應解析成既有的中立 `UpstreamEvent`。輸出端零改動；`thoughtSignature` 的往返複用 M1 為 Gemini API key 路徑建的 `tool_meta.rs`。

**設計文件：** `docs/superpowers/specs/2026-08-07-claude-code-bridge-design.md`

---

## 這份計畫的事實基礎

**所有協定細節都來自兩輪實測**（`src-tauri/tests/antigravity_probe.rs`，commit `64b1a0b` 起），不是從 Gemini 公開文件推導的。`https://cloudcode-pa.googleapis.com/v1internal:streamGenerateContent` 是逆向的無文件私有端點。

dump 檔在 `/private/tmp/claude-501/-Users-jamesju-Documents-GitHub-AITERM/e08874e0-db22-4a64-aec9-86efc165d3c5/scratchpad/antigravity_probe_*`。若已被清掉，重跑：

```bash
cd src-tauri && cargo test --test antigravity_probe -- --ignored --nocapture
```

### 實測確立的事實

**端點接受 tools**，格式是 `request.tools = [{"functionDeclarations":[{name, description, parameters}]}]`。

**`functionCall` 的 part 結構**（三個並行呼叫的實際 dump）：

```json
[{"thoughtSignature":"EjQKMgER…","functionCall":{"name":"get_weather","args":{"city":"Taipei"},"id":"MrBcwQsI"}},
 {"functionCall":{"name":"get_weather","args":{"city":"Tokyo"},"id":"FYv2YHnR"}},
 {"functionCall":{"name":"list_files","args":{"path":"/private/tmp"},"id":"CVgZwt4H"}},
 {"text":""}]
```

| 事實 | 內容 |
|---|---|
| `functionCall.id` | **原生就有**，1:1、互不相同。不需要 adapter 合成 |
| `args` | 一次到齊，沒觀察到跨 chunk 分片 |
| 並行時的 SSE | 每個 `functionCall` **各自獨立成一個 `data:` 事件**，不是塞在同一個 parts 陣列 |
| 結束 | `candidates[0].finishReason == "STOP"` |
| usage | `response.usageMetadata.{promptTokenCount, candidatesTokenCount}`（跟 `candidates` 同層） |
| 多輪工具結果 | `{"role":"user","parts":[{"functionResponse":{"name":…,"response":{…}}}]}` |
| `functionResponse` 的 `id` | 帶或不帶**都 200**，且模型沒有張冠李戴（實測有檢查語意正確性，不是只看狀態碼） |

### `thoughtSignature`：強制，但不是每個都有

這是 M3 最關鍵的一點，兩輪探勘的結論不同，**以第二輪為準**：

- **強制**：把伺服器實際附掛的那個簽章拿掉 → **400** `Function call is missing a thought_signature in functionCall parts`
- **不是 1:1**：三個並行 `functionCall` 中，**只有第 1 個帶 `thoughtSignature`**，其餘兩個**欄位本身不存在**（不是值不同）

**因此規則是：伺服器給了簽章的那個 part 才要原樣回送，沒有的直接不帶。** adapter 不需要幫每個並行呼叫合成簽章。

位置跟 M1 的 Gemini API key 路徑**不同**：這裡是原生格式，`thoughtSignature` 直接掛在 part 上跟 `functionCall` 平行，不是 OpenAI 相容格式的 `extra_content.google.thought_signature`。

M1 建的 `src/bridge/tool_meta.rs`（以工具呼叫 id 為鍵的有界快取，存整個不透明值）**可以複用**，而且因為 id 是原生的，鍵不用合成，比 M1 更簡單。

### 探勘沒涵蓋到的（實作按「可能發生」處理）

- 簽章的分布規律（只看到「第 1 個帶」，樣本是 3 次同 prompt 重跑；不確定是「永遠第一個」「隨機一個」還是跟內容有關）
- 4 個以上並行呼叫
- 多輪對話時 `position N` 錯誤訊息的數字語意

**但這些都不影響 adapter 設計** —— 只要照「有簽章就存、回送時有就帶沒有就不帶」處理，任何分布都正確。

### 兩輪都沒觀察到的

獨立的 `thought: true` part。試過簡單問題、需要多步權衡的排程題、以及並行工具情境，三種都只看到不透明的 `thoughtSignature`，沒有可讀的推理摘要。**所以 M3 不產生 `ThinkingDelta`。**

---

## 翻譯層一定會撞到、但探勘沒點出的問題

**`functionResponse` 需要 `name`，而 Anthropic 的 `tool_result` 只帶 `tool_use_id` 不帶名稱。**

```json
{"functionResponse": {"name": "get_weather", "response": {...}}}
```

Anthropic 側：

```json
{"type":"tool_result","tool_use_id":"MrBcwQsI","content":[...]}
```

解法：Claude Code 會把完整歷史送來，所以同一個請求裡一定有先前的 `tool_use` 區塊帶著 id 與 name。**先掃一遍 `messages` 建立 `id → name` 對照**，翻譯 `tool_result` 時查表。

查不到時（理論上不該發生）用空字串並記 `log::warn!`，不要讓整個請求失敗。

---

## 檔案結構

```
src-tauri/src/bridge/upstream/antigravity/
  mod.rs        pub mod request; pub mod stream; pub mod client;
  request.rs    MessagesRequest → Gemini 原生請求（純函式，吃 ToolMetaCache）
  stream.rs     Gemini chunk → UpstreamEvent（純狀態機，逐行餵入，寫入 ToolMetaCache）
  client.rs     HTTP + 串流組裝，實作 BridgeUpstream
```

修改：
- `src-tauri/src/ai/antigravity.rs` — 抽出共用的信封建構（見 Task 1）
- `src-tauri/src/bridge/upstream/mod.rs` — 加 `pub mod antigravity_native;`（**注意命名**：`bridge/upstream/anthropic.rs` 已存在，而 `antigravity` 不衝突，直接用 `pub mod antigravity;` 即可）
- `src-tauri/src/bridge/factory.rs` — `UpstreamKind::Antigravity`、`kind_for`、`build`、`Upstream` enum
- `src/components/Settings/ClaudeBridgePage.tsx` — `isSupported()` 的 google-ai + oauth 分支
- `src-tauri/tests/bridge_antigravity_upstream.rs`（新）

---

## 階段總覽

| 階段 | 任務 | 產出 |
|---|---|---|
| A | 1–2 | 請求翻譯、串流解析（純函式，可單獨測） |
| B | 3–4 | adapter 接線、工廠與 UI 支援矩陣 |
| C | 5 | 手動驗收 |

---

## 階段 A：純函式

### Task 1: Anthropic → Gemini 原生請求翻譯

**Files:**
- Create: `src-tauri/src/bridge/upstream/antigravity/request.rs`
- Create: `src-tauri/src/bridge/upstream/antigravity/mod.rs`
- Modify: `src-tauri/src/bridge/upstream/mod.rs`
- Modify: `src-tauri/src/ai/antigravity.rs`（抽出共用信封）

- [ ] **Step 1: 抽出共用的信封建構**

`ai/antigravity.rs` 的 `build_request_body` 尾端組出的外層信封是端點知識，橋接要用同一份：

```rust
serde_json::json!({
    "project": project_id,
    "requestId": request_id,
    "userAgent": "antigravity",
    "requestType": "agent",
    "model": model,
    "request": { ... },
})
```

把它抽成 `ai/antigravity.rs` 的公開函式：

```rust
/// Wrap a Gemini `request` payload in the Antigravity envelope.
///
/// The outer fields (`project`, `requestId`, `userAgent`, `requestType`) are
/// endpoint knowledge shared with the Claude Code bridge — keep one copy.
pub fn wrap_envelope(project_id: &str, model: &str, request: serde_json::Value) -> serde_json::Value {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let request_id = format!("agent/{now_ms}/{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    serde_json::json!({
        "project": project_id,
        "requestId": request_id,
        "userAgent": "antigravity",
        "requestType": "agent",
        "model": model,
        "request": request,
    })
}
```

`build_request_body` 改成呼叫它。**既有測試必須維持通過** —— 若某個測試斷言了 `requestId` 的格式，它驗的行為不變。

M2 的教訓：新增一條翻譯路徑時要逐項對照 `ai/` 底下同一供應商的既有 client，那裡的知識是踩過坑換來的。**請把 `build_request_body` 從頭讀一遍**，除了信封還有沒有其他該共用的（例如 role 映射）。有的話一併抽出，並在回報裡列出。

- [ ] **Step 2: 寫失敗測試**

建立 `src-tauri/src/bridge/upstream/antigravity/request.rs`，**先只寫測試**：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn req(v: serde_json::Value) -> MessagesRequest {
        serde_json::from_value(v).unwrap()
    }

    fn empty_cache() -> ToolMetaCache {
        ToolMetaCache::new(512)
    }

    /// 取出信封裡的 `request` 物件，測試大多只關心它。
    fn inner(v: &serde_json::Value) -> &serde_json::Value {
        &v["request"]
    }

    #[test]
    fn envelope_carries_project_and_model() {
        let body = build_body(
            &req(json!({"model":"aiterm:sonnet","messages":[{"role":"user","content":"hi"}]})),
            "gemini-2.5-flash",
            "proj-1",
            &empty_cache(),
        );
        assert_eq!(body["project"], "proj-1");
        assert_eq!(body["model"], "gemini-2.5-flash");
        assert_eq!(body["userAgent"], "antigravity");
        assert_eq!(body["requestType"], "agent");
        assert!(body["requestId"].as_str().unwrap().starts_with("agent/"));
    }

    #[test]
    fn system_becomes_system_instruction() {
        let body = build_body(
            &req(json!({"model":"m","system":"你是助手","messages":[{"role":"user","content":"hi"}]})),
            "m", "p", &empty_cache(),
        );
        assert_eq!(inner(&body)["systemInstruction"]["parts"][0]["text"], "你是助手");
    }

    #[test]
    fn user_and_assistant_roles_map_to_user_and_model() {
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"user","content":"a"},
                {"role":"assistant","content":"b"}
            ]})),
            "m", "p", &empty_cache(),
        );
        let c = inner(&body)["contents"].as_array().unwrap();
        assert_eq!(c[0]["role"], "user");
        assert_eq!(c[0]["parts"][0]["text"], "a");
        assert_eq!(c[1]["role"], "model");
    }

    #[test]
    fn max_tokens_is_passed_through() {
        // ai/antigravity.rs 把 maxOutputTokens 寫死成 16384；橋接要透傳
        // Claude Code 送來的值。
        let body = build_body(
            &req(json!({"model":"m","max_tokens":4096,"messages":[{"role":"user","content":"x"}]})),
            "m", "p", &empty_cache(),
        );
        assert_eq!(inner(&body)["generationConfig"]["maxOutputTokens"], 4096);
    }

    #[test]
    fn tools_become_function_declarations() {
        let body = build_body(
            &req(json!({
                "model":"m","messages":[{"role":"user","content":"x"}],
                "tools":[{"name":"Read","description":"讀檔",
                          "input_schema":{"type":"object","properties":{}}}]
            })),
            "m", "p", &empty_cache(),
        );
        let d = &inner(&body)["tools"][0]["functionDeclarations"][0];
        assert_eq!(d["name"], "Read");
        assert_eq!(d["description"], "讀檔");
        assert_eq!(d["parameters"]["type"], "object");
    }

    #[test]
    fn no_tools_means_no_tools_field() {
        let body = build_body(
            &req(json!({"model":"m","messages":[{"role":"user","content":"x"}]})),
            "m", "p", &empty_cache(),
        );
        assert!(inner(&body).get("tools").is_none());
    }

    #[test]
    fn tool_use_becomes_a_function_call_part() {
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"assistant","content":[
                    {"type":"tool_use","id":"MrBcwQsI","name":"Read","input":{"p":1}}
                ]}
            ]})),
            "m", "p", &empty_cache(),
        );
        let part = &inner(&body)["contents"][0]["parts"][0];
        assert_eq!(part["functionCall"]["name"], "Read");
        assert_eq!(part["functionCall"]["id"], "MrBcwQsI");
        // args 是物件，不是 JSON 字串（跟 OpenAI 不同）。
        assert_eq!(part["functionCall"]["args"]["p"], 1);
    }

    #[test]
    fn cached_thought_signature_is_reattached() {
        // 實測：拿掉伺服器給的簽章 → 400。
        let cache = empty_cache();
        cache.insert("MrBcwQsI".into(), json!("EjQKMgER-sig"));
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"assistant","content":[
                    {"type":"tool_use","id":"MrBcwQsI","name":"Read","input":{}}
                ]}
            ]})),
            "m", "p", &cache,
        );
        let part = &inner(&body)["contents"][0]["parts"][0];
        assert_eq!(part["thoughtSignature"], "EjQKMgER-sig");
    }

    #[test]
    fn missing_signature_is_simply_omitted() {
        // 實測：三個並行呼叫只有第 1 個帶簽章，其餘欄位根本不存在。
        // 沒有的就不要帶，也不要塞 null。
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"assistant","content":[
                    {"type":"tool_use","id":"NoSig","name":"Read","input":{}}
                ]}
            ]})),
            "m", "p", &empty_cache(),
        );
        let part = &inner(&body)["contents"][0]["parts"][0];
        assert!(part.get("thoughtSignature").is_none());
    }

    #[test]
    fn tool_result_becomes_function_response_with_the_name_from_history() {
        // functionResponse 需要 name，但 Anthropic 的 tool_result 只有
        // tool_use_id —— 要從同一個請求的歷史裡查出名稱。
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"assistant","content":[
                    {"type":"tool_use","id":"c1","name":"get_weather","input":{}}
                ]},
                {"role":"user","content":[
                    {"type":"tool_result","tool_use_id":"c1",
                     "content":[{"type":"text","text":"31°C"}]}
                ]}
            ]})),
            "m", "p", &empty_cache(),
        );
        let c = inner(&body)["contents"].as_array().unwrap();
        let fr = &c[1]["parts"][0]["functionResponse"];
        assert_eq!(fr["name"], "get_weather");
        assert_eq!(c[1]["role"], "user");
    }

    #[test]
    fn tool_result_with_unknown_id_still_produces_a_part() {
        // 理論上不該發生，但不能讓整個請求失敗。
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"user","content":[
                    {"type":"tool_result","tool_use_id":"ghost","content":"x"}
                ]}
            ]})),
            "m", "p", &empty_cache(),
        );
        let fr = &inner(&body)["contents"][0]["parts"][0]["functionResponse"];
        assert_eq!(fr["name"], "");
    }

    #[test]
    fn two_parallel_tool_calls_keep_their_own_ids() {
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"assistant","content":[
                    {"type":"tool_use","id":"c1","name":"A","input":{}},
                    {"type":"tool_use","id":"c2","name":"B","input":{}}
                ]}
            ]})),
            "m", "p", &empty_cache(),
        );
        let parts = inner(&body)["contents"][0]["parts"].as_array().unwrap();
        assert_eq!(parts[0]["functionCall"]["id"], "c1");
        assert_eq!(parts[1]["functionCall"]["id"], "c2");
    }

    #[test]
    fn thinking_blocks_are_dropped() {
        // 兩輪探勘都沒觀察到可讀的 thought part，我們也產生不出合法的
        // thoughtSignature，送回去只是雜訊。
        let body = build_body(
            &req(json!({"model":"m","messages":[
                {"role":"assistant","content":[
                    {"type":"thinking","thinking":"嗯"},
                    {"type":"text","text":"答案"}
                ]}
            ]})),
            "m", "p", &empty_cache(),
        );
        let parts = inner(&body)["contents"][0]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0]["text"], "答案");
    }
}
```

- [ ] **Step 3: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --lib bridge::upstream::antigravity::request`
Expected: 編譯失敗，`cannot find function 'build_body'`。**必須真的看到失敗**。

- [ ] **Step 4: 實作**

在測試區塊之前實作 `build_body(req: &MessagesRequest, model: &str, project_id: &str, tool_meta: &ToolMetaCache) -> Value`。

要點（全部來自實測，見本計畫開頭）：
- 用 `crate::ai::antigravity::wrap_envelope` 包外層
- `contents`：role 映射 `assistant` → `model`，其餘 → `user`
- `systemInstruction.parts[0].text` ← `system_text(req.system.as_ref())`
- `generationConfig.maxOutputTokens` ← `req.max_tokens`（沒給就用 `ai/antigravity.rs` 既有的 16384），`topK`/`topP` 沿用既有值
- `tools` ← `[{"functionDeclarations":[…]}]`
- `ContentBlock::ToolUse` → `{"functionCall":{"name","args","id"}}`，`args` 是**物件**不是字串；若 `tool_meta.get(id)` 有值，在**同一個 part 物件**上加 `thoughtSignature`（跟 `functionCall` 平行，不是塞進 `functionCall` 裡）
- `ContentBlock::ToolResult` → `{"functionResponse":{"name","response":{"result": <文字>}}}`，`name` 從**第一次掃描建立的 id→name 對照**查
- `ContentBlock::Thinking` / `Image` → 丟棄（圖片的 `inlineData` 兩輪探勘都沒驗證過）

**先掃一遍 `req.messages` 建立 `id → name` 對照**，再做翻譯。

- [ ] **Step 5: 執行測試確認通過**

Run: `cd src-tauri && cargo test --lib bridge::upstream::antigravity::request`
Expected: 13 passed。

同時 `cargo test --lib ai::antigravity` 必須維持通過（Step 1 的重構不改行為）。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/ai/antigravity.rs src-tauri/src/bridge/upstream/
git commit -m "feat(bridge): Anthropic → Gemini 原生請求翻譯（Antigravity）

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Antigravity 串流解析

**Files:**
- Create: `src-tauri/src/bridge/upstream/antigravity/stream.rs`
- Modify: `src-tauri/src/bridge/upstream/antigravity/mod.rs`

- [ ] **Step 1: 寫失敗測試**

建立 `stream.rs`，**先只寫測試**。回應的每個 `data:` 行是一個 chunk，外層可能是 `{"response":{…}}` 信封，也可能是裸的 chunk（`ai/antigravity.rs:186` 的既有邏輯是先試信封再退回裸的，順序重要因為所有欄位都是 `#[serde(default)]`）。

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn parser() -> (StreamParser, Arc<ToolMetaCache>) {
        let cache = Arc::new(ToolMetaCache::new(512));
        (StreamParser::new(cache.clone()), cache)
    }

    fn run(lines: &[&str]) -> Vec<UpstreamEvent> {
        let (mut p, _) = parser();
        let mut out = Vec::new();
        for l in lines {
            out.extend(p.feed_line(l));
        }
        out.extend(p.finish());
        out
    }

    #[test]
    fn text_parts_become_text_deltas() {
        let ev = run(&[
            r#"data: {"response":{"candidates":[{"content":{"parts":[{"text":"你好"}]}}]}}"#,
        ]);
        assert_eq!(ev[0], UpstreamEvent::TextDelta("你好".into()));
    }

    #[test]
    fn bare_chunk_without_envelope_also_parses() {
        // ai/antigravity.rs 兩種都處理，因為所有欄位都是 serde default。
        let ev = run(&[r#"data: {"candidates":[{"content":{"parts":[{"text":"hi"}]}}]}"#]);
        assert_eq!(ev[0], UpstreamEvent::TextDelta("hi".into()));
    }

    #[test]
    fn function_call_emits_start_input_and_end_in_one_go() {
        // 實測：args 一次到齊，沒有分片。
        let ev = run(&[
            r#"data: {"response":{"candidates":[{"content":{"parts":[{"functionCall":{"name":"Read","args":{"p":1},"id":"c1"}}]}}]}}"#,
        ]);
        assert_eq!(ev[0], UpstreamEvent::ToolUseStart { id: "c1".into(), name: "Read".into() });
        assert_eq!(ev[1], UpstreamEvent::ToolInputDelta("{\"p\":1}".into()));
        assert_eq!(ev[2], UpstreamEvent::ToolUseEnd);
    }

    #[test]
    fn thought_signature_is_cached_keyed_by_call_id() {
        // 實測：回送時拿掉它會 400。
        let (mut p, cache) = parser();
        p.feed_line(
            r#"data: {"response":{"candidates":[{"content":{"parts":[{"thoughtSignature":"SIG","functionCall":{"name":"Read","args":{},"id":"c1"}}]}}]}}"#,
        );
        assert_eq!(cache.get("c1"), Some(serde_json::json!("SIG")));
    }

    #[test]
    fn a_call_without_a_signature_caches_nothing() {
        // 實測：三個並行呼叫只有第 1 個帶簽章，其餘欄位不存在。
        let (mut p, cache) = parser();
        p.feed_line(
            r#"data: {"response":{"candidates":[{"content":{"parts":[{"functionCall":{"name":"Read","args":{},"id":"c2"}}]}}]}}"#,
        );
        assert_eq!(cache.get("c2"), None);
    }

    #[test]
    fn parallel_calls_arrive_in_separate_chunks() {
        // 實測：每個 functionCall 各自獨立成一個 data: 事件。
        let ev = run(&[
            r#"data: {"response":{"candidates":[{"content":{"parts":[{"thoughtSignature":"S1","functionCall":{"name":"A","args":{},"id":"c1"}}]}}]}}"#,
            r#"data: {"response":{"candidates":[{"content":{"parts":[{"functionCall":{"name":"B","args":{},"id":"c2"}}]}}]}}"#,
        ]);
        let starts: Vec<_> = ev.iter().filter_map(|e| match e {
            UpstreamEvent::ToolUseStart { id, name } => Some((id.clone(), name.clone())),
            _ => None,
        }).collect();
        assert_eq!(starts, vec![("c1".into(), "A".into()), ("c2".into(), "B".into())]);
    }

    #[test]
    fn finish_reason_ends_the_stream_with_usage() {
        let ev = run(&[
            r#"data: {"response":{"candidates":[{"content":{"parts":[{"text":"hi"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":7,"candidatesTokenCount":3}}}"#,
        ]);
        match ev.last().unwrap() {
            UpstreamEvent::Done { stop_reason, usage } => {
                assert_eq!(*stop_reason, StopReason::EndTurn);
                assert_eq!(usage.input_tokens, 7);
                assert_eq!(usage.output_tokens, 3);
            }
            other => panic!("預期 Done，實際 {other:?}"),
        }
    }

    #[test]
    fn stop_reason_is_tool_use_when_a_tool_was_called() {
        let ev = run(&[
            r#"data: {"response":{"candidates":[{"content":{"parts":[{"functionCall":{"name":"A","args":{},"id":"c1"}}]}}]}}"#,
            r#"data: {"response":{"candidates":[{"finishReason":"STOP"}],"usageMetadata":{}}}"#,
        ]);
        assert!(matches!(
            ev.last().unwrap(),
            UpstreamEvent::Done { stop_reason: StopReason::ToolUse, .. }
        ));
    }

    #[test]
    fn max_tokens_finish_reason_maps_to_max_tokens() {
        let ev = run(&[
            r#"data: {"response":{"candidates":[{"finishReason":"MAX_TOKENS"}],"usageMetadata":{}}}"#,
        ]);
        assert!(matches!(
            ev.last().unwrap(),
            UpstreamEvent::Done { stop_reason: StopReason::MaxTokens, .. }
        ));
    }

    #[test]
    fn empty_text_parts_emit_nothing() {
        // 實測的 dump 尾端有一個 {"text":""}。
        let ev = run(&[
            r#"data: {"response":{"candidates":[{"content":{"parts":[{"text":""}]}}]}}"#,
        ]);
        assert!(!ev.iter().any(|e| matches!(e, UpstreamEvent::TextDelta(_))));
    }

    #[test]
    fn prompt_feedback_block_surfaces_an_error() {
        let (mut p, _) = parser();
        let err = p.take_error(
            r#"data: {"response":{"candidates":[],"promptFeedback":{"blockReason":"SAFETY"}}}"#,
        );
        assert!(err.is_some(), "被擋下的請求必須產生錯誤");
    }

    #[test]
    fn malformed_json_is_skipped_not_fatal() {
        let ev = run(&[
            "data: {not json",
            r#"data: {"response":{"candidates":[{"content":{"parts":[{"text":"ok"}]}}]}}"#,
        ]);
        assert_eq!(ev[0], UpstreamEvent::TextDelta("ok".into()));
    }

    #[test]
    fn stream_ending_without_finish_reason_still_emits_done() {
        let ev = run(&[
            r#"data: {"response":{"candidates":[{"content":{"parts":[{"text":"a"}]}}]}}"#,
        ]);
        assert!(matches!(ev.last(), Some(UpstreamEvent::Done { .. })));
    }
}
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --lib bridge::upstream::antigravity::stream`
Expected: 編譯失敗，`cannot find type 'StreamParser'`。

- [ ] **Step 3: 實作**

`StreamParser::new(tool_meta: Arc<ToolMetaCache>)`，有 `feed_line(&str) -> Vec<UpstreamEvent>`、`take_error(&str) -> Option<AiError>`、`finish() -> Vec<UpstreamEvent>`（冪等）。

要點：
- 解析順序：先試 `{"response":{…}}` 信封，再退回裸 chunk（沿用 `ai/antigravity.rs:186` 的既有理由：所有欄位都是 `serde default`，順序反了會靜默解析成空的）
- `parts[]` 逐一處理：有 `text` 且非空 → `TextDelta`；有 `functionCall` → `ToolUseStart` + `ToolInputDelta(args 序列化成 JSON 字串)` + `ToolUseEnd` 一次發完（實測 args 一次到齊）
- part 上有 `thoughtSignature` 時，以 `functionCall.id` 為鍵存進 `tool_meta`
- `finishReason` 出現 → `Done`。映射：`MAX_TOKENS` → `StopReason::MaxTokens`，其餘 → 若曾有工具呼叫則 `ToolUse`，否則 `EndTurn`
- usage 來自 `usageMetadata.{promptTokenCount, candidatesTokenCount}`
- `take_error`：`candidates` 為空且有 `promptFeedback.blockReason` → `AiError::ModelError`

在 `mod.rs` 加 `pub mod stream;`。

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test --lib bridge::upstream::antigravity::stream`
Expected: 13 passed。

- [ ] **Step 5: 確認沒弄壞既有功能**

Run: `cd src-tauri && cargo test --lib`
Expected: 全綠（基準 642 + 13 + 13 = 668）。

- [ ] **Step 6: Commit**

---

## 階段 B：接線

### Task 3: Antigravity adapter

**Files:**
- Create: `src-tauri/src/bridge/upstream/antigravity/client.rs`
- Modify: `src-tauri/src/bridge/upstream/antigravity/mod.rs`
- Create: `src-tauri/tests/bridge_antigravity_upstream.rs`

- [ ] **Step 1: 寫失敗的整合測試**

照 `src-tauri/tests/bridge_codex_upstream.rs` 的結構寫（**先讀它**）。至少三個測試：

1. 串流文字 + 工具呼叫 → 正確的 `UpstreamEvent` 序列，且 `thoughtSignature` 被存進快取
2. `Authorization: Bearer` 標頭正確、body 帶 `project`
3. HTTP 錯誤 → `AiError`

`CodexUpstream` 的建構子形狀可參考；Antigravity 需要 `(base_url, access_token, project_id, tool_meta)`。

- [ ] **Step 2–4: 實作、驗證、Commit**

照 `src-tauri/src/bridge/upstream/codex/client.rs` 的骨架寫（`unfold` + `ai::sse::{find_line_end, separator_len}` 切行）。URL 與 headers 用 `ai/antigravity.rs` 既有的 `generate_content_url`、`apply_headers`（探勘時已提為 `pub`），**不要自己重編**。

每行先 `parser.take_error(line)`，有錯就結束串流；否則 `parser.feed_line(line)`。

---

### Task 4: 工廠與 UI 支援矩陣

**Files:**
- Modify: `src-tauri/src/bridge/factory.rs`
- Modify: `src/components/Settings/ClaudeBridgePage.tsx` + `.test.tsx`

- [ ] **Step 1: Rust**

`factory.rs`：
- `UpstreamKind` 加 `Antigravity`
- `kind_for` 的 `ProviderType::GoogleAi` 分支，`Some("oauth")` 從 `None` 改成 `Some(UpstreamKind::Antigravity)`
- `Upstream` enum 加變體
- `build` 加分支：用 `crate::ai::router::get_valid_google_oauth_token(provider_id, secrets).await?` 拿 `(token, project_id)`（探勘時已提為 `pub`），base_url 預設 `https://cloudcode-pa.googleapis.com`

既有測試 `google_ai_splits_on_auth_method` 斷言 oauth → `None`，**語意變了**，改寫成斷言 → `Some(UpstreamKind::Antigravity)`。在回報裡說明。

`src/bridge/stream.rs` 與 `server.rs` 的 `match up` 要加新變體，編譯器會指出來。

- [ ] **Step 2: 前端**

`ClaudeBridgePage.tsx` 的 `isSupported()` 目前對 `google-ai` + `auth_method === "oauth"` 回 `false`，改成 `true`（也就是 `google-ai` 全部支援）。

`ClaudeBridgePage.test.tsx` 有個測試用 google-ai + oauth 當「不支援」的樣本（M2 時改的），**現在沒有任何不支援的 provider type 了**。把那個測試改寫成驗證「所有已設定的 provider 都可選」，或直接刪掉並在回報裡說明理由。**用你的判斷，但要說明。**

- [ ] **Step 3: 驗證與 Commit**

```bash
cd src-tauri && cargo test && cd ..
npm run test -- --run && npx tsc -b && npm run lint
```

---

## 階段 C：驗收

### Task 5: 手動端到端驗收

- [ ] **Step 1: 全套自動化驗證** —— 全綠才往下走

- [ ] **Step 2: 設定** —— `npm run tauri:dev`，把某一層指向 Antigravity provider（使用者的是 `Gemini2.5`）

⚠️ 每次重新編譯後 macOS 會要求重新授權鑰匙圈（症狀：app 活著但零 log、橋接不上線）。診斷用 `pgrep -f SecurityAgent`。

- [ ] **Step 3: 完整工具循環** —— 開 Claude Code 分頁，下一個需要多步工具呼叫的指令。**這一步通過才算 M3 完成。**

- [ ] **Step 4: 並行工具呼叫** —— 下一個明確需要同時讀多個檔案的指令。這驗證的是 `thoughtSignature` 只有第一個 part 帶時，我們的「有就帶沒有就不帶」邏輯在真實多輪對話中成立。**這是 M3 最可能出問題的地方。**

- [ ] **Step 5: 更新規格** —— 記錄 M3 實測結果，以及探勘未解項（簽章分布規律、4 個以上並行）有沒有被真實使用觸發到

---

## M3 完成條件

- [ ] `cargo test` / `npm run test` / `npx tsc -b` 全綠，lint 與基準一致
- [ ] Task 5 Step 3 手動驗收通過
- [ ] Task 5 Step 4 並行工具呼叫驗收通過

---

## 給實作者的提醒

協定細節來自兩輪實測，但**實測涵蓋不到的仍是推測**：

- 圖片（`inlineData`）**完全沒驗證過**，計畫選擇丟棄
- 簽章分布規律、4 個以上並行呼叫、多輪對話時的錯誤訊息語意都未確認

若實作中發現跟這份計畫描述不符的行為，**很可能是計畫錯了不是你錯了**。用最小的修正讓它過，並明確回報改了什麼、為什麼。不要默默改掉，也不要為了讓測試通過而扭曲實作語意。

**M2 的教訓**：新增翻譯路徑時要逐項對照 `ai/` 底下同一供應商的既有 client —— M2 就因為漏抄 `ai/codex.rs` 的 `system` → `developer` 角色映射而在驗收時撞了 400。Task 1 Step 1 已經要求你做這件事，請認真做。
