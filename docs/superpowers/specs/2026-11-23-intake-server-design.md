# 問題收集 Server 版（無頭 server＋AITERM Client 模式）— 設計

日期：2026-11-23
狀態：待使用者核可

## 問題

問題收集（LINE、Email、Telegram、網頁表單 → 佇列 → Claude Code 診斷 → 回覆）現在整套跑在某一個人的 AITERM 視窗裡。同事看不到這份清單與處理狀態，也無法接手；那台電腦關機，LINE 與信件就沒人收。

使用者的需求：

> 我想要讓這些 AITERM 的服務變成 server 版本，而原本 AITERM 就可以變成 client 連到 server，可以讓大家看到相同的問題清單處理狀態，或是可以著手進行處理。

## 決策紀錄（使用者在 brainstorm 中選定，實作時不要「順手改回來」）

| 決策 | 選定 |
|---|---|
| 「著手處理」由誰執行 | **兩種都要**：Server 自己用 Claude Code 處理；成員也可在自己的 AITERM「領取」後用自己機器處理並回報 |
| Server 形態 | **獨立的無頭 server 程式**（沿用 `aiterm-host` 的做法），不是「AITERM 開伺服器模式」 |
| 連線與認證 | **沿用遠端終端機的機制**：TLS＋驗證碼（SAS）配對；配對後發長期 token |
| 網路情境 | **內網／VPN 與公開網路都支援**：協定相同，只差憑證來源 |
| Server 的 AI | **Server 自己設定 AI 供應商與金鑰**，行為與本機相同（使用者選 A，否決「② 先不啟用 AI」）。理由：AI 分類有兩處（見下），沒有 AI 時群組會太吵、其他管道全被當成要改程式 |
| 範圍 | 拆成四個子專案依序做（見下）。**本 spec 詳細設計 ①②，③④只定介面與方向**，各自開工前再補 spec |

## 範圍切分

| # | 子專案 | 做完能做到 | 本 spec |
|---|---|---|---|
| ① | 抽出 `aiterm-intake-core` crate，AITERM 行為不變 | 使用者看不到變化；地基 | **詳細** |
| ② | `aiterm-intake-server`＋Client 模式（看、回覆、核准、改狀態、設定） | 大家看到同一份清單與狀態；server 24 小時收訊 | **詳細** |
| ③ | Server 自動處理（無頭跑 Claude Code） | Server 自己診斷並回覆 | 只定介面 |
| ④ | Client 領取處理（claim／release／回報） | 成員接手處理 | 只定介面 |

**為什麼這個順序**：② 做完就達到「大家看到相同清單與狀態」，且不碰最難的「無頭跑 Claude Code」。② 完成後 server 能收訊但**不會自動處理**，題目停在待處理，直到 ③ 或 ④ 上線；這是已知且接受的中間狀態（見「已知限制」）。

**明確不含（①②）**：自動處理、領取處理、AnyDesk 連線、知識庫檢索、多組織／多租戶、Server 的網頁管理介面、行動版 Client。

## 現況調查（設計的事實基礎）

以下是讀過程式碼確認的，不是推測：

