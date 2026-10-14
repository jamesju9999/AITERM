# Ask AI 附圖：各家供應商都收得到圖片

日期：2026-10-14（分支 `feat/image-input-providers`，從 `feat/unified-agent-mcp-toggle` 分出）

## 問題

Ask AI 可以附圖（📎、貼上、拖曳），前端一律送 OpenAI 格式的內容陣列：

```json
[{"type":"text","text":"…"},{"type":"image_url","image_url":{"url":"data:image/png;base64,…"}}]
```

後端各家幾乎都原樣轉送，只有 OpenAI 風格的端點吃得下。另外超過 5MB 的圖片會被無聲略過。

## 探勘結果（2026-10-14，`src-tauri/tests/image_input_probe.rs`）

測試圖是左黃右紫的 64×64 PNG，問左右各是什麼顏色（第一輪用純藍，會被猜中，不算數）。

| 供應商 | App 現在的送法 | 原生格式 |
|---|---|---|
| OpenAI 相容（oMLX gemma-4）、Copilot（Claude Sonnet 4.6）、Gemini API 金鑰（OpenAI 相容端點） | ✅ 左黃右紫（純聊天、帶工具都對） | — |
| Anthropic（訂閱 OAuth） | ❌ HTTP 400：`Input tag 'image_url' … does not match any of the expected tags` | ✅ `{"type":"image","source":{"type":"base64","media_type":"image/png","data":…}}` 答對 |
| Ollama 0.23.2（gemma4，curl 直接打 `/api/chat`） | ❌ `cannot unmarshal array into … content of type string` | ✅ `content` 放文字、`images: [base64]` 答對 |
| Codex（GPT5.6） | 現在把整個陣列 `to_string()` 成文字送出；第一輪純藍答 blue（判定為猜中） | 未驗證：帳號額度用完（free plan，約 27 天後重置） |
| Gemini（Antigravity，Gemini2.5） | 同 Codex，陣列變文字 | 未驗證：429 RESOURCE_EXHAUSTED |
| ChatGPT 網頁版 | 陣列變文字（一長串 base64） | 不做（要走網頁版的檔案上傳流程，逆向且無文件） |

另外觀察到：Codex 與 Antigravity 的 `generate_with_tools` 回 `Unsupported`，⚡ 模式下走 `ai_chat` 的文字協定降級（重打 `generate`），所以轉換必須放在 `generate` 會經過的請求組裝裡。

## 設計

### 後端：共用解析＋各家轉換

新增 `src-tauri/src/ai/content.rs`：

```rust
pub enum Part { Text(String), Image { media_type: String, data: String /* base64，無前綴 */ } }
/// 字串 → 一個 Text；陣列 → 依序解析 text／image_url（data URI）；
/// 不是 data URI 的 image_url、不認得的段落 → 略過；null → 空。
pub fn parts(content: &serde_json::Value) -> Vec<Part>;
pub fn has_image(content: &serde_json::Value) -> bool;
/// 只取文字段落、用 "\n\n" 接起來（給只吃純文字的地方）。
pub fn text_only(content: &serde_json::Value) -> String;
```

各家在組請求時使用（只動 user／tool 以外的角色不需要，圖片只出現在 user 訊息）：

| 供應商 | 檔案 | 轉換 |
|---|---|---|
| OpenAI、OpenAI 相容、Copilot、Google API 金鑰 | 不動 | 原樣 |
| Anthropic | `ai/anthropic.rs`：串流路徑（`AnthropicMessage`）與工具路徑（`build_anthropic_messages`）都要 | content 是陣列時轉成 `[{type:text},{type:image,source:{type:base64,media_type,data}}]`；字串照舊 |
| Ollama | `ai/ollama.rs` `build_messages` | `content` = `text_only`，新欄位 `images: Vec<String>`（base64，`skip_serializing_if` 空） |
| Codex | `ai/codex.rs` `build_request_body` | user 訊息的 content 陣列轉成 `[{type:input_text,text},{type:input_image,image_url:"data:…"}]`；其他角色用 `text_only`（不再 `to_string()` 整個陣列） |
| Antigravity | `ai/antigravity.rs` `build_request_body` | parts 轉成 `[{text},{inlineData:{mimeType,data}}]`；system 一樣用 `text_only` |
| ChatGPT 網頁版 | `ai/chatgpt_web.rs` `message_text` | 改用 `text_only`：圖片丟掉，不再把 base64 當文字送出 |

