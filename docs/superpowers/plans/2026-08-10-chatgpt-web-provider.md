# ChatGPT Web 供應商實作計畫

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓 AITerm 能使用 ChatGPT 網頁版訂閱的模型，同時供 `/ai`、聊天面板與 Claude Code 橋接使用。

**Architecture:** 用一個隱藏的 Tauri `WebviewWindow` 載入 chatgpt.com 當傳輸層 —— 所有上游請求都由注入該頁面的 JS 發出，因此天然帶有真實瀏覽器的 TLS 指紋與 cookie，不需要指紋偽裝也不需要儲存 session cookie。Rust 端負責組請求（歷史攤平、工具契約）與解析回應（SSE → 中性事件），JS 端負責認證、sentinel 工作量證明與 fetch。

**Tech Stack:** Rust（Tauri 2.10、axum、tokio）、TypeScript／JS（注入腳本，vitest 測試）、React（設定 UI）

## 執行進度

| 任務 | 狀態 | commit |
|---|---|---|
| Task 1 模組骨架與 ProviderType | ✅ 完成 | `cb88bcd` |
| Task 2 歷史攤平 | ✅ 完成 | `4a7b413` `a147e19` `c5e155b` + 收尾 |
| Task 3–16 | 未開始 | |

**分支**：`feat/chatgpt-web-provider`（master 未動）
**執行方式**：Task 2–7 用 subagent 全套（實作＋規格審查＋品質審查）；
Task 8–16 由主 session 直接實作並自行驗證。理由：純函式的錯誤不會編譯失敗、
也看不出來，值得雙重審查；整合與 UI 的錯誤回饋很快。

**Task 1 的審查發現（後續任務要沿用的教訓）**：
`ai/router.rs` 的 `default_base_url_covers_every_provider_type` 這類「自稱涵蓋每種
type」的測試不受編譯器保護——新增變體時窮舉 match 會強制你補 match 臂，但不會
強制你補測試斷言。新增任何 provider 相關分支後，要手動檢查這類清單型測試。

**Task 2 的審查發現（後續任務要沿用的教訓）**：

1. **判斷測試有沒有效，要把實作改壞去跑，不要用推理**。Task 2 的
   `system_prompt_omitted_when_empty` 原本只餵 `""`，把實作的 `trim()` 拿掉後測試
   照樣全綠——這個關鍵細節根本沒被鎖住。審查者是實際暫改實作才發現的。
   後續每個純函式任務（Task 3–7）都用這招驗一遍。
2. **序列化格式要用一個 `assert_eq!` 精確比對來鎖**，不要只用 `contains`。
   `contains` 鎖不住角色前綴、分隔符、區塊順序——而這些就是格式本身，
   Task 5 的剖析器要靠它。意圖測試保留（失敗訊息好讀），精確比對是額外一道鎖。
3. **守衛用了 `trim()`，推進去的值也要用**。Task 2 一度只 trim 判斷、卻推原字串，
   使得「格式鎖」對「system 區塊帶尾端換行」這個最常見的真實輸入並不成立。
4. **`0 passed` 不是通過**。Rust 對「模組目錄下存在但未被宣告的 `.rs` 檔」靜默不
   編譯，無錯誤也無警告，`cargo test` 會印 `0 passed` 且 exit code 0。每個新模組
   任務都要先確認 `mod.rs` 有宣告，並把「N 個測試 PASS」當成斷言看。

**Task 3 的審查發現（後續任務要沿用的教訓）**：

5. **手寫的 fixture 會把「我以為的上游長相」固化成測試**。Task 3 的 7 個測試全綠，
   但解析器有兩個真實 bug：串流中段夾一個沒有內容的 frame 會清空差分狀態、
   使用者看到重複文字；以及沒人負責跨 chunk 組行，回答結尾會永久少一截。兩者
   都是因為測試的 JSON 太乾淨。Task 16 刪探勘程式碼前要先錄一條真實串流。
6. **測試的「位置」跟輸入一樣重要**。`malformed_frames_are_skipped_not_fatal`
   把畸形 frame 放在串流最開頭，那時狀態本來就是空的，重設沒有可觀察後果；
   同樣的輸入放到中段就會爆。有狀態的東西，邊界案例要放在中段測。
7. **審查者要去讀還沒執行的後續任務**。Task 3 的品質審查是因為去讀了 Task 8/9/
   11/12/13 的計畫內容，才發現「組行責任無人認領」與「`sinks` 從不移除會讓三個
   消費端永久卡住」。那些任務還沒動工，當下修的成本接近零。

---

**設計依據：** `docs/superpowers/specs/2026-08-10-chatgpt-web-provider-design.md`。該 spec 的「探勘實證」章節是實測值，實作時不要重新推測。

---

## 檔案結構

| 檔案 | 職責 |
|---|---|
| `src-tauri/src/chatgpt_web/mod.rs` | 模組匯出、`ChatgptWebError`、全域 `Session` 存取點 |
| `src-tauri/src/chatgpt_web/protocol.rs` | 歷史攤平（含工具回合編碼）、ChatGPT SSE → `UpstreamEvent` |
| `src-tauri/src/chatgpt_web/tools.rs` | 工具契約序列化（雙位置＋nonce）、`<tool>` 封套剖析 |
| `src-tauri/src/chatgpt_web/session.rs` | webview 生命週期、請求配對、Tauri command |
| `src-tauri/src/chatgpt_web/inject.js` | auth／sentinel／PoW／fetch／chunk 回傳 |
| `src/lib/chatgptWebInject.test.ts` | `inject.js` 純函式的 vitest 測試 |
| `src-tauri/src/ai/chatgpt_web.rs` | 實作 `AiProvider` |
| `src-tauri/src/bridge/upstream/chatgpt_web.rs` | 實作 `BridgeUpstream` |
| `src-tauri/capabilities/chatgpt-web.json` | 只給 `chatgpt-web` 視窗、只對 `https://chatgpt.com/*` 開放 IPC |

**需要修改的既有檔案**（新增 `ProviderType` 變體會觸發窮舉 match，漏改會靜默壞掉）：

- `src-tauri/src/config/types.rs` — enum、`Display`、序列化對應
- `src-tauri/src/ai/router.rs` — `default_base_url()`、`openai_chat_url()`、provider 建構
- `src-tauri/src/bridge/factory.rs` — `kind_for()`、`build()`
- `src-tauri/src/lib.rs` — 模組宣告、command 註冊、setup 初始化
- `src/components/Settings/ClaudeBridgePage.tsx` — `SUPPORTED_TYPES`
- `src/lib/i18n.ts` — en / zh-TW 字串

---

## Task 1：模組骨架與 ProviderType

**Files:**
- Create: `src-tauri/src/chatgpt_web/mod.rs`
- Modify: `src-tauri/src/config/types.rs`
- Modify: `src-tauri/src/lib.rs`

- [x] **Step 1: 寫失敗的測試**

在 `src-tauri/src/config/types.rs` 既有的 `mod tests` 內加入：

```rust
#[test]
fn chatgpt_web_provider_type_round_trips() {
    let json = serde_json::to_string(&ProviderType::ChatgptWeb).unwrap();
    assert_eq!(json, r#""chatgpt-web""#);
    let back: ProviderType = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ProviderType::ChatgptWeb);
    assert_eq!(ProviderType::ChatgptWeb.to_string(), "ChatGPT Web");
}
```

- [x] **Step 2: 執行測試確認失敗**

```bash
cd src-tauri && cargo test --lib config::types::tests::chatgpt_web_provider_type_round_trips
```

預期：編譯失敗，`no variant named ChatgptWeb found for enum ProviderType`

- [x] **Step 3: 加入 enum 變體**

`src-tauri/src/config/types.rs`，在 `ProviderType` 的 `Codex,` 之後加入：

```rust
    /// ChatGPT 網頁版（chatgpt.com/backend-api/conversation）。
    /// 與 `Codex` 不同：那是 Responses API + 原生 function calling，
    /// 這是網頁前端自己的後端 + prompt 模擬工具。兩者吃同一份訂閱額度。
    ChatgptWeb,
```

在 `impl std::fmt::Display for ProviderType` 的 match 內加入：

```rust
            ProviderType::ChatgptWeb => write!(f, "ChatGPT Web"),
```

找到既有測試裡的 `(ProviderType::Codex, "codex"),` 這一組對應表，在其後加入：

```rust
            (ProviderType::ChatgptWeb, "chatgpt-web"),
```

- [x] **Step 4: 執行測試確認通過**

```bash
cd src-tauri && cargo test --lib config::types
```

預期：全部 PASS

- [x] **Step 5: 補齊所有窮舉點**

```bash
cd src-tauri && cargo check 2>&1 | grep -A5 "non-exhaustive\|patterns.*not covered"
```

依編譯錯誤逐一補上。至少會有這兩處：

`src-tauri/src/ai/router.rs` 的 `default_base_url()`：

```rust
        // 網頁版走 webview 傳輸，base_url 由 session 固定為 https://chatgpt.com，
        // 不從設定讀。
        ProviderType::ChatgptWeb => None,
```

`src-tauri/src/ai/router.rs` 的 `openai_chat_url()`：把 `ProviderType::ChatgptWeb` 加進**現有的 `Codex` 那一組**（同樣不會被實際使用，但必須列出）。

- [x] **Step 6: 建立模組骨架**

`src-tauri/src/chatgpt_web/mod.rs`：

```rust
//! ChatGPT 網頁版供應商。
//!
//! 傳輸層是一個載入 chatgpt.com 的隱藏 WebviewWindow：所有上游請求都由注入
//! 該頁面的 JS 發出，因此天然帶有真實瀏覽器的 TLS 指紋與 cookie。設計與實測
//! 依據見 docs/superpowers/specs/2026-08-10-chatgpt-web-provider-design.md。

```

> 注意：Task 1 **不**宣告任何子模組——`protocol.rs` 與 `tools.rs` 都還不存在，
> 宣告了會編譯失敗。`pub mod protocol;` 由 Task 2 補、`pub mod tools;` 由 Task 4 補。

`src-tauri/src/lib.rs`，在 `pub mod bridge;` 之後加入：

```rust
pub mod chatgpt_web;
```

- [x] **Step 7: 確認編譯與既有測試**

```bash
cd src-tauri && cargo check && cargo test --lib config
```

預期：`Finished`，測試全 PASS

- [x] **Step 8: Commit**

```bash
git add src-tauri/src/chatgpt_web/mod.rs src-tauri/src/config/types.rs src-tauri/src/ai/router.rs src-tauri/src/lib.rs
git commit -m "feat(chatgpt-web): 新增 ProviderType::ChatgptWeb 與模組骨架"
```

---

## Task 2：歷史攤平

把 Claude Code／`/ai` 的訊息序列攤平成 ChatGPT 網頁版需要的單一 user turn。

**Files:**
- Create: `src-tauri/src/chatgpt_web/protocol.rs`

- [ ] **Step 1: 寫失敗的測試**

