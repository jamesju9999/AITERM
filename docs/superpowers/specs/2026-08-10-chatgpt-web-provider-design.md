# ChatGPT Web 供應商設計規格

**日期**：2026-08-10
**狀態**：設計完成，待實作
**參考實作**：OmniRoute v3.8.50 的 `chatgpt-web`（只當作踩坑地圖，不取用其程式碼）

## 目標

讓 AITerm 能使用 ChatGPT **網頁版**訂閱的模型，涵蓋兩個消費端：

1. `/ai` 指令與聊天面板（走 `AiProvider` trait）
2. Claude Code 橋接（走 `BridgeUpstream` trait）

這與既有的 `ProviderType::Codex` 是**不同的路徑**：Codex 走 `chatgpt.com/backend-api/codex/responses`（Responses API、原生 function calling），本規格走 `chatgpt.com/backend-api/conversation`（網頁前端自己的後端、無原生工具呼叫）。兩者吃同一份訂閱額度。

## 為什麼值得做

網頁版有 Codex 端點拿不到的模型（`gpt-5.6-pro`、`gpt-5.6-thinking`、`o3` 等，實際可用者依帳號方案而定）。

## 風險（必須在 UI 明示）

- **違反 OpenAI 服務條款**：以程式化方式驅動消費者網頁介面。帳號有被停權的風險。
- **工具呼叫是模擬的**：網頁後端沒有 function calling，靠 prompt 契約 + 回覆剖析。
- **持續性維護成本**：sentinel 的工作量證明演算法是社群逆向的產物，OpenAI 變更即失效（整個 provider 回 403）。

使用者選擇此 provider 時必須看到以上三點。

---

## 探勘實證

以下全部在 macOS + Tauri 2.10.3 的 WebviewWindow 內實測取得（探勘程式碼見 `probe/chatgpt-web` 分支，實作時移除）。

### 四層防護，逐層驗證結果

| 層 | 從外部 HTTP client | 從 webview 內 |
|---|---|---|
| Cloudflare TLS 指紋 | `403` + `cf-mitigated: challenge`（未帶憑證即被擋） | `200` + `cf-mitigated: null` |
| 頁面 CSP | — | 腳本注入不受限；`connect-src` 會擋往 `127.0.0.1` 的 fetch |
| Bearer 認證 | — | `/api/auth/session` 以 cookie 換 `accessToken` |
| Sentinel PoW | — | 兩段 `chat-requirements` + SHA3-512，69 次雜湊／181ms |

**關鍵結論**：webview 內部不需要任何 TLS 指紋偽裝，也不需要取出或儲存 session cookie。

### 認證

`GET /api/auth/session` → `{ accessToken, user: { email }, ... }`。
後續所有 `backend-api` 請求帶 `Authorization: Bearer <accessToken>`。

**沒帶 Bearer 的症狀**：sentinel 回應的 `persona` 是 `chatgpt-noauth`，對話請求則是 `403 {"detail":"Unusual activity has been detected from your device."}`。帶了之後 `persona` 變成 `chatgpt-freeaccount`（依方案而異）。

### Sentinel 流程

```
POST /backend-api/sentinel/chat-requirements/prepare
  body: { p: "gAAAAAC" + base64(JSON(config)) }
  → { persona, prepare_token }

POST /backend-api/sentinel/chat-requirements
  body: { p: <同上>, prepare_token }
  → { persona, token, proofofwork: { required, seed, difficulty } }

POST /backend-api/conversation
  headers:
    authorization: Bearer <accessToken>
    openai-sentinel-chat-requirements-token: <token>
    openai-sentinel-chat-requirements-prepare-token: <prepare_token>
    openai-sentinel-proof-token: "gAAAAAB" + base64(JSON(解出的 config))
  body: { action, messages, model, parent_message_id,
          websocket_request_id, conversation_mode: { kind: "primary_assistant" } }
  → SSE
```

`proofofwork.required` 實測為 `true`，未解的 token 會被拒（403）。`difficulty` 實測為 `06b931`、`073272` 這種 6 位十六進位值，命中機率約 2.6%，平均數十次雜湊即可。

### 工作量證明