| 事實 | 位置 |
|---|---|
| `intake/` 共約 9100 行；多數模組**已不依賴 Tauri**：`store`、`config`、`rules`、`dispatcher`、`line_source`、`line_store`、`line_text`、`telegram_source`、`mail_source`、`queue`、`refs`、`verify`、`tabcd`、`attach`、`classify`、`filter`、`smtp` 的 `AppHandle`／`tauri::` 出現次數為 0 | `src-tauri/src/intake/*.rs` |
| Tauri 耦合集中在 `commands.rs`（54 處）、`runtime.rs`（32）、`mod.rs`（9）、`line_server.rs`（8）、`web.rs`（6）、`board.rs`（1） | 同上 |
| 耦合的種類只有幾種：① `app.try_state::<IntakeDb>()` 取資料庫 ② `SecretStore` 取祕密（LINE 憑證、郵件密碼） ③ `app.emit(CHANGED_EVENT)` 通知前端 ④ `ProjectRegistry`（專案與其資料庫） ⑤ `AiRouter`（分類用的 AI） ⑥ `PtyManager`／工作看板／canvas／系統連線（派工） | `runtime.rs`、`mod.rs` |
| AI 分類有兩處，都經 `AiRouter` 送一小段提示詞（30 秒逾時）：① `classify::classify` 判斷題目是「問答」還是「要改程式」，決定走終端機回答或看板卡片（診斷模式下不呼叫 AI、一律問答；失敗預設要改程式）；② `classify::is_new_problem` 判斷 LINE 群組訊息是不是新問題（失敗預設是新問題） | `runtime.rs` 的 `classify_item`、`ai_is_new_problem`；`classify.rs` |
| `ai/` 目錄（約 7400 行）**沒有任何 `tauri::`／`AppHandle`**，`AiRouter` 只依賴 `ConfigStore`（設定）與 `SecretStore`（金鑰） | `src-tauri/src/ai/` |
| 前端已經用「`CHANGED_EVENT` 到了就重抓」的模式，不吃增量資料 | `src/components/IntakeView/index.tsx` |
| `aiterm-core` 已有 TLS（`tls.rs`）、SAS 配對（`auth.rs`）、WebSocket 協定（`protocol.rs`）、server（`server.rs`）、退避（`backoff.rs`），**但它的模型是「單一終端機分頁、每次連線都要主控端即時同意」** | `crates/aiterm-core/src/share/` |
| `aiterm-host` 是現成的無頭程式範本（clap、tokio、有 keyfile、有端到端測試） | `crates/aiterm-host/` |
| 工作區成員目前是 `.`、`aiterm-core`、`aiterm-host`、`aiterm-elevated-host` | `src-tauri/Cargo.toml` |

**要注意的差異**：遠端終端機是「即時同意、臨時、一對一」；Server 版是「常駐、多人、事先配對」。所以**重用的是 TLS 與 SAS 配對的密碼學部分，不是它的授權模型**——Server 版的授權是「配對時管理員同意一次，之後靠 token」。

## 架構

```
┌────────────┐   LINE webhook / IMAP / Telegram / 網頁表單
│ 外部管道    │ ─────────────────────────────┐
└────────────┘                               ▼
                                  ┌──────────────────────────┐
                                  │ aiterm-intake-server     │  無頭、常駐
                                  │  ├ 收訊（四個管道）        │
                                  │  ├ SQLite（佇列/設定）     │
                                  │  ├ API（HTTPS＋WebSocket） │
                                  │  └ 認證（成員 token）      │
                                  └──────────┬───────────────┘
                                             │ 使用 aiterm-intake-core
              ┌──────────────────────────────┼──────────────────┐
              ▼                              ▼                  ▼
        AITERM（Client 模式）          AITERM（Client 模式）   AITERM（本機模式，原樣）
        同一份清單與狀態                    …                  行為不變
```

三個單元，各自可獨立理解與測試：

1. **`aiterm-intake-core`**（新 crate）：資料與規則，不含任何 UI、視窗、Tauri、終端機。
2. **`aiterm-intake-server`**（新 crate，二進位）：`core`＋收訊連線＋API＋認證。
3. **AITERM 的 `IntakeBackend`**：Client 端的抽象層，把「問題收集」的操作分流到本機或遠端 server。

### ① `aiterm-intake-core`

**搬進去的**：上表「Tauri 次數為 0」的模組（`store`、`config`、`rules`、`dispatcher` 的提示詞建構、`line_source`、`line_store`、`line_text`、`telegram_source`、`mail_source`、`queue`、`refs`、`filter`、`smtp`、`classify` 的提示詞與解析等純邏輯）。實際清單在寫計畫時用 `cargo tree`／編譯結果確認，**以「搬完 `cargo tree -p aiterm-intake-core` 沒有 `tauri` 為準」**。

**不搬的**（留在 AITERM，因為它們就是「用本機視窗與終端機」）：`commands.rs`（Tauri 指令）、`runtime.rs` 中派到終端機分頁／看板／canvas／系統連線的部分、`verify.rs` 與 `tabcd.rs`（若確認它們綁終端機輸出）。

**與平台的接觸面改成一個介面** `IntakeHost`（依賴注入，取代 `app.try_state`）：

| 方法 | 現在是 | Server 實作 | AITERM 實作 |
|---|---|---|---|
| `secret(id) -> Option<String>` | `SecretStore` | 檔案金鑰（沿用 `aiterm-host` 的 keyfile 做法）；不可用系統鑰匙圈 | `SecretStore` |
| `changed()` | `app.emit(CHANGED_EVENT)` | 推播給所有已連線的 Client | `app.emit` |
| `ai() -> Arc<dyn AiProvider>` | `AiRouter` | server 自己的 AI 設定（見「Server 的 AI 設定」） | `AiRouter` |