`src-tauri/src/chatgpt_web/protocol.rs` 檔案結尾：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_system_and_turns_in_order() {
        let out = flatten_history(
            "你是助理",
            &[
                FlatTurn::User("第一問".into()),
                FlatTurn::Assistant("第一答".into()),
                FlatTurn::User("第二問".into()),
            ],
        );
        assert!(out.starts_with("你是助理"), "system 要在最前面，實際：{out}");
        let first = out.find("第一問").unwrap();
        let second = out.find("第二問").unwrap();
        assert!(first < second, "順序要保留");
        assert!(out.contains("第一答"));
    }

    #[test]
    fn system_prompt_omitted_when_empty() {
        let out = flatten_history("", &[FlatTurn::User("只有這句".into())]);
        assert!(!out.starts_with('\n'), "空的 system 不該留下空行，實際：{out:?}");
        assert!(out.contains("只有這句"));
    }

    /// 界定符刻意與模型要輸出的 `<tool>` 封套不同——共用會讓剖析器把歷史
    /// 誤判成模型發出的新呼叫。
    #[test]
    fn tool_turns_use_distinct_delimiters() {
        let out = flatten_history(
            "",
            &[
                FlatTurn::ToolCall { id: "call_1".into(), name: "Read".into(),
                                     args: r#"{"path":"a.txt"}"#.into() },
                FlatTurn::ToolResult { id: "call_1".into(), content: "檔案內容".into() },
            ],
        );
        assert!(out.contains("[[tool_call:Read#call_1]]"), "實際：{out}");
        assert!(out.contains(r#"{"path":"a.txt"}"#));
        assert!(out.contains("[[/tool_call]]"));
        assert!(out.contains("[[tool_result:call_1]]"));
        assert!(out.contains("檔案內容"));
        assert!(out.contains("[[/tool_result]]"));
        assert!(!out.contains("<tool>"), "不可使用模型輸出用的封套標籤");
        assert!(!out.contains("<tool_call"), "不可使用模型輸出用的封套標籤");
    }
}
```

- [ ] **Step 2: 執行測試確認失敗**

```bash
cd src-tauri && cargo test --lib chatgpt_web::protocol
```

預期：編譯失敗，`cannot find function flatten_history` 與 `cannot find type FlatTurn`

- [ ] **Step 3: 實作**

`src-tauri/src/chatgpt_web/protocol.rs` 開頭：

```rust
//! ChatGPT 網頁版的協定轉換：歷史攤平與 SSE 解析。

/// 攤平前的一個回合。
///
/// 網頁版沒有 `role: "tool"` 這種結構化角色，工具回合只能以文字回填——
/// 這是此傳輸路徑的先天限制，不是實作取捨。見 spec 的「工具結果的回填」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlatTurn {
    User(String),
    Assistant(String),
    ToolCall { id: String, name: String, args: String },
    ToolResult { id: String, content: String },
}

/// 把 system prompt 與整段歷史攤平成單一 user turn 的內容。
///
/// 界定符 `[[tool_call:…]]` / `[[tool_result:…]]` 刻意與模型要輸出的
/// `<tool>` 封套不同：若共用同一組標籤，`tools::parse_tool_calls` 可能把
/// 歷史裡我們自己寫進去的回合誤判成模型發出的新呼叫。nonce 檢查雖然也會
/// 擋下（歷史回合沒有 `_nonce`），但不該把正確性建立在第二道防線上。
pub fn flatten_history(system_prompt: &str, turns: &[FlatTurn]) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !system_prompt.trim().is_empty() {
        parts.push(system_prompt.to_string());
    }
    for turn in turns {
        parts.push(match turn {
            FlatTurn::User(text) => format!("User: {text}"),
            FlatTurn::Assistant(text) => format!("Assistant: {text}"),
            FlatTurn::ToolCall { id, name, args } => {
                format!("[[tool_call:{name}#{id}]]\n{args}\n[[/tool_call]]")
            }
            FlatTurn::ToolResult { id, content } => {
                format!("[[tool_result:{id}]]\n{content}\n[[/tool_result]]")
            }
        });
    }
    parts.join("\n\n")
}
```

- [ ] **Step 4: 執行測試確認通過**

```bash
cd src-tauri && cargo test --lib chatgpt_web::protocol
```

預期：3 個測試全 PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/chatgpt_web/protocol.rs
git commit -m "feat(chatgpt-web): 歷史攤平，工具回合用專屬界定符"
```

---

## Task 3：SSE 解析

ChatGPT 網頁版回的每個 SSE data 是一份完整的訊息快照（累積文字），不是增量。要轉成 `UpstreamEvent::TextDelta` 必須自己算差分。

**Files:**
- Modify: `src-tauri/src/chatgpt_web/protocol.rs`

- [ ] **Step 1: 寫失敗的測試**

加到 `protocol.rs` 的 `mod tests` 內：

```rust
    /// 網頁版送的是「到目前為止的完整文字」，不是增量。直接轉發會讓
    /// 使用者看到重複累加的內容，必須自己算差分。
    #[test]
    fn snapshot_frames_become_incremental_deltas() {
        let mut p = SseParser::default();
        let a = p.feed_line(r#"data: {"message":{"content":{"parts":["你好"]}}}"#);
        let b = p.feed_line(r#"data: {"message":{"content":{"parts":["你好，世界"]}}}"#);
        assert_eq!(a, vec![SseOut::Text("你好".into())]);
        assert_eq!(b, vec![SseOut::Text("，世界".into())], "只能回新增的部分");
    }

    #[test]
    fn done_marker_ends_the_stream() {
        let mut p = SseParser::default();
        assert_eq!(p.feed_line("data: [DONE]"), vec![SseOut::Done]);
    }

    #[test]
    fn non_data_and_blank_lines_are_ignored() {
        let mut p = SseParser::default();
        assert!(p.feed_line("").is_empty());
        assert!(p.feed_line("event: delta").is_empty());
        assert!(p.feed_line(": keep-alive").is_empty());
    }

    /// 上游偶爾夾雜非 JSON 或缺欄位的 frame，不能因此中斷整條串流。
    #[test]
    fn malformed_frames_are_skipped_not_fatal() {
        let mut p = SseParser::default();
        assert!(p.feed_line("data: {不是 JSON").is_empty());
        assert!(p.feed_line(r#"data: {"message":null}"#).is_empty());
        assert_eq!(
            p.feed_line(r#"data: {"message":{"content":{"parts":["還活著"]}}}"#),
            vec![SseOut::Text("還活著".into())],
        );
    }

    /// 上游換一則新訊息時，這個 frame 不是前一個的延續。按 byte 位移切字串
    /// 會讓切點落在多位元組字元中間而 panic——中文內容下這不是理論風險。
    #[test]
    fn non_continuation_frame_does_not_panic_on_multibyte() {
        let mut p = SseParser::default();
        assert_eq!(
            p.feed_line(r#"data: {"message":{"content":{"parts":["ok"]}}}"#),
            vec![SseOut::Text("ok".into())],
        );
        // "ok" 是 2 bytes，"你好世界" 的 byte 2 在 '你' 的中間。
        assert_eq!(
            p.feed_line(r#"data: {"message":{"content":{"parts":["你好世界"]}}}"#),
            vec![SseOut::Text("你好世界".into())],
            "換訊息時要整段送出，不是 panic 也不是丟掉",
        );
    }

    /// 新訊息比舊的短時，用長度比較會把它整段靜默吃掉。
    #[test]
    fn shorter_new_message_is_not_swallowed() {
        let mut p = SseParser::default();
        p.feed_line(r#"data: {"message":{"content":{"parts":["很長的第一則回答"]}}}"#);
        assert_eq!(
            p.feed_line(r#"data: {"message":{"content":{"parts":["短"]}}}"#),
            vec![SseOut::Text("短".into())],
        );
    }

    /// 上游重送同一份快照時不該重複輸出。
    #[test]
    fn identical_resend_emits_nothing() {
        let mut p = SseParser::default();
        let frame = r#"data: {"message":{"content":{"parts":["你好"]}}}"#;
        assert_eq!(p.feed_line(frame), vec![SseOut::Text("你好".into())]);
        assert!(p.feed_line(frame).is_empty(), "重送不該再輸出一次");
    }
```

- [ ] **Step 2: 執行測試確認失敗**

```bash
cd src-tauri && cargo test --lib chatgpt_web::protocol
```

預期：`cannot find type SseParser` / `SseOut`

- [ ] **Step 3: 實作**

加到 `protocol.rs`（`flatten_history` 之後、`mod tests` 之前）：

```rust
/// 解析出的一個事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseOut {
    Text(String),
    Done,
}

/// ChatGPT 網頁版 SSE 的逐行解析器。
///
/// 每個 data frame 是「到目前為止的完整文字」快照而非增量，所以要保留
/// 已送出的長度自己算差分。
#[derive(Default)]
pub struct SseParser {
    emitted: String,
}

impl SseParser {
    pub fn feed_line(&mut self, line: &str) -> Vec<SseOut> {
        let Some(payload) = line.strip_prefix("data:") else {
            return Vec::new();
        };
        let payload = payload.trim();
        if payload == "[DONE]" {
            return vec![SseOut::Done];
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) else {
            return Vec::new();
        };
        let full = v
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.as_array())
            .and_then(|arr| arr.first())
            .and_then(|s| s.as_str())
            .unwrap_or("");
        // 用 strip_prefix 而非 `full[self.emitted.len()..]`：後者按 byte 切字串，
        // 當這個 frame 不是前一個的延續時（上游換了一則訊息），切點會落在多位元組
        // 字元的中間而 panic。實測：emitted="ok"、full="你好世界" 會炸在
        // 「byte index 2 is not a char boundary」。中文內容下這不是理論風險。
        match full.strip_prefix(self.emitted.as_str()) {
            // 沒有新增內容（上游重送同一份快照）。
            Some("") => Vec::new(),
            // 正常的累積快照：只回新增的那一段。
            Some(delta) => {
                let delta = delta.to_string();
                self.emitted = full.to_string();
                vec![SseOut::Text(delta)]
            }
            // 不是延續——上游換了一則訊息。重新起算並把整段當新內容送出。
            // 不能沿用「比較長度」的判斷：新訊息比舊的短時會被整段靜默吃掉。
            None => {
                self.emitted = full.to_string();
                if full.is_empty() {
                    Vec::new()
                } else {
                    vec![SseOut::Text(full.to_string())]
                }
            }
        }
    }
}
```

- [ ] **Step 4: 執行測試確認通過**

```bash
cd src-tauri && cargo test --lib chatgpt_web::protocol
```

預期：12 個測試全 PASS（Task 2 的 5 個 + 本任務的 7 個）

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/chatgpt_web/protocol.rs
git commit -m "feat(chatgpt-web): SSE 快照轉增量差分"
```

---

## Task 4：工具契約序列化

**Files:**
- Create: `src-tauri/src/chatgpt_web/tools.rs`
- Modify: `src-tauri/src/chatgpt_web/mod.rs`（加 `pub mod tools;`）

> ⚠️ **先做這件事，否則整個 TDD 循環是假的**：在 `src-tauri/src/chatgpt_web/mod.rs`
> 加上 `pub mod tools;`。Task 1 並**沒有**預先宣告它（宣告不存在的模組會編譯失敗），
> Task 2 只補了 `pub mod protocol;`。
>
> Rust 對「模組目錄下存在但未被宣告的 `.rs` 檔」是**靜默不編譯**，無錯誤也無警告。
> 漏掉這步的話：Step 2 期待的編譯失敗不會出現，你只會看到 `0 tests`；Step 4 也不會
> 是「3 個測試全 PASS」，而是 `0 passed` 且 **exit code 0**——看起來像通過。
> `tools.rs` 會一路沒被編譯過，直到 Task 12／13 才爆。

- [ ] **Step 1: 寫失敗的測試**

`src-tauri/src/chatgpt_web/tools.rs` 結尾：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(name: &str) -> crate::ai::McpToolDefinition {
        crate::ai::McpToolDefinition {
            name: name.into(),
            description: format!("{name} 的說明"),
            input_schema: json!({"type": "object"}),
        }
    }

    #[test]
    fn contract_lists_every_tool_and_embeds_the_nonce() {
        let c = build_contract(&[tool("Read"), tool("Edit")], "abc123");
        assert!(c.contains("Read"));
        assert!(c.contains("Edit"));
        assert!(c.contains("abc123"), "nonce 要出現在契約裡");
        assert!(c.contains("_nonce"), "要明確告訴模型欄位名");
    }

    #[test]
    fn empty_tool_list_produces_no_contract() {
        assert_eq!(build_contract(&[], "abc123"), "");
    }

    /// 依據 OmniRoute #7679 的實測：契約 prepend 在巨大 system 區塊開頭時，
    /// 30K 字元 prompt 下模型會直接忽略它（0/3），雙位置為 16/17。
    #[test]
    fn reminder_is_short_and_names_the_tools() {
        let r = build_reminder(&[tool("Read"), tool("Edit")]);
        assert!(r.contains("Read") && r.contains("Edit"), "要點名工具");
        assert!(r.len() < 400, "只是指回 system 區塊的一行提示，不是完整契約：{}", r.len());
        assert!(!r.contains("_nonce"), "nonce 只放在完整契約，避免重複洩漏");
    }
}
```