`config` 是一個 18 元素陣列（螢幕尺寸、日期字串、UA、script src、`dpl`、語系、navigator/document/window 的隨機 key、`performance.now()`、UUID、核心數、epoch 位移…）。solver 把 `config[3]` 換成遞增計數器，計算 `SHA3-512(seed + base64(JSON(config)))`，取十六進位前綴與 `difficulty` 做字串比較，`<=` 即命中。

- prepare 階段：`seed = ""`、`target = "0fffff"`、前綴 `gAAAAAC`
- 對話階段：`seed`／`difficulty` 由伺服器給、前綴 `gAAAAAB`

**在頁面內執行的優勢**：config 需要的瀏覽器特徵全是真值。OmniRoute 因為跑在伺服器上必須捏造（假螢幕尺寸、假核心數、從硬編清單隨機挑 key），我們送出的指紋則與 OpenAI 看到的其他一切一致。

SHA3-512 不在 WebCrypto 裡，需自帶實作（已用 `""` 與 `"abc"` 的標準測試向量驗證）。

### 串流

`/backend-api/conversation` 回 SSE。實測「數 1 到 40」：21 個 chunk、每個約 500 bytes、首個 chunk 1295ms、總時長 3645ms。chunk 間隔 27–92ms。

**IPC 不是瓶頸** —— 該速率是上游送出的節奏，網頁版本來就不是逐 token 送。

### 模型清單

`GET /backend-api/models` 回**該帳號實際可用**的清單：

```json
{"models":[{"slug":"gpt-5-5","max_tokens":34834,"title":"GPT-5.5",
            "description":"...","tags":[...]}]}
```

因此不需要維護方案↔模型的對應表，登入哪個帳號就顯示什麼。`max_tokens` 也一併取得（免費方案實測 34834，遠小於 API 版本）。

---

## 架構

```
Claude Code ──HTTP──> 橋接 server (axum，既有)
                          │  ChatgptWebUpstream: BridgeUpstream
/ai、聊天面板 ─────────>  │  ChatgptWebProvider: AiProvider
                          ▼
                  chatgpt_web::Session
                          │  eval(JS) ↑↓ invoke(IPC)
                          ▼
                 隱藏的 WebviewWindow ──HTTPS──> chatgpt.com
```

### 檔案結構

| 檔案 | 職責 | 依賴 |
|---|---|---|
| `src-tauri/src/chatgpt_web/session.rs` | webview 生命週期、登入狀態、請求配對 | Tauri |
| `src-tauri/src/chatgpt_web/inject.js` | auth／sentinel／PoW／fetch／chunk 回傳 | 純瀏覽器 API |
| `src-tauri/src/chatgpt_web/protocol.rs` | ChatGPT SSE ↔ 中性事件；歷史攤平 | 無 |
| `src-tauri/src/chatgpt_web/tools.rs` | 工具契約序列化與封套剖析 | 無 |
| `src-tauri/src/ai/chatgpt_web.rs` | 實作 `AiProvider` | 上列 |
| `src-tauri/src/bridge/upstream/chatgpt_web.rs` | 實作 `BridgeUpstream` | 上列 |

`inject.js` 是獨立檔案而非 Rust 字串常數，Rust 端以 `include_str!` 引入 —— 這樣它的純函式（PoW、config 組裝、封套剖析）能用 vitest 測。

sentinel 與 PoW **只存在於注入腳本**，不放 Rust：config 需要的全是頁面內的真實瀏覽器值，跨到 Rust 就得偽造。

---

## 資料流

```
1. 消費端（橋接或 AiProvider）呼叫 Session::request(payload)
2. Session 配 request_id，存進 pending map，開一條 mpsc
3. webview.eval(`__aiterm.pull("<id>")`)          ← 只送 id
4. 注入腳本 invoke("chatgpt_web_take", { id })     ← 反向拉取 payload
5. 腳本：確保 accessToken → sentinel 兩段 → 解 PoW → fetch /conversation
6. 每個 chunk invoke("chatgpt_web_chunk", { id, data })
7. Session 推進該 id 的 mpsc；消費端轉成自己需要的型別
```

**第 3、4 步刻意分離**：Claude Code 的 system prompt 動輒 30K 字元，用 `eval` 拼進 JS 字串會踩上跳脫與長度限制。改為 eval 只傳 id、腳本反向拉取，payload 全程走 IPC 的結構化通道。

