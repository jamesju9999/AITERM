# Server 系統連線技能（M3）

日期：2026-11-27　接續 `2026-11-26-server-skills-custom-design.md`（總覽與決策）與 `2026-11-17-system-connections-design.md`（桌面版設計）。

## 目標與已拍板

共用看板專案的 Agent 在 server 上也能操作外部**網頁系統**。已拍板：
- 登入方式：**從桌面版匯出 session 給 server**（使用者的系統登入後長時間有效）。帳密不離開使用者的電腦。
- 範圍：只做網頁系統；權限模式 `read_only`／`approve_each`／`autonomous`（`rules` 模式、桌面 App 型連線、配方、定時先不做）；
  即時預覽與人工接手是 M4。

## 設計

### 共用程式（`aiterm-systems` crate）

把桌面版 `systems/` 裡與 App 無關的部分抽出，桌面版 re-export，server 直接用：
`types`（連線、模式、規則）、`gate`（寫入判定）、`approvals`（核准等待）、`cdp`（Chrome 的 CDP 閘門）、`browser`（啟動 Chrome）、`proxy`（代理 Playwright MCP）、`audit`（審計寫入；配方與定時任務的資料表仍在桌面版）。
抽出時切掉的 App 相依（Playwright MCP 版本常數、啟動子行程的函式、MCP 初始化參數）改成參數或在 crate 內自備。桌面版既有測試全部保留並通過。

### 資料（server 的 intake.db）

- `board_systems(id, name, start_url, allowed_hosts JSON, mode, enabled, created_by, created_at, updated_at)`。
- `board_agents.systems TEXT`：被勾選的系統 id（JSON 陣列）。
- `board_system_sessions(system_id, cookies_sealed, exported_by, exported_at)`：cookie 用 `FileSecrets` 的加密保存（鍵 `syssession:<id>`），資料表只記是誰、何時匯出，**不含 cookie**。
- 審計用共用 crate 的 `system_audit` 表（建在 intake.db）；**待核准的請求只存在記憶體**（`Approvals`），並記下屬於哪個專案／卡片／Agent。
  server 重啟時等核准的請求本來就會消失（Agent 也已被結束），等同拒絕，所以不另存資料表。（原先規劃的 `board_sys_events` 沒有做。）

### 登入（匯出 session）

- 桌面版：使用者照原本方式在可見視窗登入一個系統（既有「登入」功能）。管理員按「分享 session 到 server」：
  AITerm 用 CDP `Network.getAllCookies` 取出該系統設定檔裡**屬於 `allowed_hosts` 網域**的 cookie（其他網域的不送），傳給 server。
- server：加密保存。每次要操作這個系統時，啟動一個**全新、隔離的 Chrome 設定檔**，用 CDP `Network.setCookies` 載入，再開始工作；結束就刪掉設定檔。
- 失效偵測：導到白名單外或看起來是登入頁（沿用桌面版 `looks_logged_out`）→ 暫停這張卡，畫布與卡片顯示「需要重新分享 session」，Agent 不得嘗試自己填密碼。重新分享後卡片可重試。
- 管理員可以看到 session 是誰、何時匯出，可以刪除。cookie 本身永遠不經 API 回傳。

### 給 Claude 的工具

延用 M2 的內建 MCP 端點：每張卡的 token 對應的環境多一項 `systems`。端點新增 `sys_*` 工具（桌面版的目錄：snapshot／click／type／navigate／screenshot…，
一律多 `system`、`intent` 兩個必填參數；`browser_evaluate`、`run_code`、檔案上傳等能繞過畫面操作的不開放）。
工具呼叫轉給該卡專屬的 Playwright MCP 子行程（`--cdp-endpoint` 連到 server 上的 Chrome）。`--allowedTools` 加 `mcp__aiterm-systems`。

### 寫入閘門與核准

- 沿用桌面版第一層：CDP `Fetch.requestPaused` 攔下非 GET 請求，依連線的模式判定；頂層導覽到白名單外、非 GET 送往白名單外一律拒絕。
- 第二層：`intent` 宣告，缺少就回錯誤；核准卡顯示「AI 想做：… → POST /orders」，宣告為讀卻觸發寫入會標警示。
- `approve_each` 的待核准併進 M2 已有的畫布「待核准」面板（資料庫寫入與系統請求同一個清單，標示類型）；等待時 Playwright 的逾時改寫成「正在等人核准」；逾時（5 分鐘）視為拒絕。
- 每個動作寫一筆 `board_sys_events`；管理員可從畫布查看。
- 卡結束或 server 重啟：該卡等待中的請求一律拒絕，Chrome 與 Playwright 子行程一併結束（沿用驗收者的孤兒行程清理）。

### 桌面版畫面

- 共用專案畫布技能區顯示「系統連線」；拖到 Agent 開對話框，勾選 server 上的系統；管理員可以「從本機分享系統」（只送設定：名稱、起始網址、白名單、模式）與「分享 session」、刪除。
- 待核准面板泛化成「待核准」：資料庫寫入與系統請求。

### 已知限制（寫進說明）

- session 過期要重新分享；綁定裝置／IP 或單處登入的系統不適合。
- 用 GET 改資料的舊系統，第一層攔不到。
- server 上的 Chrome 是**無頭**；沒有即時預覽與接手（M4）。
- 偵錯埠只綁 127.0.0.1，同一台主機上的其他程式連得上。
- 需要 Node.js（Playwright MCP）與 Chrome；沒有時提示，沿用驗收者的偵測與安裝說明。

### 測試與驗證

- 抽出的 crate：桌面版原有的單元與整合測試全部沿用。
- server：連線與 session 的 CRUD 與權限（成員看不到主機與 cookie）、cookie 只送白名單網域且加密保存、`sys_*` 工具與閘門（用桌面版的測試站）、核准流程、失效偵測、卡結束清理。
- 真實驗證：真的 Claude Code 操作測試站（唯讀擋下送出、核准後送出）；用你的真實系統做一次 session 匯出需要你在場。