- [ ] **Step 2: 執行測試確認失敗**

```bash
cd src-tauri && cargo test --lib chatgpt_web::tools
```

預期：`cannot find function build_contract`

- [ ] **Step 3: 實作**

`src-tauri/src/chatgpt_web/tools.rs` 開頭：

```rust
//! 工具呼叫的 prompt 模擬：契約序列化與封套剖析。
//!
//! ChatGPT 網頁版沒有原生 function calling。做法是在 prompt 裡給模型一份
//! 契約，要它用 `<tool>{…}</tool>` 封套回覆，再從回覆文字剖析回結構。

use crate::ai::McpToolDefinition;

/// 完整契約。放在客戶端訊息**之後**（executor 摺疊 system 訊息後會落在
/// 區塊尾端）。
///
/// 依據 OmniRoute #7679 的實測：prepend 在巨大 system 區塊開頭時，30K 字元
/// prompt 下 chatgpt-web 會回答「tool X is not in my tool set」，成功率 0/3；
/// 改成尾端 + user 訊息提醒的雙位置為 16/17。Claude Code 正是那個形狀。
pub fn build_contract(tools: &[McpToolDefinition], nonce: &str) -> String {
    if tools.is_empty() {
        return String::new();
    }
    let mut lines = vec![
        "The client application provides tools beyond your built-in ones. They are NOT in \
         your native tool registry; they are invoked via a plain-text protocol: the client \
         parses your reply and executes the tool on the user machine. Treat these client \
         tools as fully available to you; never claim they are unavailable."
            .to_string(),
        format!(
            "To invoke one, reply with a single line containing a <tool> block whose JSON \
             includes the secret binding \"_nonce\": \"{nonce}\":"
        ),
        format!(r#"<tool>{{"name": "<tool_name>", "arguments": {{ ... }}, "_nonce": "{nonce}"}}</tool>"#),
        "Only emit the <tool> block when you actually want to call a tool; otherwise answer \
         normally."
            .to_string(),
        String::new(),
        "Available tools:".to_string(),
    ];
    for t in tools {
        lines.push(format!(
            "- {}: {}\n  parameters: {}",
            t.name, t.description, t.input_schema
        ));
    }
    lines.join("\n")
}

/// 掛在最新一則 user 訊息末尾的一行提醒。刻意簡短：網頁版模型對當前 user
/// 回合的權重遠高於龐大的 system 區塊，而 ChatGPT 的注入偵測又不信任藏在
/// user 內容裡的長指令，所以完整契約留在 system 尾端，這裡只指回去並點名工具。
pub fn build_reminder(tools: &[McpToolDefinition]) -> String {
    if tools.is_empty() {
        return String::new();
    }
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    format!(
        "\n\n[Client protocol reminder: the client-tool contract in the system instructions \
         is active in this conversation. These client tools ARE available via the <tool> \
         block protocol: {}.]",
        names.join(", ")
    )
}
```

- [ ] **Step 4: 執行測試確認通過**

```bash
cd src-tauri && cargo test --lib chatgpt_web::tools
```

預期：3 個測試全 PASS

- [ ] **Step 5: 確認測試數字不是零**

上面那句「3 個測試全 PASS」要當成斷言看：若輸出是 `0 passed`，代表 `pub mod tools;`
沒加成功（見本任務開頭的警告），**不要**當作通過。

```bash
cd src-tauri && cargo check
```

預期：`Finished`

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/chatgpt_web/tools.rs src-tauri/src/chatgpt_web/mod.rs
git commit -m "feat(chatgpt-web): 工具契約序列化，雙位置注入"
```

---

## Task 5：封套剖析與 nonce 驗證

**Files:**
- Modify: `src-tauri/src/chatgpt_web/tools.rs`

- [ ] **Step 1: 寫失敗的測試**

加到 `tools.rs` 的 `mod tests` 內：

```rust
    #[test]
    fn parses_tool_block_and_strips_it_from_text() {
        let text = r#"我來讀檔。<tool>{"name":"Read","arguments":{"path":"a.txt"},"_nonce":"n1"}</tool>"#;
        let (content, calls) = parse_tool_calls(text, "n1");
        let calls = calls.expect("應該剖析出工具呼叫");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "Read");
        assert_eq!(calls[0].args, serde_json::json!({"path":"a.txt"}));
        assert!(!content.contains("<tool>"), "封套要從內容剝掉，實際：{content}");
        assert!(content.contains("我來讀檔"));
    }

    #[test]
    fn accepts_the_alternate_tool_call_tag() {
        let text = r#"<tool_call name="ignored">{"name":"Edit","arguments":{},"_nonce":"n1"}</tool_call>"#;
        let (_, calls) = parse_tool_calls(text, "n1");
        let calls = calls.expect("應該剖析出工具呼叫");
        assert_eq!(calls[0].tool_name, "Edit", "名稱以 JSON 內的為準，不看標籤屬性");
    }

    /// OmniRoute #9343：使用者貼進來的內容或程式碼若含 name+arguments 的裸
    /// JSON，舊版會直接當成工具呼叫執行。
    #[test]
    fn bare_json_is_never_promoted_to_a_tool_call() {
        let text = r#"這是範例：{"name":"Bash","arguments":{"command":"rm -rf /"}}"#;
        let (content, calls) = parse_tool_calls(text, "n1");
        assert!(calls.is_none(), "裸 JSON 不可升級成工具呼叫");
        assert!(content.contains("rm -rf /"), "原文要原樣保留");
    }

    #[test]
    fn wrong_nonce_is_treated_as_text() {
        let text = r#"<tool>{"name":"Read","arguments":{},"_nonce":"別人的"}</tool>"#;
        let (_, calls) = parse_tool_calls(text, "n1");
        assert!(calls.is_none(), "nonce 不符要當成文字，不可執行");
    }

    /// 被拒絕的封套要連標籤一起原樣留在內容裡。使用者貼一段含 `<tool>` 的
    /// 範例進來時，回覆裡他的原文不該被改掉——而這正是 nonce 檢查要處理的
    /// 主要情境。
    #[test]
    fn rejected_envelope_is_preserved_verbatim_with_its_tags() {
        let text = r#"看這段：<tool>{"name":"Read","arguments":{},"_nonce":"別人的"}</tool>就這樣"#;
        let (content, calls) = parse_tool_calls(text, "n1");
        assert!(calls.is_none());
        assert_eq!(content, text, "拒絕路徑要原樣保留，包含 <tool> 與 </tool>");
    }

    /// 封套內不是合法 JSON 時同樣要原樣保留。
    #[test]
    fn malformed_envelope_body_is_preserved_verbatim() {
        let text = "前面<tool>{不是 JSON}</tool>後面";
        let (content, calls) = parse_tool_calls(text, "n1");
        assert!(calls.is_none());
        assert_eq!(content, text);
    }

    /// 模型可能不遵守指示而漏掉 _nonce。封套本身已是明確意圖，容忍。
    #[test]
    fn missing_nonce_is_tolerated() {
        let text = r#"<tool>{"name":"Read","arguments":{}}</tool>"#;
        let (_, calls) = parse_tool_calls(text, "n1");
        assert!(calls.is_some(), "缺 _nonce 應容忍");
    }

    #[test]
    fn plain_text_yields_no_calls() {
        let (content, calls) = parse_tool_calls("就只是一段回答", "n1");
        assert!(calls.is_none());
        assert_eq!(content, "就只是一段回答");
    }
```

- [ ] **Step 2: 執行測試確認失敗**

```bash
cd src-tauri && cargo test --lib chatgpt_web::tools
```

預期：`cannot find function parse_tool_calls`

- [ ] **Step 3: 實作**

加到 `tools.rs`：

```rust
use crate::ai::AiToolCall;

/// 從模型回覆剖析出工具呼叫，回傳（剝掉封套的內容, 工具呼叫）。
///
/// **只接受明確封套**（`<tool>` 或 `<tool_call>`），不把裸 JSON 升級成工具
/// 呼叫。依據 OmniRoute #9343：使用者貼進來的內容或程式碼若含 name+arguments
/// 的裸 JSON，會被當成真的工具呼叫執行——這是 prompt injection。
///
/// nonce 有帶但不符 → 視為文字；完全沒帶 → 容忍（模型未遵守指示，但封套
/// 本身已是明確意圖）。
pub fn parse_tool_calls(text: &str, nonce: &str) -> (String, Option<Vec<AiToolCall>>) {
    let mut calls = Vec::new();
    let mut content = String::new();
    let mut rest = text;

    // 被拒絕的封套要連同 `<tool>` / `</tool>` 標籤一起推回 content（用 `raw`
    // 而非 `body`）。只推回 body 會把使用者貼進來的原文改掉——而「使用者貼了
    // 一段含封套的範例」正是 nonce 檢查要處理的主要情境。
    while let Some((before, body, raw, after)) = next_envelope(rest) {
        content.push_str(before);
        rest = after;
        let Ok(v) = serde_json::from_str::<serde_json::Value>(body.trim()) else {
            // 封套內不是合法 JSON——原樣保留，別吞掉使用者看得到的內容。
            content.push_str(raw);
            continue;
        };
        if let Some(got) = v.get("_nonce").and_then(|n| n.as_str()) {
            if got != nonce {
                content.push_str(raw);
                continue;
            }
        }
        let Some(name) = v.get("name").and_then(|n| n.as_str()) else {
            content.push_str(raw);
            continue;
        };
        calls.push(AiToolCall {
            id: format!("call_{}", calls.len()),
            tool_name: name.to_string(),
            args: v.get("arguments").cloned().unwrap_or(serde_json::json!({})),
            thought_signature: None,
        });
    }
    content.push_str(rest);

    (content, if calls.is_empty() { None } else { Some(calls) })
}

/// 找出下一個封套，回傳（封套前的文字, 封套內容, 封套原文, 封套後的剩餘文字）。
///
/// 「封套原文」含 `<tool>` / `</tool>` 標籤本身，給拒絕路徑原樣推回用。
fn next_envelope(text: &str) -> Option<(&str, &str, &str, &str)> {
    const PAIRS: [(&str, &str); 2] = [("<tool>", "</tool>"), ("<tool_call", "</tool_call>")];
    // (起點, 內容起點, 內容終點, 封套終點)
    let mut best: Option<(usize, usize, usize, usize)> = None;
    for (open, close) in PAIRS {
        let Some(start) = text.find(open) else { continue };
        // `<tool_call name="…">` 的屬性要跳過，內容從 '>' 之後開始。
        let Some(gt) = text[start..].find('>').map(|i| start + i + 1) else { continue };
        let Some(end) = text[gt..].find(close).map(|i| gt + i) else { continue };
        // 用 map_or 而非 is_none_or：後者是 Rust 1.82 才穩定，而 Cargo.toml
        // 宣告的 rust-version 是 1.77.2，且整個 codebase 沒有用過它。
        if best.map_or(true, |(b, _, _, _)| start < b) {
            best = Some((start, gt, end, end + close.len()));
        }
    }
    let (start, body_start, body_end, env_end) = best?;
    Some((
        &text[..start],
        &text[body_start..body_end],
        &text[start..env_end],
        &text[env_end..],
    ))
}
```

- [ ] **Step 4: 執行測試確認通過**

```bash
cd src-tauri && cargo test --lib chatgpt_web::tools
```

預期：11 個測試全 PASS（Task 4 的 3 個 + 本任務的 8 個）。
若看到 `0 passed`，是 `pub mod tools;` 沒宣告——見 Task 4 開頭的警告。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/chatgpt_web/tools.rs
git commit -m "feat(chatgpt-web): 封套剖析與 nonce 驗證，拒絕裸 JSON"
```