`request_id` 讓多個並行請求各自對應，不假設一次只有一個。

### 對話狀態：無狀態

每次請求都開新對話，把完整歷史攤平成單一 user turn。

**理由**：有狀態（維持 `conversation_id` + `parent_message_id`）**換不到上下文空間** —— 模型看到的仍是整段對話，`max_tokens` 上限照樣適用；省下的只是重新上傳的頻寬。而 Claude Code 的請求不是線性對話：它會 compact、編輯歷史、重試、派平行 subagent。對應到 ChatGPT 的訊息樹一旦對不上，就是上下文悄悄變成錯的 —— 最糟的失效模式，因為看不出來。

無狀態的失效模式只有一種：撞到上限、拿到明確錯誤。

### 上下文上限的處理

Claude Code 透過 `ANTHROPIC_DEFAULT_*_MODEL=aiterm:opus` 把該層當成 Opus，會假設有 200K∼1M 的空間，**不會主動 compact**，於是一路把對話養大直到撞牆。

採取的做法：

1. **誠實回報**：撞到就把上游原文帶回（`bridge/server.rs` 的 `error_text` 已會附上上游 body）。設定頁顯示該模型實際的 `max_tokens`。
2. **建議對應到 Haiku 層**：Claude Code 用 Haiku 跑背景小任務，請求短、不需長上下文。設定頁在選擇時給提示。

**明確不做**：橋接自行截斷歷史。那會讓 Claude Code 不知道自己的上下文被動過，推理莫名變差 —— 又是一種「悄悄變成錯的」。

---

## 工具模擬

`chatgpt_web/tools.rs` 一份實作，同時供兩個消費端使用：

- `AiProvider::generate_with_tools()` —— **要實作**，不留預設的 `Unsupported`。否則聊天面板帶 MCP 工具、Agent 迴圈、程式庫協助、知識庫問答這四處都會失敗。
- `BridgeUpstream`（Claude Code 橋接）

### 契約注入：雙位置

完整契約放在客戶端訊息**之後**（executor 摺疊 system 訊息後會落在區塊尾端），另在最新一則 user 訊息末尾掛一行提醒。

**依據**：OmniRoute #7679 的實測 —— 契約 prepend 在巨大 system 區塊開頭時，30K 字元 prompt 下 chatgpt-web 直接忽略它，回答「tool X is not in my tool set」，成功率 0/3；雙位置為 16/17（涵蓋 30K–250K prompt、30 個工具、多輪工具歷史、串流）。Claude Code 正是這個形狀。

### 封套與 nonce

每次請求產生隨機 nonce 嵌入契約，模型必須在 JSON 裡回帶 `_nonce`。剖析時：

- **只接受明確封套**（`<tool>{…}</tool>` 或 `<tool_call …>{…}</tool_call>`），不把裸 JSON 升級成工具呼叫
- 有 `_nonce` 但不符 → 視為文字，不執行
- 缺 `_nonce` → 容忍（模型未遵守指示），但仍需明確封套

**擋的是 prompt injection**：使用者貼進來的內容或程式碼若含 `{"name":"Bash","arguments":{…}}`，舊版會直接當成工具呼叫執行（OmniRoute #9343）。

### 工具結果的回填（先天限制）

因為無狀態、歷史攤平成單一 user turn，**工具結果只能以文字形式回填** —— ChatGPT 網頁版沒有 `role: "tool"` 這種結構化角色。

歷史中的工具回合以固定格式攤平：

```
[[tool_call:<name>#<id>]]
<arguments JSON>
[[/tool_call]]

[[tool_result:<id>]]
<結果內容>
[[/tool_result]]
```

**界定符刻意與模型要輸出的 `<tool>` 封套不同**。若兩者共用同一組標籤，剖析器可能把歷史裡我們自己寫進去的回合誤判成模型發出的新呼叫；nonce 檢查雖然也會擋下（歷史回合沒有 `_nonce`），但不該把正確性建立在第二道防線上。

比 OmniRoute `flattenToolHistory` 壓成 `[Tool result: …]` 散文的做法好在：有明確邊界、保留了呼叫與結果的對應 id。

**必須接受的後果**：多輪 agent loop 的保真度低於原生 tool calling。這是此傳輸路徑的先天限制，不是實作取捨。