派工（終端機／看板／canvas）**不放進 core 介面**：它是 ③④ 的事，而且本來就在不搬的部分。

**驗收**：
- `cargo test --workspace --no-fail-fast` 全綠，測試數不減少（搬移不得刪測試）。
- `aiterm-intake-core` 的相依樹沒有 `tauri`、`tauri-plugin-*`、`portable-pty`。
- 實機：本機模式收到 LINE、核准、回覆的流程與搬移前相同。
- **沒有任何使用者可見的變化**；CHANGELOG 不需要新增項目。

### ② `aiterm-intake-server`

**程式與指令**（clap）：

| 指令 | 作用 |
|---|---|
| `init` | 建資料夾、金鑰、自簽憑證、資料庫；產生第一個管理員配對碼 |
| `run` | 啟動：收訊連線＋API；前景執行，SIGTERM 優雅結束 |
| `pair` | 產生一組一次性配對邀請（管理員用） |
| `members list/revoke` | 列出／撤銷成員 token |
| `status` | 顯示各管道連線狀態與佇列數 |

**資料夾**（預設 `~/.aiterm-intake/`，可用 `--data-dir`）：`intake.db`、`secrets.key`（0600）、`tls/`、`members.db`。

**收訊管道**：LINE webhook（沿用 `line_server` 的 axum 路由與簽章驗證，綁定位址與埠可設定）、IMAP、Telegram、網頁表單。四個管道的收訊邏輯本來就是純邏輯＋一個小外殼，外殼在 server 裡重寫成「用 `IntakeHost`」的版本。

**API**（HTTPS，JSON，路徑帶版本 `/v1`）：

| 路徑 | 說明 |
|---|---|
| `POST /v1/pair/start`、`/v1/pair/finish` | 配對（見「認證」），唯一不需 token 的端點 |
| `GET /v1/items?status=&source=&q=` | 問題清單 |
| `GET /v1/items/{id}` | 單題詳情（含附件清單、回覆草稿、前情） |
| `GET /v1/items/{id}/attachments/{name}` | 附件下載 |
| `POST /v1/items/{id}/reply`、`/approve`、`/reject`、`/status` | 動作；等同現在的 Tauri 指令 |
| `GET /v1/senders`、`PUT /v1/senders/{key}` 等 | 白名單（含標籤） |
| `GET /v1/config`、`PUT /v1/config` | 管道設定（**不含祕密**，只回「已設定／未設定」） |
| `PUT /v1/secrets/{id}` | 設定祕密（管理員，只寫不讀） |
| `GET /v1/status` | 各管道連線狀態 |
| `GET /v1/events`（WebSocket） | 推播 `{"type":"changed","seq":N}`；Client 收到就重抓。斷線重連後用 `seq` 判斷要不要重抓 |

這些端點**一一對應 `commands.rs` 現有的 Tauri 指令**，所以 Client 端不需要新概念。

**推播用「changed＋重抓」而不是增量**：前端現在就是這個模式，改增量要動整個畫面的資料流，而且問題清單量不大。要求是「清單與狀態一致」，不是「毫秒級」。

### Server 的 AI 設定

`ai/` 沒有 Tauri 耦合，所以 server 直接重用同一份供應商實作，不另寫一套。①要把 `ai/`（含它依賴的 `ConfigStore`、`SecretStore` 的可攜部分）一起抽到可被 server 引用的 crate；若 `ConfigStore`／`SecretStore` 本身依賴 Tauri 或系統鑰匙圈，就在 crate 邊界換成 trait，由 AITERM 與 server 各自實作（server 用檔案金鑰）。

- **支援**：用 API 金鑰的供應商（OpenAI、Anthropic API、OpenAI 相容、Ollama 等）。
- **不支援（②）**：需要瀏覽器互動登入的訂閱 OAuth（Anthropic／Codex／Gemini 的訂閱登入）。無頭 server 沒有瀏覽器，這部分另開需求。
- **設定方式**：管理員用 API 設定（Client 的設定頁，admin 限定）或 server 的 CLI（`aiterm-intake-server ai set …`）；金鑰與其他祕密一樣只寫不讀、只存 server。
- **沒設定時**：行為與本機沒設定 AI 時相同（分類失敗的預設值），並在 `GET /v1/status` 與 Client 設定頁顯示「尚未設定 AI」，讓管理員知道群組自動開題與題目分類沒有生效。
- **費用**：Server 上的 AI 呼叫算 server 管理員的金鑰額度；稽核紀錄記錄分類呼叫次數。