---
## Task 6：注入腳本的 SHA3-512

sentinel 的工作量證明需要 SHA3-512，而 WebCrypto 沒有提供，必須自帶實作。
Keccak 極易寫錯，所以先用標準測試向量鎖住。

**Files:**
- Create: `src-tauri/src/chatgpt_web/inject.js`
- Create: `src/lib/chatgptWebInject.test.ts`

- [ ] **Step 1: 寫失敗的測試**

`src/lib/chatgptWebInject.test.ts`：

```ts
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

/**
 * inject.js 是給瀏覽器用的 IIFE，直接 import 會找不到 window。這裡把它讀進來、
 * 在一個假的 window 上求值，再取出掛上去的純函式來測。
 */
function loadInject(): Record<string, unknown> {
  const src = readFileSync("src-tauri/src/chatgpt_web/inject.js", "utf8");
  const win: Record<string, unknown> = {};
  new Function("window", src)(win);
  return win;
}

describe("inject.js SHA3-512", () => {
  const { __aitermTest } = loadInject() as { __aitermTest: { sha3_512Hex(s: string): string } };

  it("符合標準測試向量", () => {
    expect(__aitermTest.sha3_512Hex("")).toBe(
      "a69f73cca23a9ac5c8b567dc185a756e97c982164fe25859e0d1dcc1475c80a6" +
      "15b2123af1f5f94c11e3e9402c3ac558f500199d95b6d3e301758586281dcd26",
    );
    expect(__aitermTest.sha3_512Hex("abc")).toBe(
      "b751850b1a57168a5693cd924b6b096e08f621827444f70d884f5d0240d2712e" +
      "10e116e9192af3c91a7ec57647e3934057340b4cf408d5a56592f8274eec53f0",
    );
  });
});
```

- [ ] **Step 2: 執行測試確認失敗**

```bash
npx vitest run src/lib/chatgptWebInject.test.ts
```

預期：FAIL，找不到 `src-tauri/src/chatgpt_web/inject.js`

- [ ] **Step 3: 實作**

建立 `src-tauri/src/chatgpt_web/inject.js`。以 `(window) => { … }` 的形式接受注入的 window，末端把純函式掛到 `window.__aitermTest` 供測試取用。

SHA3-512 的實作內容：Keccak-f[1600]，rate 72 bytes，以 32 位元 lo/hi 對表示 64 位元字（避免 BigInt 的效能問題）。**旋轉偏移表必須是 25 個元素**，以 lane index `x + 5y` 排列：

```js
const ROT = [0,1,62,28,27, 36,44,6,55,20, 3,10,43,25,39, 41,45,15,21,8, 18,2,61,56,14];
```

（探勘時這張表多寫了一個 `25` 變成 26 個元素，兩個測試向量都不過。）

24 個 round constant 以 lo/hi 對表示，第一個是 `[0x00000001, 0x00000000]`，最後一個是 `[0x80008008, 0x80000000]`。

吸收階段：每 72 bytes 一區塊，補 `0x06`、末位元組 `|= 0x80`，逐 8 bytes 以 little-endian XOR 進狀態後跑一次 `keccakf()`。擠出階段：取前 8 個 lane（64 bytes）轉十六進位。

完整可用的參考實作**就在當前分支**的 `src-tauri/src/probe_chatgpt.rs`，`sha3_512Hex`
從第 83 行開始（那個檔案是探勘用的 Rust，SHA3 以一段內嵌的 JS 字串存在）。該實作已
用同樣的測試向量實測通過，直接搬過來即可，不要重寫。同一份檔案裡的 `solvePow`
（約第 165 行起）是 Task 7 要用的，這次先不動。

（沒有 `probe/chatgpt-web` 這個分支——探勘程式碼是以 commit `99d9c72` 直接留在
`feat/chatgpt-web-provider` 上的，Task 16 才會連同它一起移除。）

- [ ] **Step 4: 執行測試確認通過**

```bash
npx vitest run src/lib/chatgptWebInject.test.ts
```

預期：PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/chatgpt_web/inject.js src/lib/chatgptWebInject.test.ts
git commit -m "feat(chatgpt-web): 注入腳本的 SHA3-512，鎖標準測試向量"
```

---

## Task 7：PoW solver 與 config 組裝

**Files:**
- Modify: `src-tauri/src/chatgpt_web/inject.js`
- Modify: `src/lib/chatgptWebInject.test.ts`

- [ ] **Step 1: 寫失敗的測試**

加到 `src/lib/chatgptWebInject.test.ts`：

```ts
describe("inject.js PoW", () => {
  const { __aitermTest } = loadInject() as {
    __aitermTest: {
      sha3_512Hex(s: string): string;
      buildConfig(): unknown[];
      solvePow(seed: string, target: string, prefix: string, maxIter: number):
        { token: string; iters: number; exhausted?: boolean };
    };
  };

  it("解出的 token 前綴正確且雜湊真的落在目標之下", () => {
    const r = __aitermTest.solvePow("seed", "0fffff", "gAAAAAC", 100000);
    expect(r.exhausted).toBeFalsy();
    expect(r.token.startsWith("gAAAAAC")).toBe(true);
    const encoded = r.token.slice("gAAAAAC".length);
    expect(__aitermTest.sha3_512Hex("seed" + encoded).slice(0, 6) <= "0fffff").toBe(true);
  });

  it("超過上限時回 exhausted 而不是無限跑", () => {
    // 目標 "000000" 幾乎不可能命中，用極小的 maxIter 逼出這條路徑。
    const r = __aitermTest.solvePow("seed", "000000", "gAAAAAB", 5);
    expect(r.exhausted).toBe(true);
    expect(r.iters).toBe(5);
    expect(r.token.startsWith("gAAAAAB")).toBe(true);
  });

  it("config 是 18 元素，第 4 格由 solver 改寫", () => {
    const c = __aitermTest.buildConfig();
    expect(c).toHaveLength(18);
    expect(c[3]).toBe(0);
  });
});
```

- [ ] **Step 2: 執行測試確認失敗**

```bash
npx vitest run src/lib/chatgptWebInject.test.ts
```

預期：FAIL，`__aitermTest.solvePow is not a function`

- [ ] **Step 3: 實作**

加到 `inject.js`：

```js
  // config 需要的瀏覽器特徵在頁面內全是真值。OmniRoute 跑在伺服器上必須捏造
  // （假螢幕尺寸、假核心數、從硬編清單隨機挑 key），我們送出的指紋則與 OpenAI
  // 看到的其他一切一致——這是把 webview 當傳輸層的額外好處。
  const buildConfig = () => {
    const dplAttr = doc().documentElement.getAttribute("data-build");
    const script = doc().querySelector('script[src*=".js"]');
    const perfNow = perf().now();
    const nav = navigatorRef();
    return [
      scr().width + scr().height,
      new Date().toString(),
      4294705152,
      0, // solver 改寫這一格
      nav.userAgent,
      script ? script.src : "",
      dplAttr ? "dpl=" + dplAttr : "",
      nav.language,
      (nav.languages || []).join(","),
      0,
      pickKey(nav),
      pickKey(doc()),
      pickKey(win),
      perfNow,
      uuid(),
      "",
      nav.hardwareConcurrency,
      Date.now() - perfNow,
    ];
  };

  const b64 = (obj) => btoa(unescape(encodeURIComponent(JSON.stringify(obj))));

  // 把 config[3] 換成遞增計數器，算 SHA3-512(seed + base64(JSON(config)))，
  // 取十六進位前綴與 difficulty 做字串比較。實測 difficulty 是 "06b931" 這種
  // 6 位值，命中機率約 2.6%，平均數十次即可。
  const solvePow = (seed, target, prefix, maxIter) => {
    const cfg = buildConfig();
    for (let i = 0; i < maxIter; i++) {
      cfg[3] = i;
      const enc = b64(cfg);
      if (sha3_512Hex(seed + enc).slice(0, target.length) <= target) {
        return { token: prefix + enc, iters: i + 1 };
      }
    }
    return { token: prefix + b64(cfg), iters: maxIter, exhausted: true };
  };
```

`doc()` / `scr()` / `navigatorRef()` / `perf()` / `win` / `uuid()` / `pickKey()` 是為了讓腳本在 vitest 的假 window 下也能求值而加的取值輔助：真實環境回瀏覽器全域，測試環境回可用的替身。`pickKey(obj)` 從 `Object.keys(obj)` 隨機取一個，空物件回空字串。

- [ ] **Step 4: 執行測試確認通過**

```bash
npx vitest run src/lib/chatgptWebInject.test.ts
```

預期：全部 PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/chatgpt_web/inject.js src/lib/chatgptWebInject.test.ts
git commit -m "feat(chatgpt-web): PoW solver 與真實瀏覽器特徵的 config"
```

---

## Task 8：注入腳本的認證、sentinel 與串流

這一段沒有單元測試——它全是網路 I/O，只能靠 Task 16 的端到端探勘測試驗證。

**Files:**
- Modify: `src-tauri/src/chatgpt_web/inject.js`

- [ ] **Step 1: 實作認證**

```js
  // backend-api 認的是 Bearer access token，不是 cookie。少了它，sentinel 回應
  // 的 persona 會是 "chatgpt-noauth"，對話請求則是 403 "Unusual activity has
  // been detected from your device"（實測）。
  let accessToken = null;
  const ensureAuth = async () => {
    if (accessToken) return accessToken;
    const r = await fetch("/api/auth/session", { headers: { accept: "application/json" } });
    const j = await r.json().catch(() => ({}));
    accessToken = j.accessToken || null;
    return accessToken;
  };
  const authHeaders = () => ({
    "content-type": "application/json",
    ...(accessToken ? { authorization: "Bearer " + accessToken } : {}),
  });
```

- [ ] **Step 2: 實作 sentinel 兩段流程**

```js
  // 兩段 chat-requirements：prepare 拿 prepare_token，再換取對話用的 token
  // 與 proofofwork 參數。兩段的 p 都是解過的 PoW token（prepare 階段
  // seed 為空、target 固定 "0fffff"）。
  const sentinel = async () => {
    const post = (url, body) =>
      fetch(url, { method: "POST", headers: authHeaders(), body: JSON.stringify(body) });

    const prep = await post("/backend-api/sentinel/chat-requirements/prepare",
                            { p: solvePow("", "0fffff", "gAAAAAC", 100000).token });
    if (!prep.ok) throw new Error("sentinel prepare " + prep.status);
    const prepJson = await prep.json();

    const cr = await post("/backend-api/sentinel/chat-requirements",
                          { p: solvePow("", "0fffff", "gAAAAAC", 100000).token,
                            prepare_token: prepJson.prepare_token });
    if (!cr.ok) throw new Error("sentinel chat-requirements " + cr.status);
    const crJson = await cr.json();
    return { ...crJson, prepare_token: prepJson.prepare_token };
  };
```

- [ ] **Step 3: 實作請求主流程**

```js
  // Rust 端只送 id，payload 由這裡反向拉取——Claude Code 的 system prompt
  // 動輒 30K 字元，用 eval 拼進 JS 字串會踩上跳脫與長度限制。
  window.__aiterm = {
    pull: async (id) => {
      try {
        const payload = await invoke("chatgpt_web_take", { id });
        await run(id, payload);
      } catch (e) {
        invoke("chatgpt_web_chunk", { id, data: JSON.stringify({ error: String(e) }) });
      }
    },
  };

  const run = async (id, payload) => {
    if (!(await ensureAuth())) throw new Error("not_logged_in");
    const reqs = await sentinel();
    const pow = reqs.proofofwork || {};
    const headers = { ...authHeaders(), accept: "text/event-stream" };
    if (reqs.token) headers["openai-sentinel-chat-requirements-token"] = reqs.token;
    if (reqs.prepare_token) headers["openai-sentinel-chat-requirements-prepare-token"] = reqs.prepare_token;
    headers["openai-sentinel-proof-token"] =
      solvePow(pow.seed || "", (pow.difficulty || "").toLowerCase(), "gAAAAAB", 500000).token;

    const r = await fetch("/backend-api/conversation", {
      method: "POST",
      headers,
      body: JSON.stringify({
        action: "next",
        messages: [{ id: uuid(), author: { role: "user" },
                     content: { content_type: "text", parts: [payload.text] } }],
        model: payload.model,
        parent_message_id: uuid(),
        websocket_request_id: uuid(),
        conversation_mode: { kind: "primary_assistant" },
      }),
    });
    if (!r.ok || !r.body) {
      const body = await r.text();
      invoke("chatgpt_web_chunk", { id, data: JSON.stringify({ error: body, status: r.status }) });
      return;
    }
    const reader = r.body.getReader();
    const dec = new TextDecoder();
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      invoke("chatgpt_web_chunk", { id, data: dec.decode(value, { stream: true }) });
    }
    invoke("chatgpt_web_chunk", { id, data: "data: [DONE]\n\n" });
  };
```