### 回報「圖片被略過」

`AiChatReply` 加 `images_dropped: bool`（`skip_serializing_if` false）。`ai_chat` 在解析出的供應商型別是 `chatgpt-web`、且訊息裡有圖片時設為 true（純聊天與帶工具兩條路徑都要）。

### 前端

- `src/ipc/ai.ts` `AiChatReply` 加 `images_dropped?: boolean`。
- `useMcpChat`：收到 `images_dropped` 時，在該輪 AI 回覆之前加一則 assistant 訊息 `t.ai_images_dropped`（中英文：「這個供應商不支援看圖，圖片沒有送出，只送了文字。要讓 AI 看圖請換其他供應商。」）。同一輪只加一次。
- `AiPanel` `processFiles`：超過 5MB 的圖片不再無聲略過，輸入框上方顯示 `t.ai_image_too_large(names)`（中英文），下一次成功附加或送出時清掉。

## 測試

- `content.rs`：字串、陣列（文字＋圖片）、非 data URI 的圖片、null、未知段落。
- 每家各一條請求組裝測試，斷言組出來的 JSON 形狀（Anthropic 兩條路徑、Ollama、Codex、Antigravity、ChatGPT 網頁版不含 base64）。
- `ai_chat` 的 `images_dropped`：chatgpt-web＋圖片＝true；其他供應商或沒有圖＝false（若 `ai_chat` 難以單元測試，把判斷抽成純函式測）。
- 前端：`images_dropped` 會出現提示且只一次；超過 5MB 會顯示提示、沒有附上。
- 每個新測試先證明會紅。
- 探勘測試 commit 進去（`#[ignore]`）；額度恢復後跑 `PROBE_IDS=GPT5.6,Gemini2.5 cargo test --test image_input_probe -- --ignored --nocapture --test-threads=1`，結果補回本 spec。

## CHANGELOG

只寫已實測的：Anthropic 與 Ollama 附圖修好；ChatGPT 網頁版會提示不支援；超過 5MB 會提示。Codex／Gemini（Antigravity）等驗證後再寫。

## 不做

- ChatGPT 網頁版的圖片上傳。
- 非 data URI（http 網址）的圖片。
- 圖片縮圖／壓縮、歷史訊息裡的舊圖片去重（每輪都會重送舊圖，沿用現況）。

## 實作時補充的決定

- Ollama：只要 content 是陣列就轉成字串（＋`images`），不只在有圖片時——只附文字檔／其他檔案時前端也送陣列，Ollama 一樣會拒收。
- `images_dropped` 只看「最後一則 user 訊息」（這一輪送出的），不看整段歷史，否則附過一次圖之後每一輪都會提示；也不看最後一則訊息，因為工具迴圈後面幾輪歷史以工具結果結尾。
- 陣列解析不出任何段落時（空陣列、只有 http 圖片網址），Anthropic／Codex／Antigravity 改送一段空文字，避免 `content: []` 被拒收。
- `ai_chat` 取得實際供應商型別：新增 `AiRouter::provider_type`，與 `resolve()` 共用預設供應商的選法（`default_provider_id`）。
- 超過 5MB 的提示用 `aiterm-mode-hint--degraded` 樣式顯示在輸入框上方，下一次成功附加或送出時清掉。
- 「圖片被略過」的提示是對話裡的一則 assistant 訊息（同 `mcp_tool_limit`），之後幾輪會隨歷史送回模型。

## 實作後驗證（2026-10-14，走 App 實際程式碼路徑，左黃右紫測試圖）

- 正確：Anthropic 訂閱（純聊天、帶工具）、本機 Ollama gemma4（純聊天、帶工具）、oMLX gemma-4、Copilot、Gemini API 金鑰。
- 未驗證：Codex（額度用完）、Gemini Antigravity（設定裡的 `Gemini2.5` 供應商已不存在，無法再測）。