---

## 登入與生命週期

- webview 視窗**延遲建立**（首次使用時），預設隱藏
- `/api/auth/session` 拿不到 token → **顯示視窗**讓使用者登入，同時回 `AuthFailed`
- **登入成功後自動隱藏**：顯示視窗期間輪詢 `/api/auth/session`（每 2 秒），一拿到
  `accessToken` 就 `hide()`。不做這件事的話視窗會一直開著，使用者很自然會去關掉它
- 視窗被關閉或崩潰 → 下次請求自動重建
- sentinel token 與 PoW **每次請求重算**（181ms，不值得為快取增加失效邏輯）

### 隱藏 vs 關閉

webview **就是傳輸層本身**（所有請求都是頁面內的 JS 發出的），所以兩者後果不同：

| 動作 | 後果 |
|---|---|
| `hide()` | webview 仍存活，請求照常運作。這是登入後的正常狀態。 |
| `close()` | webview 被銷毀，傳輸中斷；下次請求自動重建，但該次會多付一次載入成本。 |

**登入狀態不受影響**。Tauri 的 webview 資料儲存是應用程式層級，不隨視窗生滅，cookie 留在那裡。探勘期間視窗因 Rust 重建被銷毀重開六次，每次 `/api/auth/session` 都直接回 `hasToken: true`，使用者只登入過一次。

因此**必須使用預設資料儲存，不可用隔離分割區**。OmniRoute 的 Electron 登入管理器刻意每次開新的 `session.fromPartition('login-…-<timestamp>')`，是因為它的目的是把 cookie 撈出來；我們的目的相反，要的就是持久化。

### 錯誤對應

| 上游狀況 | 回給消費端 |
|---|---|
| 無 accessToken | `AuthFailed` +「請在 ChatGPT 視窗登入」 |
| sentinel 403 | `ModelError`，帶上游原文 |
| 用量上限 | `RateLimit` + 上游 detail |
| webview 不存在 | `Network`，並觸發重建 |

---

## 測試策略

| 層 | 測法 |
|---|---|
| `inject.js` 的純函式 | vitest：SHA3-512 標準向量、PoW 命中、封套剖析、nonce 拒絕、config 組裝 |
| SSE → 中性事件 | Rust 單元測試，餵錄下的真實 chunk |
| 歷史攤平／工具結果編碼 | Rust 單元測試 |
| 端到端 | `#[ignore]` 探勘測試（需真實登入，CI 跑不了），比照既有的 `codex_probe.rs` |

**明確不做**：對 chatgpt.com 的自動化整合測試。它需要真實憑證與登入狀態，且上游隨時變動，放進 CI 只會產生假紅燈。

---

## UI

- 新增 `ProviderType::ChatgptWeb`（顯示名稱「ChatGPT Web（網頁訂閱）」）
- 設定頁提供「登入 ChatGPT」按鈕（開啟 webview 視窗）
- 模型清單從 `/backend-api/models` 動態帶入，每個模型顯示其 `max_tokens`
- 風險提示三點（ToS、工具為模擬、維護成本）在選擇此 provider 時明示
- Claude Code 橋接設定頁：此 provider 可選，並提示建議對應到 Haiku 層

### 需要同步更新的既有窮舉點

新增 `ProviderType` 變體會影響數個 `match`。以下必須逐一檢查（歷史上因為漏改而壞過 9 個 provider）：

- `config/types.rs`：`Display`、序列化對應
- `ai/router.rs`：`default_base_url()`、`openai_chat_url()`
- `bridge/factory.rs`：`kind_for()`
- 前端 `ClaudeBridgePage.tsx` 的 `SUPPORTED_TYPES`
- i18n 字串（en / zh-TW）

---

## 已知限制

1. **工具為 prompt 模擬**，模型可能不照格式輸出；多輪保真度低於原生 tool calling。
2. **上下文受帳號方案限制**（免費方案實測 34834），且 Claude Code 不知情、不會主動 compact。
3. **需要一個常駐 webview**（約 50–100MB），且只在有 GUI 的環境可用。
4. **sentinel 演算法會被上游變更打破**，屆時整個 provider 回 403，需追上游修正。
5. **與 Codex provider 共用同一份訂閱額度**，切換介面並不會繞過帳號層級的用量限制。