- [ ] **Step 4: 確認既有測試沒被打壞**

```bash
npx vitest run src/lib/chatgptWebInject.test.ts
```

預期：Task 6、7 的測試仍全 PASS（新增的都是 I/O，不影響純函式）

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/chatgpt_web/inject.js
git commit -m "feat(chatgpt-web): 注入腳本的認證、sentinel 與串流"
```

---

## Task 9：Session 與 Tauri command

**Files:**
- Create: `src-tauri/src/chatgpt_web/session.rs`
- Create: `src-tauri/capabilities/chatgpt-web.json`
- Modify: `src-tauri/src/chatgpt_web/mod.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 寫失敗的測試**

`src-tauri/src/chatgpt_web/session.rs` 結尾（只測不需要 Tauri 執行期的配對邏輯）：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_returns_payload_once_then_none() {
        let p = PendingMap::default();
        p.insert("id1".into(), "payload".into());
        assert_eq!(p.take("id1"), Some("payload".to_string()));
        assert_eq!(p.take("id1"), None, "同一個 id 不可被取用兩次");
    }

    #[test]
    fn unknown_id_takes_as_none() {
        let p = PendingMap::default();
        assert_eq!(p.take("沒看過的"), None);
    }
}
```

- [ ] **Step 2: 執行測試確認失敗**

```bash
cd src-tauri && cargo test --lib chatgpt_web::session
```

預期：`cannot find type PendingMap`

- [ ] **Step 3: 實作 session.rs**

```rust
//! webview 生命週期與請求配對。
//!
//! webview **就是傳輸層本身**：`hide()` 保留它、請求照常；`close()` 銷毀它、
//! 下次請求自動重建。登入狀態不受影響——Tauri 的 webview 資料儲存是應用程式
//! 層級，因此**必須用預設儲存，不可用隔離分割區**。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::mpsc;

/// 待取用的請求 payload。JS 端以 id 反向拉取。
#[derive(Default)]
pub struct PendingMap(Mutex<HashMap<String, String>>);

impl PendingMap {
    pub fn insert(&self, id: String, payload: String) {
        self.0.lock().expect("pending map poisoned").insert(id, payload);
    }
    /// 取出並移除。同一個 id 只能被取用一次——重複取用代表 JS 端有重入問題，
    /// 回 None 讓它明確失敗好過送出重複請求。
    pub fn take(&self, id: &str) -> Option<String> {
        self.0.lock().expect("pending map poisoned").remove(id)
    }
}

pub struct Session {
    app: tauri::AppHandle,
    pending: PendingMap,
    sinks: Mutex<HashMap<String, mpsc::UnboundedSender<String>>>,
}

/// 全域存取點。`AiRouter` 沒有 `AppHandle`，橋接也在另一條路徑上，
/// 兩者都需要同一個 Session，因此在 setup 時初始化一次。
static SESSION: OnceLock<Arc<Session>> = OnceLock::new();

pub fn init(app: tauri::AppHandle) {
    let _ = SESSION.set(Arc::new(Session {
        app,
        pending: PendingMap::default(),
        sinks: Mutex::new(HashMap::new()),
    }));
}

pub fn get() -> Option<Arc<Session>> {
    SESSION.get().cloned()
}
```

`Session` 的方法（`ensure_window`、`request`）與兩個 command 在後續步驟加入。

- [ ] **Step 4: 執行測試確認通過**

```bash
cd src-tauri && cargo test --lib chatgpt_web::session
```

預期：2 個測試 PASS

- [ ] **Step 5: 加入視窗管理與請求送出**

在 `impl Session` 內加入：

```rust
impl Session {
    const WINDOW_LABEL: &'static str = "chatgpt-web";

    /// 確保視窗存在。已存在就沿用（保留登入狀態），不存在才建立。
    /// `visible` 為 true 時顯示出來讓使用者登入。
    pub fn ensure_window(&self, visible: bool) -> Result<tauri::WebviewWindow, tauri::Error> {
        use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
        if let Some(w) = self.app.get_webview_window(Self::WINDOW_LABEL) {
            if visible { let _ = w.show(); }
            return Ok(w);
        }
        let url = "https://chatgpt.com/".parse().expect("static url");
        let w = WebviewWindowBuilder::new(&self.app, Self::WINDOW_LABEL, WebviewUrl::External(url))
            .title("ChatGPT")
            .inner_size(1100.0, 850.0)
            .visible(visible)
            .initialization_script(include_str!("inject.js"))
            .build()?;
        Ok(w)
    }

    /// 送出一個請求，回傳接收 chunk 的通道與一個清理守衛。
    ///
    /// **守衛一定要綁在具名變數上**（`let (mut rx, _guard) = …`）。綁成 `_`
    /// 會當場 drop，sink 立刻被移除，接下來收不到任何 chunk。
    pub fn request(
        self: &Arc<Self>,
        payload: String,
    ) -> Result<(mpsc::UnboundedReceiver<String>, SinkGuard), tauri::Error> {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = mpsc::unbounded_channel();
        self.pending.insert(id.clone(), payload);
        self.sinks.lock().expect("sinks poisoned").insert(id.clone(), tx);
        let guard = SinkGuard { session: Arc::clone(self), id: id.clone() };
        let w = self.ensure_window(false)?;
        // 只送 id，payload 由 JS 反向拉取。
        w.eval(format!("window.__aiterm.pull({})", serde_json::json!(id)))?;
        Ok((rx, guard))
    }

    pub fn take_pending(&self, id: &str) -> Option<String> {
        self.pending.take(id)
    }

    pub fn push_chunk(&self, id: &str, data: String) {
        if let Some(tx) = self.sinks.lock().expect("sinks poisoned").get(id) {
            let _ = tx.send(data);
        }
    }
}

/// 請求結束時把 sink 從 map 移掉。
///
/// 沒有它的話 `sinks` 會永遠握著一份 sender clone，`rx.recv().await` 永遠不會
/// 回 `None`——任何靠「通道關閉」收尾的消費端都會永久卡住（UI 一直轉圈、
/// 沒有錯誤訊息）。用 `Drop` 而不是在每個結束點手動移除，是為了同時涵蓋
/// 錯誤路徑與提早 `return`。
pub struct SinkGuard {
    session: Arc<Session>,
    id: String,
}

impl Drop for SinkGuard {
    fn drop(&mut self) {
        self.session.sinks.lock().expect("sinks poisoned").remove(&self.id);
        // JS 若因故沒來拉取，payload 也不該留著。
        let _ = self.session.pending.take(&self.id);
    }
}
```

- [ ] **Step 6: 加入兩個 command**

```rust
#[tauri::command]
pub async fn chatgpt_web_take(id: String) -> Result<Option<String>, String> {
    Ok(get().and_then(|s| s.take_pending(&id)))
}

#[tauri::command]
pub async fn chatgpt_web_chunk(id: String, data: String) {
    if let Some(s) = get() {
        s.push_chunk(&id, data);
    }
}
```

**兩個 command 都必須是 `async`**：Tauri 文件記載，在同步 command 內接觸 webview 相關資源會在 Windows 上死鎖（wry #583）。

- [ ] **Step 7: 建立 capability 並註冊**

`src-tauri/capabilities/chatgpt-web.json`：

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "chatgpt-web",
  "description": "只讓 chatgpt-web 視窗裡的 chatgpt.com 頁面能用 IPC 回傳串流內容。",
  "windows": ["chatgpt-web"],
  "local": false,
  "remote": { "urls": ["https://chatgpt.com/*"] },
  "permissions": ["core:event:default"]
}
```

`src-tauri/src/chatgpt_web/mod.rs` 加入 `pub mod session;`。

`src-tauri/src/lib.rs` 的 `generate_handler!` 內加入：

```rust
            chatgpt_web::session::chatgpt_web_take,
            chatgpt_web::session::chatgpt_web_chunk,
```

`src-tauri/src/lib.rs` 的 `.setup(|app| {` 內加入：

```rust
            chatgpt_web::session::init(app.handle().clone());
```

- [ ] **Step 8: 確認編譯與測試**

```bash
cd src-tauri && cargo check && cargo test --lib chatgpt_web
```

預期：`Finished`，測試全 PASS

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/chatgpt_web/session.rs src-tauri/src/chatgpt_web/mod.rs \
        src-tauri/capabilities/chatgpt-web.json src-tauri/src/lib.rs
git commit -m "feat(chatgpt-web): Session、視窗管理與 IPC command"
```

---

## Task 10：登入偵測與自動隱藏

**Files:**
- Modify: `src-tauri/src/chatgpt_web/session.rs`
- Modify: `src-tauri/src/chatgpt_web/inject.js`

- [ ] **Step 1: 在注入腳本加入登入狀態回報**

```js
  // 顯示視窗期間輪詢登入狀態。拿到 token 就通知 Rust 收起視窗——不做這件事
  // 視窗會一直開著，使用者很自然會去關掉它，之後每次請求都要多付一次載入成本。
  window.__aitermWatchLogin = () => {
    const tick = async () => {
      accessToken = null;               // 強制重新查，別用登入前的快取值
      if (await ensureAuth()) {
        invoke("chatgpt_web_logged_in", {});
        return;
      }
      setTimeout(tick, 2000);
    };
    tick();
  };
```

- [ ] **Step 2: 加入對應的 command**

`session.rs`：

```rust
/// 注入腳本偵測到登入完成時呼叫。收起視窗即可——傳輸層繼續存活。
#[tauri::command]
pub async fn chatgpt_web_logged_in() {
    if let Some(s) = get() {
        if let Some(w) = {
            use tauri::Manager;
            s.app.get_webview_window(Session::WINDOW_LABEL)
        } {
            let _ = w.hide();
        }
    }
}
```

在 `lib.rs` 的 `generate_handler!` 註冊 `chatgpt_web::session::chatgpt_web_logged_in`。

- [ ] **Step 3: 在 `ensure_window(true)` 後啟動輪詢**

`Session::ensure_window` 的 `visible` 分支加上：

```rust
        if visible {
            let _ = w.eval("window.__aitermWatchLogin && window.__aitermWatchLogin()");
        }
```

- [ ] **Step 4: 確認編譯**

```bash
cd src-tauri && cargo check && cargo test --lib chatgpt_web
```

預期：`Finished`，測試全 PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/chatgpt_web/session.rs src-tauri/src/chatgpt_web/inject.js
git commit -m "feat(chatgpt-web): 登入完成後自動隱藏視窗"
```

---
## Task 11：AiProvider 的 generate 與 health_check

**Files:**
- Create: `src-tauri/src/ai/chatgpt_web.rs`
- Modify: `src-tauri/src/ai/mod.rs`
- Modify: `src-tauri/src/ai/router.rs`

- [ ] **Step 1: 寫失敗的測試**