### 認證與角色

**配對**（重用 `aiterm-core::share::{tls, auth}` 的 TLS 與 commit-then-reveal SAS）：

1. 管理員在 server 執行 `aiterm-intake-server pair --name 小明 --role member`，得到一組一次性邀請（含位址與一次性碼，**15 分鐘內有效、只能用一次**）。
2. 成員在 AITERM 輸入位址與邀請碼，兩邊各顯示 4 位驗證碼，**成員向管理員口頭核對相同**，管理員在 server 端按確認。
3. 成功後 server 發一組長期 token（隨機 256 bit），**只存雜湊**；Client 把 token 存進系統鑰匙圈。
4. 之後每個請求帶 `Authorization: Bearer <token>`。

**憑證**：
- 內網／VPN：`init` 產生自簽憑證；Client 在配對時**釘住憑證指紋**（SAS 本身綁定該 TLS 連線，因此能擋中間人）；之後只信這個指紋。
- 公開網路：`--tls-cert/--tls-key` 指定正式憑證；Client 走一般憑證驗證。
- 兩種模式協定完全相同，只差 Client 驗證憑證的方式，記在配對結果裡。

**角色**（兩種，不做更細）：

| 能做 | member | admin |
|---|---|---|
| 看清單、詳情、附件 | ✔ | ✔ |
| 回覆、核准、駁回、改狀態 | ✔ | ✔ |
| 改白名單與標籤 | | ✔ |
| 改管道設定、設定祕密（含 AI 金鑰） | | ✔ |
| 配對新成員、撤銷成員 | | ✔（在 server 本機 CLI） |

**防濫用**：配對端點每個來源 IP 限速並用 `backoff.rs` 的指數退避；token 錯誤一律回 401 且不透露是否存在；撤銷立即生效（下一個請求就 401，WebSocket 立即斷線）；所有動作記稽核紀錄（誰、何時、哪一題、做了什麼）。

**祕密不出 server**：LINE token、郵件密碼只存在 server；API 永遠不回傳祕密內容。

### Client 模式（AITERM 端）

**分流點放在 Rust 端，不放前端**：`commands.rs` 的每個指令先問 `IntakeBackend`：

- `Local`：走現在的路徑（行為不變）。
- `Remote(server)`：轉成對應的 HTTPS 請求；回傳形狀與 Local 相同。

這樣 `IntakeView` 與 `src/ipc/intake.ts` **幾乎不用改**：畫面、清單、狀態、元件都沿用。差異只在：

- 設定頁多一塊「連線」：顯示目前是「本機」或「server：xxx」，可配對、可切換、可中斷。
- Remote 模式下，管理員專屬功能（管道設定、祕密、白名單）對 member 隱藏或唯讀；顯示「這是 server 的設定」。
- Remote 模式下，**本機的收訊與派工整個停掉**（`runtime::install` 不啟動收訊／tick），避免同一封信被本機和 server 各收一次。切回本機才恢復。
- 連線中斷時：清單保留最後一次的內容並標示「已離線」，動作按鈕停用；WebSocket 用既有的退避重連，連上後重抓。

**Client 與 server 版本相容**：API 帶 `/v1`，握手時回報 server 版本與協定版本；Client 看到不認識的主版本就拒絕並說明「請更新 AITERM／server」。

## 資料與遷移

- Server 的資料庫**沿用現有 schema**（`store::init_schema` 等），不另設計一份，因此 core 搬移後兩邊用同一份遷移程式。
- 「本機 → server」的**資料搬遷不在本 spec**：先不做，使用者從乾淨的 server 開始（白名單與設定在 server 重設）。要匯入現有佇列時另開需求。
- **專案（project）**：本機模式的問題收集設定與資料庫掛在 `ProjectRegistry` 的專案下。Server 初期是**單一「專案」**（一份設定、一個資料庫）。多專案的 server 見「開放問題」。

## 介面（③④ 預留，不在本 spec 實作）

為了讓 ②→③④ 不用改協定，② 先把這些欄位與端點保留好（只存、不使用）：

