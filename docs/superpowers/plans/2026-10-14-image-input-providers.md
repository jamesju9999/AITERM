# Ask AI 附圖：各家供應商都收得到圖片 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 前端送的 OpenAI 格式圖片段落，在後端依供應商轉成各自的原生格式；ChatGPT 網頁版丟掉圖片並提示；超過 5MB 的圖片提示使用者。

**Architecture:** `ai/content.rs` 把 `ChatMessage.content`（字串或陣列）解析成 `Part::{Text, Image}`；各家請求組裝函式用它轉換。`AiChatReply.images_dropped` 回報 ChatGPT 網頁版丟圖；前端顯示提示。

**Spec:** `docs/superpowers/specs/2026-10-14-image-input-providers-design.md`（含探勘結果與各家 JSON 形狀，實作前先讀）。

**通用規則：**
- 每個新測試先跑、親眼看到紅，再實作；一寫就綠代表測試資料分不出對錯，停下來修測試。
- 不為了好測扭曲實作；計畫與實際程式碼衝突時最小調整並回報；計畫看起來錯就停下來回報。
- 只 `git add` 本 task 動到的路徑；`docs/` 要 `git add -f`。部分 Rust 檔是 CRLF，改完看 `git diff --stat`，不可用 Python 整檔改寫而換掉行尾。
- Rust 最終驗證：`cd src-tauri && cargo test --workspace --no-fail-fast`，看每一行 `test result:`。前端：`npx tsc -b`、`npm run test`。
- commit 訊息結尾：`Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`。

測試用的圖片：任何短 base64 字串即可（例如 `"QUJD"`），data URI 形如 `data:image/png;base64,QUJD`。

---

### Task 1：`ai/content.rs`

**Files:** Create `src-tauri/src/ai/content.rs`；Modify `src-tauri/src/ai/mod.rs`（`pub mod content;`）。

API（照 spec）：

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part { Text(String), Image { media_type: String, data: String } }
pub fn parts(content: &serde_json::Value) -> Vec<Part>;
pub fn has_image(content: &serde_json::Value) -> bool;
pub fn text_only(content: &serde_json::Value) -> String;
```

規則：
- `Value::String(s)` → `[Text(s)]`（空字串也回一個 Text）；`Null` → `[]`。
- 陣列：`{type:"text",text}` → Text；`{type:"image_url",image_url:{url}}` 且 url 是 `data:<mime>;base64,<data>` → Image；url 不是 data URI、或格式壞掉 → 略過；其他 type → 略過。
- 其他型別（數字、物件）→ `[Text(value.to_string())]`（維持舊行為的保底）。
- `text_only`：所有 Text 用 `"\n\n"` 接起來。

測試（全部先紅）：字串、null、文字＋圖片陣列（順序保留、media_type 與 data 正確拆開）、http 圖片網址被略過、壞掉的 data URI（沒有 `;base64,`）被略過、未知 type 被略過、`has_image` 真／假、`text_only` 只含文字。

Commit：`feat(ai): parse chat content into text and image parts`

### Task 2：Anthropic

**Files:** `src-tauri/src/ai/anthropic.rs`。

- 新增 `fn to_anthropic_content(content: &Value) -> Value`：字串、null 原樣回傳；陣列 → 依 `content::parts` 轉成 `{type:"text",text}` 與 `{type:"image",source:{type:"base64",media_type,data}}`。
- 串流路徑（`AnthropicMessage { content: m.content.clone() }`，約第 384 行）與工具路徑 `build_anthropic_messages` 一般訊息那行（`json!({ "role": m.role, "content": m.content })`）都改用它。tool_result 與 assistant tool_use 不動。
- 測試：兩條路徑各一條，送一則含圖片的 user 訊息，斷言組出的 content[1] 是 image 區塊且 `source.data == "QUJD"`、沒有 `image_url`；純字串訊息不變。

Commit：`fix(anthropic): send attached images as image blocks`

### Task 3：Ollama

**Files:** `src-tauri/src/ai/ollama.rs`。

- `OllamaMessage` 加 `#[serde(skip_serializing_if = "Vec::is_empty", default)] images: Vec<String>`。
- `build_messages` 一般訊息：若 `content::has_image`，`content` = `Value::String(text_only)`、`images` = 圖片的 data（無前綴）；否則照舊。其他建構 `OllamaMessage` 的地方補 `images: vec![]`。
- 測試：含圖片 → `content` 是字串、`images == ["QUJD"]`；純文字 → 序列化結果不含 `images` 鍵。

Commit：`fix(ollama): send attached images in the images field`

### Task 4：Codex（照文件，未驗證）

**Files:** `src-tauri/src/ai/codex.rs` `build_request_body`（約 129 行）。