`src-tauri/src/ai/chatgpt_web.rs` 結尾（只測不需要 webview 的請求組裝）：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{ChatMessage, EnvSnapshot, QueryMode};

    fn req(system: &str, msgs: &[(&str, &str)]) -> GenerateRequest {
        GenerateRequest {
            system_prompt: system.into(),
            messages: msgs.iter()
                .map(|(r, c)| ChatMessage { role: (*r).into(), content: (*c).into() })
                .collect(),
            context: EnvSnapshot::default(),
            mode: QueryMode::SingleCommand,
            max_tokens: None,
        }
    }

    #[test]
    fn payload_carries_flattened_text_and_model() {
        let p = build_payload(&req("你是助理", &[("user", "哈囉")]), "gpt-5-5");
        assert_eq!(p["model"], "gpt-5-5");
        let text = p["text"].as_str().unwrap();
        assert!(text.contains("你是助理"));
        assert!(text.contains("哈囉"));
    }

    #[test]
    fn assistant_turns_are_preserved_in_order() {
        let p = build_payload(
            &req("", &[("user", "一"), ("assistant", "二"), ("user", "三")]),
            "gpt-5-5",
        );
        let text = p["text"].as_str().unwrap();
        let (a, b, c) = (text.find('一').unwrap(), text.find('二').unwrap(), text.find('三').unwrap());
        assert!(a < b && b < c, "順序要保留：{text}");
    }
}
```

- [ ] **Step 2: 執行測試確認失敗**

```bash
cd src-tauri && cargo test --lib ai::chatgpt_web
```

預期：`cannot find function build_payload`

- [ ] **Step 3: 實作**

```rust
//! ChatGPT 網頁版的 `AiProvider` 實作。傳輸走 `chatgpt_web::session`。

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::ai::{AiError, AiProvider, GenerateChunk, GenerateRequest};
use crate::chatgpt_web::protocol::{FlatTurn, SseOut, SseParser};
use crate::chatgpt_web::{flatten_history_for, session};

pub struct ChatgptWebProvider {
    model: String,
}

impl ChatgptWebProvider {
    pub fn new(model: String) -> Self {
        Self { model }
    }
}

/// 把 `GenerateRequest` 組成注入腳本要的 payload。抽成獨立函式才能不啟動
/// webview 就測。
pub fn build_payload(req: &GenerateRequest, model: &str) -> serde_json::Value {
    let turns: Vec<FlatTurn> = req.messages.iter().map(|m| {
        if m.role == "assistant" {
            FlatTurn::Assistant(m.content.clone())
        } else {
            FlatTurn::User(m.content.clone())
        }
    }).collect();
    serde_json::json!({
        "text": crate::chatgpt_web::protocol::flatten_history(&req.system_prompt, &turns),
        "model": model,
    })
}

#[async_trait]
impl AiProvider for ChatgptWebProvider {
    fn id(&self) -> &str { "chatgpt-web" }
    fn display_name(&self) -> &str { "ChatGPT Web" }

    async fn generate(
        &self,
        req: GenerateRequest,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<(), AiError> {
        let s = session::get().ok_or(AiError::NotConfigured)?;
        let payload = build_payload(&req, &self.model).to_string();
        // `_guard` 必須綁具名變數：綁成 `_` 會當場 drop，sink 立刻被移除，
        // 接下來一個 chunk 都收不到。
        let (mut rx, _guard) = s.request(payload)
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        let mut parser = SseParser::default();
        while let Some(raw) = rx.recv().await {
            // 注入腳本用 {"error":…} 回報失敗；其餘是 SSE 原文。
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
                    return Err(map_upstream_error(err, v.get("status").and_then(|s| s.as_u64())));
                }
            }
            // 餵原始 chunk，不要自己 `raw.lines()`——HTTP chunk 不保證切在行
            // 邊界上，切開的半行兩邊都會被靜默丟掉。行緩衝在 SseParser 裡。
            for out in parser.feed_str(&raw) {
                match out {
                    SseOut::Text(delta) => {
                        let _ = tx.send(GenerateChunk { delta, done: false, usage: None }).await;
                    }
                    SseOut::Done => {
                        let _ = tx.send(GenerateChunk {
                            delta: String::new(), done: true, usage: None,
                        }).await;
                        return Ok(());
                    }
                }
            }
        }
        // 通道關閉但沒收到 [DONE]（視窗被關、上游斷線）：仍要補一個 done，
        // 否則呼叫端會一直等。SinkGuard 會讓通道真的關得起來。
        let _ = tx.send(GenerateChunk { delta: String::new(), done: true, usage: None }).await;
        Ok(())
    }

    async fn health_check(&self) -> Result<(), AiError> {
        let s = session::get().ok_or(AiError::NotConfigured)?;
        // 顯示視窗：health_check 是設定頁「測試連線」按的，這正是引導登入的時機。
        s.ensure_window(true).map_err(|e| AiError::Network { message: e.to_string() })?;
        Ok(())
    }
}

/// 上游錯誤字串 → `AiError`。`not_logged_in` 由注入腳本產生，其餘是上游原文。
fn map_upstream_error(err: &str, status: Option<u64>) -> AiError {
    if err.contains("not_logged_in") {
        return AiError::AuthFailed;
    }
    match status {
        Some(401) | Some(403) => AiError::ModelError {
            reason: "ChatGPT 網頁版拒絕了請求".into(), raw: err.to_string(),
        },
        Some(429) => AiError::RateLimit { retry_after: None, body: Some(err.to_string()) },
        _ => AiError::Network { message: err.to_string() },
    }
}
```

同時把 `flatten_history` 由 `protocol` 匯出（Task 2 已 `pub`）。

- [ ] **Step 4: 執行測試確認通過**

```bash
cd src-tauri && cargo test --lib ai::chatgpt_web
```

預期：2 個測試 PASS

- [ ] **Step 5: 在 router 建構此 provider**

`src-tauri/src/ai/mod.rs` 加入 `pub mod chatgpt_web;`。

`src-tauri/src/ai/router.rs` 的 provider 建構 match，在 `ProviderType::Codex => {…}` 之後加入：

```rust
            ProviderType::ChatgptWeb => {
                // 憑證不在這裡——session 的 webview 自己持有登入狀態。
                Arc::new(crate::ai::chatgpt_web::ChatgptWebProvider::new(
                    provider_cfg.model.clone(),
                ))
            }
```

- [ ] **Step 6: 確認編譯與全套測試**

```bash
cd src-tauri && cargo check && cargo test --lib
```

預期：`Finished`，測試全 PASS

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/ai/chatgpt_web.rs src-tauri/src/ai/mod.rs src-tauri/src/ai/router.rs
git commit -m "feat(chatgpt-web): 實作 AiProvider::generate 與 health_check"
```

---

## Task 12：AiProvider 的 generate_with_tools

不實作這個，聊天面板（帶 MCP 工具）、Agent 迴圈、程式庫協助、知識庫問答四處都會拿到 `Unsupported`。

**Files:**
- Modify: `src-tauri/src/ai/chatgpt_web.rs`

- [ ] **Step 1: 寫失敗的測試**

加到 `ai/chatgpt_web.rs` 的 `mod tests`：

```rust
    use crate::ai::McpToolDefinition;

    fn tool(name: &str) -> McpToolDefinition {
        McpToolDefinition {
            name: name.into(),
            description: "說明".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    /// 契約要在最後（摺疊後落在 system 區塊尾端），提醒要黏在最後一則 user
    /// 訊息之後。依據 OmniRoute #7679 的實測：prepend 在 30K prompt 下是 0/3。
    #[test]
    fn tool_payload_places_contract_last_and_reminder_after_user() {
        let p = build_payload_with_tools(
            &req("系統指示", &[("user", "請讀檔")]),
            "gpt-5-5",
            &[tool("Read")],
            "n1",
        );
        let text = p["text"].as_str().unwrap();
        let user_at = text.find("請讀檔").unwrap();
        let reminder_at = text.find("Client protocol reminder").unwrap();
        let contract_at = text.find("Available tools").unwrap();
        assert!(user_at < reminder_at, "提醒要在 user 訊息之後");
        assert!(reminder_at < contract_at, "完整契約要在最後");
        assert!(text.contains("n1"), "nonce 要在契約裡");
    }

    #[test]
    fn no_tools_means_no_contract() {
        let p = build_payload_with_tools(&req("", &[("user", "哈囉")]), "gpt-5-5", &[], "n1");
        let text = p["text"].as_str().unwrap();
        assert!(!text.contains("Available tools"));
    }
```

- [ ] **Step 2: 執行測試確認失敗**

```bash
cd src-tauri && cargo test --lib ai::chatgpt_web
```

預期：`cannot find function build_payload_with_tools`

- [ ] **Step 3: 實作**

```rust
use crate::ai::{AiToolCall, GenerateWithToolsResult, McpToolDefinition};
use crate::chatgpt_web::tools;

/// 帶工具的 payload。契約放最後、提醒黏在最後一則 user 訊息之後——雙位置注入。
pub fn build_payload_with_tools(
    req: &GenerateRequest,
    model: &str,
    tool_defs: &[McpToolDefinition],
    nonce: &str,
) -> serde_json::Value {
    let mut turns: Vec<FlatTurn> = req.messages.iter().map(|m| {
        if m.role == "assistant" {
            FlatTurn::Assistant(m.content.clone())
        } else {
            FlatTurn::User(m.content.clone())
        }
    }).collect();

    let reminder = tools::build_reminder(tool_defs);
    if !reminder.is_empty() {
        // 掛在最後一個回合。這條路徑攤不出 ToolResult，所以最後一個回合實際上
        // 一定是 User；但寫法刻意與 Task 13 的 BridgeUpstream 一致——那邊會有
        // 只含 tool_result 的回合，兩處邏輯若分岔，之後只會修到其中一邊。
        match turns.last_mut() {
            Some(FlatTurn::User(text)) => text.push_str(&reminder),
            _ => turns.push(FlatTurn::User(reminder.trim_start().to_string())),
        }
    }

    let mut text = crate::chatgpt_web::protocol::flatten_history(&req.system_prompt, &turns);
    let contract = tools::build_contract(tool_defs, nonce);
    if !contract.is_empty() {
        text.push_str("\n\n");
        text.push_str(&contract);
    }
    serde_json::json!({ "text": text, "model": model })
}
```

在 `impl AiProvider for ChatgptWebProvider` 內加入：

```rust
    async fn generate_with_tools(
        &self,
        req: GenerateRequest,
        tool_defs: Vec<McpToolDefinition>,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<GenerateWithToolsResult, AiError> {
        let s = session::get().ok_or(AiError::NotConfigured)?;
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let payload = build_payload_with_tools(&req, &self.model, &tool_defs, &nonce).to_string();
        // `_guard` 必須綁具名變數：綁成 `_` 會當場 drop，sink 立刻被移除，
        // 接下來一個 chunk 都收不到。
        let (mut rx, _guard) = s.request(payload)
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        // 工具呼叫只能在整段回覆收完後才判斷得出來（封套可能跨 chunk），
        // 所以先收滿再剖析。串流仍照送，使用者看得到進度。
        let mut full = String::new();
        let mut parser = SseParser::default();
        while let Some(raw) = rx.recv().await {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
                    return Err(map_upstream_error(err, v.get("status").and_then(|s| s.as_u64())));
                }
            }
            // 餵原始 chunk，不要自己 `raw.lines()`——理由同 Task 11。
            let mut finished = false;
            for out in parser.feed_str(&raw) {
                match out {
                    SseOut::Text(delta) => {
                        full.push_str(&delta);
                        let _ = tx.send(GenerateChunk {
                            delta, done: false, usage: None,
                        }).await;
                    }
                    // 一定要主動結束，不能等通道關閉。
                    SseOut::Done => finished = true,
                }
            }
            if finished {
                break;
            }
        }
        let _ = tx.send(GenerateChunk { delta: String::new(), done: true, usage: None }).await;

        let (content, calls) = tools::parse_tool_calls(&full, &nonce);
        Ok(match calls {
            Some(calls) => GenerateWithToolsResult::ToolCalls { calls, raw: None },
            None => GenerateWithToolsResult::Text(content),
        })
    }
```

- [ ] **Step 4: 執行測試確認通過**