| 項目 | 說明 |
|---|---|
| `items.claimed_by`、`items.claimed_at`、`items.claim_expires_at` | 領取者（`server` 或成員名）與逾時；② 一律為空 |
| `POST /v1/items/{id}/claim`、`/release`、`/result` | ② 回 `501 not implemented`；④ 實作。**領取必須是 server 端原子操作**（單一 SQL `UPDATE … WHERE claimed_by IS NULL`），逾時自動釋放 |
| `IntakeHost::dispatch` | ③ 才加入：Server 端怎麼跑 Claude Code（非互動模式）。此處不設計 |

## 已知限制

- **② 完成後 server 不會自動處理**：題目收進來、顯示在清單、可以人工回覆，但不會自動診斷，要等 ③ 或 ④。使用者要知道這個中間狀態。
- 「回覆」是以 server 身分送出（LINE／信件的寄件者是同一個），不是成員個人的身分。稽核紀錄會記是誰按的。
- 公開網路模式的安全性取決於憑證與 server 本身的維運（作業系統更新、防火牆）；程式只負責 token、速率限制與稽核，不替使用者做網路防護。
- Server 的 AI 只支援 API 金鑰類供應商，不支援訂閱 OAuth；沒設定 AI 時，LINE 群組每段安靜後都會被當成新問題、其他管道的題目都被當成要改程式（與本機沒設 AI 時相同），所以管理員應該設定。
- 一個 server 一份設定一個資料庫；不支援多組織隔離。

## 開放問題（寫計畫時先釘死，必要時回來問使用者）

1. ~~Server 上的 AI 分類~~：**已決定**，server 自己設定 AI 供應商與金鑰（見「Server 的 AI 設定」）。仍待確認：`ConfigStore`／`SecretStore` 能否直接搬進 crate，還是要換成 trait；用編譯結果判斷。
2. **多專案**：本機的問題收集是「每個專案各有設定」。Server 是單專案夠用，還是要對應多個專案？傾向單專案，多專案等真有需求。
3. **`verify.rs`／`tabcd.rs` 是否綁終端機**：決定它們留在 AITERM 還是進 core，用編譯結果判斷。
4. **網頁表單**：`web.rs`（1044 行）現在是 AITERM 內嵌的小 HTTP server。Server 版要沿用同一份程式，還是併進 API 的同一個埠？傾向沿用、獨立埠，改動最小。
5. **Server 的作業系統**：目標是 Linux（VPS／Docker）與 macOS／Windows 常駐機器都能跑。CI 是否要為 server 多一組發佈產物（比照 `aiterm-host` 的 `host-v*` tag 分家發佈）？傾向是，另開 `intake-server-v*` tag，**不與桌面版 Release 混在一起**。

## 測試策略

**①**：搬移前後的測試數對照；`cargo tree` 檢查 core 沒有 Tauri；既有 `intake` 測試（222 條）全數通過。

**②**：
- core／server 單元測試：認證（token 雜湊、過期邀請、重放、撤銷）、角色閘門（member 呼叫 admin 端點得 403）、API 端點與 Tauri 指令的對應。
- **整合測試**（真的起 server、真的 TLS、wiremock 模擬 LINE API）：配對→取清單→回覆→核准→改狀態→WebSocket 收到 `changed`。
- **中間人測試**：比照 `aiterm-host` 的 `cli_host.rs`，用假 Client 在配對時換憑證，確認 SAS 不符、配對失敗。
- **雙收防護**：Remote 模式下確認本機收訊與 tick 都沒有啟動。
- **前端**：`IntakeView` 在 Remote 與 Local 兩種後端下行為相同的測試；離線狀態的呈現。
- **實機驗收（必做，自動測試不能代替）**：起一個真的 server，兩台 AITERM 各自配對，確認同一份清單、一邊回覆另一邊即時看到、撤銷成員後立即斷線。CHANGELOG 只寫實機看過的行為。

## 實作順序（交給 writing-plans 細化）

1. 建 `aiterm-intake-core`，逐模組搬移，每搬一批就跑全部測試；定義 `IntakeHost`。
2. AITERM 改用 core 與 `IntakeHost`（本機實作），行為不變，驗收 ①。
3. 建 `aiterm-intake-server`：`init`／`run`、資料庫、收訊連線（先 LINE 與網頁表單，Email、Telegram 隨後）。
4. 認證與配對；API 端點；WebSocket 推播；稽核紀錄。
5. AITERM 的 `IntakeBackend`、Remote 實作、設定頁「連線」區塊、離線處理。
6. 整合測試、中間人測試、實機驗收。
7. 發佈產物與流程（`intake-server-v*` tag），文件（如何在 VPS 部署、如何配對成員）。