- 每則訊息的 content 段落：user 訊息 → Text 轉 `{type: content_type_for_role(role), text}`、Image 轉 `{type:"input_image", image_url: "data:<mime>;base64,<data>"}`；assistant 等其他角色 → 一個 `{type: content_type, text: text_only}`（取代原本對陣列 `to_string()`）。字串 content 的輸出必須和現在完全一樣（既有測試要維持綠）。
- 測試：含圖片的 user 訊息 → content 有 `input_text` 與 `input_image`，且整份 body 的字串表示不含 `image_url":{`（不再把陣列塞進文字）。
- 程式碼註解標明「照 Responses API 文件寫，尚未用真帳號驗證（見 spec 探勘結果）」。

Commit：`feat(codex): send attached images as input_image (unverified)`

### Task 5：Antigravity（照文件，未驗證）

**Files:** `src-tauri/src/ai/antigravity.rs` `build_request_body`（約 337 行）。

- 非 system 訊息：parts → `[{text}, {inlineData:{mimeType,data}}]`（只有文字時輸出必須與現在相同：`[{"text": …}]`）。system 訊息用 `text_only` 併入 system。
- 測試：含圖片 → parts 有 `inlineData.data == "QUJD"`、`mimeType == "image/png"`；body 字串不含 `image_url`。
- 同 Task 4 的「未驗證」註解。

Commit：`feat(antigravity): send attached images as inlineData (unverified)`

### Task 6：ChatGPT 網頁版

**Files:** `src-tauri/src/ai/chatgpt_web.rs` `message_text`。

- 改用 `content::text_only`；陣列 content 不再 `to_string()`。
- 測試：含圖片的訊息 → payload 文字含使用者文字、不含 `QUJD`、不含 `image_url`。

Commit：`fix(chatgpt-web): drop images instead of sending base64 as text`

### Task 7：`images_dropped`

**Files:** `src-tauri/src/commands/ai.rs`。

- `AiChatReply` 加 `#[serde(skip_serializing_if = "std::ops::Not::not")] pub images_dropped: bool`；所有建構處補 `images_dropped: false`（之後再設定）。
- 純函式 `fn images_will_be_dropped(provider_type: Option<ProviderType>, messages: &[ChatMessage]) -> bool`：型別是 ChatGPT 網頁版且任何訊息 `content::has_image` 時為真。`ProviderType` 的實際變體名稱請在 `config` 裡確認。
- `ai_chat`（純聊天 `run_chat` 與帶工具兩條路徑、含降級）回傳前，依「實際使用的供應商」（`provider_id` 參數，沒給時用預設）設定。實際取得型別的方式請看 `AiRouter::resolve`／`resolve_by_id` 怎麼找 provider 設定。
- 測試：純函式三種情況（網頁版＋圖＝真；網頁版無圖＝假；其他型別有圖＝假）。

Commit：`feat(ai): report when the provider dropped attached images`

### Task 8：前端

**Files:** `src/ipc/ai.ts`、`src/hooks/useMcpChat.ts`（＋test）、`src/lib/i18n.ts`、`src/components/AiPanel/index.tsx`（＋`AiPanel.test.tsx`）。

- `AiChatReply` 加 `images_dropped?: boolean`。
- `useMcpChat`：每輪 `aiChat` 回來若 `images_dropped` 且本次 `sendMessage` 還沒提示過，`setMessages` 加一則 assistant 訊息 `t.ai_images_dropped`。測試：mock 回 `images_dropped: true` 兩輪（第一輪工具呼叫、第二輪文字）→ 提示只出現一次；沒有旗標 → 不出現。
- i18n（zhTW 與 enRaw 都要）：
  - `ai_images_dropped`: 「這個供應商不支援看圖，圖片沒有送出，只送了文字。要讓 AI 看圖請換其他供應商。」／ "This provider can't read images, so the image wasn't sent — only your text was. Switch to another provider to have the AI look at images."
  - `ai_image_too_large: (names: string) =>` 「{names} 超過 5MB，沒有附上」／ "{names} is larger than 5 MB and wasn't attached"
- `AiPanel.processFiles`：被拒絕的圖片名稱收集起來，有的話設 state `oversizeNotice`，顯示在輸入框上方（放在 `extraAboveInput`，沿用附近現有提示的樣式 class）；下一次 `processFiles` 成功附加或送出時清掉。`useCallback` 依賴要包含 `t`。測試：上傳一個 `type: "image/png"`、大小超過 5MB 的 File（可用 `Object.defineProperty(file, "size", { value: 6 * 1024 * 1024 })`）→ 看到提示、附件列沒有它。

Commit：`feat(ask-ai): tell the user when images are too large or the provider can't read them`

### Task 9：CHANGELOG 與驗證

- CHANGELOG 最上面的 `## v1.41.0` 段落補一節（寫給使用者看，只寫已實測的）：Anthropic 與 Ollama 附圖修好（以前 Anthropic 會直接報錯、Ollama 會失敗）；ChatGPT 網頁版附圖會提示不支援、只送文字；超過 5MB 的圖片會提示沒有附上。不提 Codex／Gemini（Antigravity）。
- spec 補「實作時補充的決定」。
- 完整驗證：`npx tsc -b`、`npm run test`、`cd src-tauri && cargo test --workspace --no-fail-fast`。
