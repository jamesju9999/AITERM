# OpenAI 相容橋接伺服器 — 設計

日期：2026-09-30
狀態：已實作（分支 feat/openai-compatible-server），真供應商尚未實機驗證

## 問題

使用者想把 AITerm 當成橋接伺服器：對外提供 OpenAI 相容 API，讓 Cursor、Cline、Open WebUI、自寫腳本等外部工具使用，背後實際呼叫 AITerm 已經設定好的任一 AI 供應商（訂閱 OAuth、API key、本地模型皆可）。

現況：`src-tauri/src/bridge/` 已有綁 `127.0.0.1:8317` 的 axum server，但只實作 Anthropic Messages 入口（給 Claude Code 用）。設計背景與各家協定陷阱見 `2026-08-07-claude-code-bridge-design.md`，動手前必讀。

## 範圍

**含：**

- OpenAI 入口：`POST /v1/chat/completions`（串流與非串流，含 tool calling）、`GET /v1/models`
- 自訂模型別名表：對外名稱 → 供應商＋模型
- 綁定範圍可選：`127.0.0.1`（預設）或 `0.0.0.0`（區網）
- 一律要求 API key
- 設定頁 UI（中英文）

**不含：**

- 改動現有 Claude Code 橋接（8317 埠）的行為
- `/v1/embeddings`、`/v1/images`、`/v1/audio`、Responses API
- 公網暴露、TLS 終結（使用者自行用反向代理處理）
- 依 API key 分用戶、用量配額

## 架構

### 重用現有 bridge 的中立事件

現有 `bridge/upstream/` 已把各上游收斂成中立事件（`TextDelta`、`ThinkingDelta`、`ToolUseStart`、`ToolInputDelta`、`ToolUseEnd`、`Done`）。新入口只新增兩件事：

1. **請求端**：把 OpenAI `chat.completions` 請求轉成 bridge 內部的請求型別。
2. **回應端**：新增一份 OpenAI SSE 序列化器，把中立事件輸出成 `chat.completion.chunk`；非串流時累積事件後輸出單一 `chat.completion`。

上游 adapter（openai / anthropic / codex / antigravity）不改。憑證解析與端點知識沿用 `router.rs` 與共用自由函式，逆向端點知識仍然只有一份。

### 模組佈局（新增）

```
src-tauri/src/bridge/
  openai/
    request.rs        傳入 ChatCompletionRequest 的 deserializer
    response.rs       OpenAI SSE / 非串流 serializer（唯一一份）
    models.rs         /v1/models 回應
  alias_map.rs        對外別名 → (provider_id, model)
```

### 埠與生命週期

OpenAI 入口用獨立埠（預設 8319；8317 是 Claude Code 橋接、8318 是 MCP tool server），獨立開關，不影響 8317。兩個 server 共用 `BridgeState` 的啟停機制與上游層，但各自有自己的 router 與綁定設定。

## 模型別名表

- 設定內一份清單：`{ alias, provider_id, model }`。
- `/v1/models` 只列出別名，格式為 OpenAI 的 `{object:"list", data:[{id, object:"model", owned_by}]}`。
- 收到未登錄的 model：回 404，`error.code = "model_not_found"`。
- 別名不可重複；別名指向的供應商被刪除時，該別名在 UI 顯示為失效，請求回 404 而非 500。

## 網路與安全

- 預設綁 `127.0.0.1`。設定開關可改為 `0.0.0.0`，預設關閉。
- 兩種模式都必須帶 `Authorization: Bearer <key>`；缺少或錯誤回 401（OpenAI 格式）。
- API key 常數時間比對；存放於系統 keyring，不寫入 JSON 設定；UI 提供產生與重設。
- 綁區網時：
  - 設定頁顯示風險警告：這等於把使用者的訂閱與金鑰對外開放。
  - 處理 CORS（預設不放行任意來源，僅回應 preflight 給瀏覽器型客戶端所需的標頭）。
  - Windows 首次綁定會出現防火牆提示，說明文字需寫清楚。
- 請求大小上限與逾時要設定，避免區網內單一客戶端拖垮 server。

## 翻譯要點

- OpenAI `tools` / `tool_calls` / `role:"tool"` 訊息 ⇄ 中立事件的 tool_use；串流時 `tool_calls[].function.arguments` 以片段輸出。
- `finish_reason`：`stop` / `length` / `tool_calls` 由 `Done.stop_reason` 對映。
- `usage` 由 `Done.usage` 填入；串流時僅在客戶端要求 `stream_options.include_usage` 時輸出。
- 上游不支援的參數（`response_format`、`logit_bias` 等）忽略並記 log，不報錯；`n>1` 回 400。
- 圖片內容：上游支援才透傳，否則回 400 並說明原因。
- 錯誤一律轉成 `{error:{message,type,code}}`，HTTP 狀態碼對映：認證 401、額度 429、上游失敗 502、逾時 504。
- 上游補工具呼叫能力的既有前置工作（Codex、Antigravity）沿用 Claude Code 橋接已完成者，不重做。

## 前端

設定頁新增「OpenAI 相容伺服器」區塊：

- 啟用開關、埠、綁定範圍（含風險警告）
- API key 產生／重設／複製
- 別名表編輯（供應商與模型下拉沿用現有選擇器）
- 狀態燈（沿用 `/health`）與範例 curl／base_url 顯示
- 字串加入 `src/lib/i18n.ts`，en 與 zh-TW 兩邊都要補（en 由 `{...zhTW,...enRaw}` 合併，tsc 抓不到漏字串，測試須比對 `localeSources`）

## 錯誤模式提醒

- 端點位址：沿用逐 `ProviderType` 窮舉 match（`router::openai_chat_url`），不要從 URL 形狀推導。
- 別名解析放在請求入口單點處理，不要在各上游 adapter 內各自處理。

## 測試

- 單元：請求／回應 serializer 的往返、別名解析、API key 驗證、錯誤格式對映。
- 整合（wiremock 假上游）：每個上游家族各一條串流＋工具呼叫往返；`stream:false` 路徑；401 / 404 / 上游 429。
- 端到端真供應商：Codex、Gemini、Copilot 各一個，使用 openai 官方 SDK 或 curl 對 AITerm 發請求。
- 跨平台：綁定行為與 keyring 在 macOS、Windows、Linux 皆需可運作。

## 待實作前確認

- （已解決）8318 已被 MCP tool server 占用，改用 8319，埠可在設定頁修改。