```bash
cd src-tauri && cargo test --lib ai::chatgpt_web
```

預期：4 個測試 PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/ai/chatgpt_web.rs
git commit -m "feat(chatgpt-web): 實作 generate_with_tools，雙位置注入契約"
```

---

## Task 13：BridgeUpstream

**Files:**
- Create: `src-tauri/src/bridge/upstream/chatgpt_web.rs`
- Modify: `src-tauri/src/bridge/upstream/mod.rs`
- Modify: `src-tauri/src/bridge/factory.rs`

- [ ] **Step 1: 寫失敗的測試**

`src-tauri/src/bridge/upstream/chatgpt_web.rs` 結尾：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::anthropic::request::MessagesRequest;

    fn req(json: serde_json::Value) -> MessagesRequest {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn tool_result_blocks_become_delimited_turns() {
        let r = req(serde_json::json!({
            "model": "aiterm:opus",
            "messages": [
                { "role": "user", "content": "讀檔" },
                { "role": "assistant", "content": [
                    { "type": "tool_use", "id": "tu_1", "name": "Read", "input": {"path": "a"} }
                ]},
                { "role": "user", "content": [
                    { "type": "tool_result", "tool_use_id": "tu_1", "content": "內容" }
                ]}
            ]
        }));
        let p = build_payload(&r, "gpt-5-5", "n1");
        let text = p["text"].as_str().unwrap();
        assert!(text.contains("[[tool_call:Read#tu_1]]"), "實際：{text}");
        assert!(text.contains("[[tool_result:tu_1]]"));
        assert!(text.contains("內容"));
    }

    #[test]
    fn system_prompt_is_included() {
        let r = req(serde_json::json!({
            "model": "aiterm:opus",
            "system": "你是助理",
            "messages": [{ "role": "user", "content": "哈囉" }]
        }));
        let text = build_payload(&r, "gpt-5-5", "n1")["text"].as_str().unwrap().to_string();
        assert!(text.contains("你是助理"));
    }

    /// agent loop 的常態：最後一則訊息只含 tool_result，攤出來一個 FlatTurn::User
    /// 都沒有。提醒必須落在整段文字的尾端（契約之前），不能被丟回很前面的
    /// user 回合——多輪工具迴圈正是最需要提醒生效的場景。
    #[test]
    fn reminder_stays_at_the_end_when_last_turn_is_a_tool_result() {
        let r = req(serde_json::json!({
            "model": "aiterm:opus",
            "tools": [{ "name": "Read", "description": "讀檔",
                        "input_schema": {"type": "object"} }],
            "messages": [
                { "role": "user", "content": "讀檔" },
                { "role": "assistant", "content": [
                    { "type": "tool_use", "id": "tu_1", "name": "Read", "input": {"path": "a"} }
                ]},
                { "role": "user", "content": [
                    { "type": "tool_result", "tool_use_id": "tu_1", "content": "內容" }
                ]}
            ]
        }));
        let p = build_payload(&r, "gpt-5-5", "n1");
        let text = p["text"].as_str().unwrap();
        let first_user_at = text.find("讀檔").unwrap();
        let result_at = text.find("[[tool_result:tu_1]]").unwrap();
        let reminder_at = text.find("Client protocol reminder")
            .unwrap_or_else(|| panic!("提醒完全沒出現，實際：{text}"));
        let contract_at = text.find("Available tools").unwrap();
        assert!(result_at < reminder_at,
                "提醒被掛到工具結果之前了（掉回舊的 user 回合），實際：{text}");
        assert!(first_user_at < reminder_at);
        assert!(reminder_at < contract_at, "完整契約仍要在最後");
    }
}
```

- [ ] **Step 2: 執行測試確認失敗**

```bash
cd src-tauri && cargo test --lib bridge::upstream::chatgpt_web
```

預期：`cannot find function build_payload`

- [ ] **Step 3: 實作**

```rust
//! ChatGPT 網頁版上游（Claude Code 橋接用）。

use async_trait::async_trait;
use futures_util::stream;

use crate::ai::AiError;
use crate::bridge::anthropic::request::{parse_content, system_text, ContentBlock, MessagesRequest};
use crate::bridge::upstream::{BridgeUpstream, StopReason, UpstreamEvent, UpstreamResponse, Usage};
use crate::chatgpt_web::protocol::{flatten_history, FlatTurn, SseOut, SseParser};
use crate::chatgpt_web::{session, tools};

pub struct ChatgptWebUpstream;

/// 把 Anthropic Messages 請求攤平成注入腳本要的 payload。
pub fn build_payload(req: &MessagesRequest, model: &str, nonce: &str) -> serde_json::Value {
    let mut turns: Vec<FlatTurn> = Vec::new();
    for m in &req.messages {
        for block in parse_content(&m.content) {
            match block {
                ContentBlock::Text(t) | ContentBlock::Thinking(t) => {
                    if m.role == "assistant" {
                        turns.push(FlatTurn::Assistant(t));
                    } else {
                        turns.push(FlatTurn::User(t));
                    }
                }
                ContentBlock::ToolUse { id, name, input } => {
                    turns.push(FlatTurn::ToolCall { id, name, args: input.to_string() });
                }
                ContentBlock::ToolResult { tool_use_id, content } => {
                    let text = content.iter().filter_map(|b| match b {
                        ContentBlock::Text(t) => Some(t.as_str()),
                        _ => None,
                    }).collect::<Vec<_>>().join("\n");
                    turns.push(FlatTurn::ToolResult { id: tool_use_id, content: text });
                }
                ContentBlock::Image { .. } => {
                    turns.push(FlatTurn::User("[圖片：此供應商不支援]".into()));
                }
            }
        }
    }

    let tool_defs = tools::from_anthropic_tools(req.tools.as_ref());
    let reminder = tools::build_reminder(&tool_defs);
    if !reminder.is_empty() {
        // 掛在「最後一個回合」而非「最後一個 user 回合」：agent loop 裡最後一則
        // 訊息通常只含 tool_result 區塊，攤出來一個 FlatTurn::User 都沒有。往回
        // 找 User 會把提醒掛到很前面的回合、甚至掛不上去——而雙位置注入的依據
        // （OmniRoute #7679）正是「貼近當前回合」才有效。多輪工具迴圈是這個
        // provider 最需要提醒生效的場景，不能剛好是它失準的場景。
        match turns.last_mut() {
            Some(FlatTurn::User(text)) => text.push_str(&reminder),
            _ => turns.push(FlatTurn::User(reminder.trim_start().to_string())),
        }
    }

    let mut text = flatten_history(&system_text(req.system.as_ref()), &turns);
    let contract = tools::build_contract(&tool_defs, nonce);
    if !contract.is_empty() {
        text.push_str("\n\n");
        text.push_str(&contract);
    }
    serde_json::json!({ "text": text, "model": model })
}

#[async_trait]
impl BridgeUpstream for ChatgptWebUpstream {
    async fn send(&self, req: &MessagesRequest, model: &str) -> Result<UpstreamResponse, AiError> {
        let s = session::get().ok_or(AiError::NotConfigured)?;
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let payload = build_payload(req, model, &nonce).to_string();
        // `_guard` 必須綁具名變數：綁成 `_` 會當場 drop，sink 立刻被移除，
        // 接下來一個 chunk 都收不到。
        let (mut rx, _guard) = s.request(payload)
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        // 工具封套可能跨 chunk，必須收滿整段才剖析——所以這裡不是逐 chunk
        // 轉發，而是收完後一次產生事件序列。使用者端的體感差異由 Claude Code
        // 自己的等待指示吸收。
        let mut full = String::new();
        let mut parser = SseParser::default();
        while let Some(raw) = rx.recv().await {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
                    let status = v.get("status").and_then(|s| s.as_u64());
                    return Err(crate::ai::chatgpt_web::map_upstream_error(err, status));
                }
            }
            // 餵原始 chunk，不要自己 `raw.lines()`——理由同 Task 11。
            let mut finished = false;
            for out in parser.feed_str(&raw) {
                match out {
                    SseOut::Text(d) => full.push_str(&d),
                    // 一定要主動結束，不能等通道關閉。
                    SseOut::Done => finished = true,
                }
            }
            if finished {
                break;
            }
        }

        let (content, calls) = tools::parse_tool_calls(&full, &nonce);
        let mut events: Vec<Result<UpstreamEvent, AiError>> = Vec::new();
        if !content.trim().is_empty() {
            events.push(Ok(UpstreamEvent::TextDelta(content)));
        }
        let stop = match &calls {
            Some(calls) => {
                for c in calls {
                    events.push(Ok(UpstreamEvent::ToolUseStart {
                        id: c.id.clone(), name: c.tool_name.clone(),
                    }));
                    events.push(Ok(UpstreamEvent::ToolInputDelta(c.args.to_string())));
                    events.push(Ok(UpstreamEvent::ToolUseEnd));
                }
                StopReason::ToolUse
            }
            None => StopReason::EndTurn,
        };
        events.push(Ok(UpstreamEvent::Done {
            stop_reason: stop,
            usage: Usage { input_tokens: 0, output_tokens: 0 },
        }));

        Ok(UpstreamResponse::Events(Box::pin(stream::iter(events))))
    }
}
```

- [ ] **Step 3b: 加入 Anthropic 工具定義的轉換**

Anthropic 的 `tools[]` 是 JSON 陣列，`AiProvider` 那條路徑則是 `McpToolDefinition`。
**不要寫第二份契約文字** —— 轉成同一個型別後重用 `build_contract`／`build_reminder`，
否則兩份文字會漸漸漂移，而契約措辭正是 OmniRoute #7679 花力氣調出來的東西。

加到 `src-tauri/src/chatgpt_web/tools.rs`：

```rust
/// Anthropic Messages 的 `tools[]` → `McpToolDefinition`。
///
/// 缺 `name` 的項目直接略過：沒有名字的工具無法被呼叫，放進契約只會佔用
/// 上下文並誤導模型。
pub fn from_anthropic_tools(tools: Option<&serde_json::Value>) -> Vec<McpToolDefinition> {
    let Some(arr) = tools.and_then(|t| t.as_array()) else { return Vec::new() };
    arr.iter().filter_map(|t| {
        Some(McpToolDefinition {
            name: t.get("name")?.as_str()?.to_string(),
            description: t.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string(),
            input_schema: t.get("input_schema").cloned()
                .unwrap_or(serde_json::json!({"type": "object"})),
        })
    }).collect()
}
```

對應的測試，加到 `tools.rs` 的 `mod tests`：

```rust
    #[test]
    fn converts_anthropic_tools_and_skips_nameless_ones() {
        let v = json!([
            { "name": "Read", "description": "讀檔", "input_schema": {"type":"object"} },
            { "description": "沒有名字" },
        ]);
        let out = from_anthropic_tools(Some(&v));
        assert_eq!(out.len(), 1, "缺 name 的要略過");
        assert_eq!(out[0].name, "Read");
    }

    #[test]
    fn missing_tools_field_yields_empty_vec() {
        assert!(from_anthropic_tools(None).is_empty());
    }
```

執行：

```bash
cd src-tauri && cargo test --lib chatgpt_web::tools
```

預期：11 個測試全 PASS

- [ ] **Step 4: 執行測試確認通過**

```bash
cd src-tauri && cargo test --lib bridge::upstream::chatgpt_web
```

預期：2 個測試 PASS

- [ ] **Step 5: 接上 factory**

`src-tauri/src/bridge/upstream/mod.rs` 加入 `pub mod chatgpt_web;`。

`src-tauri/src/bridge/factory.rs`：`UpstreamKind` 加入 `ChatgptWeb`，`kind_for()` 加入 `ProviderType::ChatgptWeb => UpstreamKind::ChatgptWeb`，`build()` 加入對應分支回 `Box::new(ChatgptWebUpstream)`。

- [ ] **Step 6: 確認編譯與全套測試**

```bash
cd src-tauri && cargo check && cargo test
```

