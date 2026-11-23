# 問題收集：LINE 當作第四個來源（診斷對談）

## 1. 目標

使用者在 LINE 傳訊息給官方帳號，訊息進 Intake 佇列，循序派給 Claude Code 診斷，答覆再用 LINE 回給對方。
沿用 Intake 既有的白名單、等級、派送、回覆規則，不另設規則。

**範圍外（這一期不做）**：AnyDesk 連線與遠端操作、圖片與檔案、群組／多人聊天室、LINE 顯示名稱。

## 2. 已拍板的決定

- webhook 由**使用者自備通道**（Cloudflare Tunnel／ngrok）對外；AITerm 只在本機收，預設只綁 127.0.0.1。
- 模式**沿用 Intake 現有規則**（內部預設自動派送、外部預設手動，回覆自動或存草稿，路由 Terminal／Board／Canvas／System 皆可）。
- 沒有「主人」概念：LINE 沒有像 Telegram 的 `/ask`，所有寄件者都必須在白名單。

## 3. 設計

### 3.1 模組

| 檔案 | 責任 |
|------|------|
| `intake/line_source.rs` | 純邏輯：簽章驗證、webhook JSON 解析、`ingest`／`route`、回覆目標還原、切段。不碰網路與 Tauri。 |
| `intake/line_server.rs` | axum 小伺服器（`POST /line/webhook`）、啟動／重啟、呼叫 LINE push API。 |
| `intake/runtime.rs` | `send_reply` 加 `line` 分支。 |
| `intake/rules.rs` | `valid_address` 接受 `line:<userId>`。 |
| `intake/config.rs` | `Config.line_enabled`；`GlobalConfig.line_port`（預設 8321）、`line_default_project` 不做（YAGNI，見 3.4）。 |
| `intake/commands.rs` | `intake_line_status`、`intake_line_set_credentials`、`intake_line_clear_credentials`。 |
| 前端 `IntakeView` | 通道分頁新增「LINE」：開關、埠號、憑證輸入、Webhook 網址提示。 |

### 3.2 寄件者代號

- 白名單與項目的 `sender` 用 `line:<userId小寫>`。LINE 的 userId 是 `U` ＋ 32 位十六進位。
- `rules::normalize` 會轉小寫，所以存小寫；`reply_target` 還原時把第一個字元改回大寫 `U`。
- `valid_address` 只接受 `line:u` ＋ 32 位十六進位（大小寫不拘）。
- 項目的 `message_id` 為 `line:<userId>:<LINE 訊息 id>`，供去重（webhook 重送不會入兩次）。

### 3.3 接收

- `POST /line/webhook`，body 上限 1 MB。
- **每個請求先驗 `X-Line-Signature`**：`base64(HMAC-SHA256(channel_secret, 原始 body))`，以常數時間比對；缺 header、驗不過、沒設 secret 一律 401，不讀 body 內容。
- 驗過後回 200，事件逐一處理。只處理 `type=message`、`source.type=user`、`message.type=text` 的事件；其餘（貼圖、圖片、加好友、群組等）忽略。`events` 為空（LINE 後台的 Verify 按鈕）直接 200。
- 路由：訊息所屬專案 ＝ 啟用 LINE 且該 `line:<id>` 在其白名單內的專案。同時在多個專案 → `ambiguous_project` 丟棄並計數；不在任何白名單 → `not_allowed` 丟棄並計數，**不回覆**（不洩漏機器人在聽）。
- 入列後觸發 `intake://changed`。

### 3.4 為什麼沒有 reply token

LINE 的 reply token 時效短（約一分鐘內要用掉），而 Claude Code 診斷通常更久；若兩條路都做，絕大多數回覆仍然會走 push。所以**一律用 push API**，邏輯單純、不必保存 token。代價：push 受官方帳號方案的月訊息額度限制，超額時 LINE 回 429，`send_reply` 以明確錯誤回報（項目停在「待寄出」草稿，可重試），不默默失敗。

因此也不做「LINE 預設專案」：沒有主人概念，所以不需要預設專案。

### 3.5 回覆

- `runtime::send_reply` 新增 `item.source == "line"` 分支；內容先過 `filter::redact`。
- push 到 `https://api.line.me/v2/bot/message/push`，`Authorization: Bearer <channel access token>`。
- 單則文字上限 5000 字元；以 4500 字元切段（沿用 `telegram_source::split_message` 的行為），每次請求最多 5 則，多於 5 則分多次請求。
- 任何一次非 2xx 即回錯誤，附 HTTP 狀態與 LINE 回的 `message`。

### 3.6 設定與憑證

- 憑證存系統鑰匙圈（`SecretStore`）：`line_channel_secret`、`line_channel_access_token`。**憑證不跨 IPC 回傳**，前端只知道「已設定／未設定」。
- `intake_line_status` 回傳 `{ has_secret, has_token, port, listening }`。
- 埠號是全域（一個伺服器）；存在 `GlobalConfig.line_port`。
- 任一專案 `line_enabled` 且已有 secret 才開伺服器；設定變更或憑證變更後 `line_server::restart`。
- 綁定失敗（埠被占用）記 log，狀態 `listening=false`，前端顯示提示。

### 3.7 安全

- 簽章驗證是唯一的入口驗證，缺一不可；測試涵蓋正確、竄改、缺 header、空 secret。
- 陌生人不回覆、不下載任何東西；只計數。
- 回覆過 `redact`。
- 憑證只在系統鑰匙圈，不寫入 intake.db、不進 log。
- 預設只綁 127.0.0.1；是否對外由使用者的通道決定。

### 3.8 跨平台

只用 axum、reqwest、sha2/hmac，皆為跨平台純 Rust；沒有平台專屬 API。

### 3.9 測試

- `line_source`（單元）：簽章（對／竄改／缺／空 secret）、`valid_address`、userId 大小寫還原、事件解析（文字／貼圖／群組／空 events）、白名單與陌生人、去重、多專案歧義、停用、切段。
- `line_server`（HTTP，用 `tower::ServiceExt`，仿 `web.rs`）：401／200、body 過大被拒、成功入列。
- push 呼叫：wiremock 驗證 header、body、切多請求、429 的錯誤訊息。
- 前端：LINE 分頁渲染、憑證輸入送出後清空、狀態徽章。

## 4. 實作紀錄

- 已完成並通過：後端 `line_source`（單元 11 條）、`line_server`（HTTP＋wiremock 8 條）、前端 LINE 分頁（2 條）與 `validAddress`（2 條）；`cargo test --workspace` 與 `npm run test` 全綠、`npx tsc -b` 乾淨。
- push 把只剩空白的段落（被切開的換行）濾掉：LINE 對空白訊息回 400。
- 前端另加 `Config.line_port`（與 `web_port` 一樣由全域設定回填）。
- **未自動測試**：`line_server::restart` 的綁埠與讀鑰匙圈（需要真的 OS 鑰匙圈）；`restart` 以外的路徑都有測試。需用真的 LINE 官方帳號＋Tunnel 實機驗收。