預期：`Finished`，測試全 PASS

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/bridge/
git commit -m "feat(chatgpt-web): 實作 BridgeUpstream，接上 Claude Code 橋接"
```

---

## Task 14：模型清單命令

**Files:**
- Modify: `src-tauri/src/chatgpt_web/session.rs`
- Modify: `src-tauri/src/chatgpt_web/inject.js`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 在注入腳本加入模型查詢**

```js
  // /backend-api/models 回的是「該帳號實際可用」的清單，所以不需要維護方案
  // 與模型的對應表——登入哪個帳號就顯示什麼。max_tokens 也一併取得。
  window.__aitermModels = async (id) => {
    try {
      if (!(await ensureAuth())) throw new Error("not_logged_in");
      const r = await fetch("/backend-api/models", { headers: authHeaders() });
      invoke("chatgpt_web_chunk", { id, data: await r.text() });
    } catch (e) {
      invoke("chatgpt_web_chunk", { id, data: JSON.stringify({ error: String(e) }) });
    }
  };
```

- [ ] **Step 2: 加入 command**

`session.rs`：

```rust
/// 設定頁用：取回該帳號可用的模型清單。
#[tauri::command]
pub async fn chatgpt_web_models() -> Result<Vec<ChatgptWebModel>, String> {
    let s = get().ok_or("session 未初始化")?;
    let (mut rx, _guard) = s.request_raw("__aitermModels").map_err(|e| e.to_string())?;
    let body = rx.recv().await.ok_or("沒有回應")?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
        return Err(err.to_string());
    }
    Ok(v.get("models").and_then(|m| m.as_array()).map(|arr| {
        arr.iter().filter_map(|m| Some(ChatgptWebModel {
            slug: m.get("slug")?.as_str()?.to_string(),
            title: m.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string(),
            max_tokens: m.get("max_tokens").and_then(|t| t.as_u64()).unwrap_or(0) as u32,
        })).collect()
    }).unwrap_or_default())
}

#[derive(serde::Serialize)]
pub struct ChatgptWebModel {
    pub slug: String,
    pub title: String,
    pub max_tokens: u32,
}
```

`request_raw` 的實作，加到 `impl Session`：

```rust
    /// 呼叫注入腳本上某個具名函式（模型查詢等一次性用途），不經過 pending map
    /// ——這類請求沒有 payload 要拉取。
    pub fn request_raw(
        self: &Arc<Self>,
        js_fn: &str,
    ) -> Result<(mpsc::UnboundedReceiver<String>, SinkGuard), tauri::Error> {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = mpsc::unbounded_channel();
        self.sinks.lock().expect("sinks poisoned").insert(id.clone(), tx);
        let guard = SinkGuard { session: Arc::clone(self), id: id.clone() };
        let w = self.ensure_window(false)?;
        w.eval(format!("window.{js_fn}({})", serde_json::json!(id)))?;
        Ok((rx, guard))
    }
```

在 `lib.rs` 註冊 `chatgpt_web::session::chatgpt_web_models`。

- [ ] **Step 3: 確認編譯**

```bash
cd src-tauri && cargo check && cargo test --lib chatgpt_web
```

預期：`Finished`，測試全 PASS

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/chatgpt_web/ src-tauri/src/lib.rs
git commit -m "feat(chatgpt-web): 動態取得帳號可用的模型清單"
```

---

## Task 15：前端設定 UI 與 i18n

**Files:**
- Create: `src/ipc/chatgptWeb.ts`
- Modify: `src/components/Settings/ProviderForm.tsx`
- Modify: `src/components/Settings/ClaudeBridgePage.tsx`
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: 寫失敗的測試**

`src/components/Settings/ProviderForm.test.tsx`（若不存在則建立）：

```tsx
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { ProviderForm } from "./ProviderForm";

vi.mock("../../ipc/chatgptWeb", () => ({ chatgptWebModels: vi.fn().mockResolvedValue([]) }));

describe("ProviderForm — ChatGPT Web 風險提示", () => {
  it("選擇 chatgpt-web 時三項風險都要顯示", () => {
    render(
      <LocaleProvider>
        <ProviderForm initialType="chatgpt-web" onSave={() => {}} onCancel={() => {}} />
      </LocaleProvider>,
    );
    expect(screen.getByText(/服務條款/)).toBeInTheDocument();
    expect(screen.getByText(/模擬/)).toBeInTheDocument();
    expect(screen.getByText(/上游變更/)).toBeInTheDocument();
  });
});
```

（實際的 props 名稱依 `ProviderForm.tsx` 現況調整；若該元件不接受 `initialType`，改用它既有的型別選單互動來觸發。）

- [ ] **Step 2: 執行測試確認失敗**

```bash
npx vitest run src/components/Settings/ProviderForm.test.tsx
```

預期：FAIL，找不到風險文字

- [ ] **Step 3: 加入 i18n 字串**

`src/lib/i18n.ts` 的 zh-TW 與 en 兩份都加：

```ts
    chatgpt_web_risk_tos: "以程式化方式驅動 ChatGPT 網頁介面違反 OpenAI 服務條款，帳號有被停權的風險。",
    chatgpt_web_risk_tools: "工具呼叫是 prompt 模擬的，模型可能不照格式輸出，多輪保真度低於原生 tool calling。",
    chatgpt_web_risk_upstream: "認證機制是逆向而來，上游變更即失效，整個供應商會回 403。",
    chatgpt_web_login: "登入 ChatGPT",
    chatgpt_web_context_hint: "上下文上限：{0} tokens",
    chatgpt_web_tier_hint: "建議對應到 Haiku 層——Claude Code 用它跑背景小任務，請求短、不需長上下文。",
```

- [ ] **Step 4: 實作 UI**

`src/ipc/chatgptWeb.ts`：

```ts
import { invoke } from "@tauri-apps/api/core";

export interface ChatgptWebModel {
  slug: string;
  title: string;
  max_tokens: number;
}

export const chatgptWebModels = () => invoke<ChatgptWebModel[]>("chatgpt_web_models");
```

在 `ProviderForm.tsx`：型別選單加入 `chatgpt-web`；選中時顯示三項風險、「登入 ChatGPT」按鈕（呼叫 `test_provider` 觸發 `health_check`，會把視窗顯示出來）、以及以 `chatgptWebModels()` 取得的模型下拉（每項顯示 `title` 與 `max_tokens`）。

`ClaudeBridgePage.tsx` 的 `SUPPORTED_TYPES` 加入 `"chatgpt-web"`，並在選到它時顯示 `chatgpt_web_tier_hint`。

- [ ] **Step 5: 執行測試確認通過**

```bash
npx vitest run src/components/Settings/ProviderForm.test.tsx && npx tsc -b && npm run lint
```

預期：測試 PASS、tsc 無輸出、lint 維持 92 problems（基準值）

- [ ] **Step 6: Commit**

```bash
git add src/ipc/chatgptWeb.ts src/components/Settings/ src/lib/i18n.ts
git commit -m "feat(chatgpt-web): 設定 UI、動態模型清單與風險提示"
```

---

## Task 16：移除探勘程式碼並加入端到端測試

**Files:**
- Delete: `src-tauri/src/probe_chatgpt.rs`
- Delete: `src-tauri/capabilities/probe-chatgpt.json`
- Modify: `src-tauri/src/lib.rs`
- Create: `src-tauri/tests/chatgpt_web_probe.rs`

> **刪掉探勘程式碼之前，先用它錄一條真實的 SSE 串流存成 fixture。**
>
> Task 3 的 `SseParser` 測試全部是手寫的極簡 JSON（`{"message":{"content":
> {"parts":[…]}}}`），沒有 `author`、`id`、`status`、`end_turn`、`metadata`。
> 手寫 fixture 會把「我以為的上游長相」固化成測試——Task 3 就是因此讓兩個
> 真實 bug（中段空 frame 重設狀態、跨 chunk 切行）溜過 7 個綠燈測試。
>
> 錄下來之後要回頭確認三件事，並補測試：
> 1. 非內容 frame（moderation、`role:"system"`、只帶 `conversation_id`）
>    實際長什麼樣，`SseParser` 是否真的原封不動略過。
> 2. **兩則訊息會不會在同一條串流裡交錯**（思考區塊與答案）。目前的
>    `SseParser` 只保留一份 `emitted`，交錯時每次切換都會整段重送。若確認
>    會交錯，`emitted` 要改成以 `message.id` 為鍵。
> 3. chunk 實際切在哪裡——確認行緩衝真的有派上用場。

- [ ] **Step 1: 移除探勘程式碼**

```bash
rm src-tauri/src/probe_chatgpt.rs src-tauri/capabilities/probe-chatgpt.json
```

`src-tauri/src/lib.rs` 移除三處：`pub mod probe_chatgpt;`、`generate_handler!` 裡的 `probe_chatgpt::probe_report`、setup 裡的 `AITERM_PROBE_CHATGPT` 區塊。

- [ ] **Step 2: 確認沒有殘留**

```bash
grep -rn "probe_chatgpt\|AITERM_PROBE_CHATGPT" src-tauri/src/ || echo "已清空"
cd src-tauri && cargo check
```

預期：印出「已清空」，`cargo check` 通過

- [ ] **Step 3: 建立端到端測試**

`src-tauri/tests/chatgpt_web_probe.rs`：

```rust
//! ChatGPT 網頁版的端到端檢查。
//!
//! `#[ignore]`：需要真實登入狀態與 GUI，CI 跑不了。手動執行：
//!   cargo test --test chatgpt_web_probe -- --ignored --nocapture
//!
//! 這裡刻意不做自動化整合測試——上游隨時變動，放進 CI 只會產生假紅燈。

#[test]
#[ignore = "需要真實 ChatGPT 登入與 GUI"]
fn flatten_and_contract_round_trip() {
    // 驗證攤平 → 契約 → 剖析這條純函式鏈是自洽的：契約要求模型輸出的封套，
    // 剖析器必須認得。這一段不需要網路。
    use aiterm_lib::ai::McpToolDefinition;
    use aiterm_lib::chatgpt_web::tools;

    let defs = vec![McpToolDefinition {
        name: "Read".into(),
        description: "讀檔".into(),
        input_schema: serde_json::json!({"type": "object"}),
    }];
    let nonce = "test-nonce";
    let contract = tools::build_contract(&defs, nonce);
    assert!(contract.contains(nonce));

    // 模仿模型照契約輸出
    let reply = format!(
        r#"好的。<tool>{{"name":"Read","arguments":{{"path":"a.txt"}},"_nonce":"{nonce}"}}</tool>"#
    );
    let (content, calls) = tools::parse_tool_calls(&reply, nonce);
    let calls = calls.expect("契約產生的封套必須被自己的剖析器認得");
    assert_eq!(calls[0].tool_name, "Read");
    assert!(content.contains("好的"));
}
```

- [ ] **Step 4: 執行**

```bash
cd src-tauri && cargo test --test chatgpt_web_probe -- --ignored --nocapture
```

預期：PASS

- [ ] **Step 5: 全套驗證**

```bash
cd src-tauri && cargo test && cd .. && npx tsc -b && npm run lint && npm run test
```

預期：Rust 全 PASS、tsc 無輸出、lint 92 problems（基準值）、前端測試全 PASS

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "chore(chatgpt-web): 移除探勘程式碼，加入端到端契約自洽測試"
```

---

## 完成後的檢查清單

- [ ] `cargo test` 全綠
- [ ] `npx tsc -b` 無輸出
- [ ] `npm run lint` 維持 92 problems（基準值，不可增加）
- [ ] `npm run test` 全綠
- [ ] 手動：設定頁新增 ChatGPT Web 供應商 → 登入 → 模型清單出現 → 三項風險可見
- [ ] 手動：`/ai` 指令能用此供應商回應
- [ ] 手動：Claude Code 橋接分頁能用此供應商，且工具呼叫會被執行
- [ ] 手動：關閉 ChatGPT 視窗後，下一個請求會自動重建且**不需要重新登入**
- [ ] `probe/chatgpt-web` 分支可刪除
