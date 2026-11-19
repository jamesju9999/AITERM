# 系統連線支援桌面 App

日期：2026-11-19　狀態：設計已與使用者逐段確認，使用者授權照設計一路做到完成

## 目標

「系統連線」目前只能操作網頁（Playwright MCP）。讓同一套機制也能操作傳統桌面 App
（ERP、WinForms／WPF 舊系統、macOS／Linux 原生 App），底層改走既有的電腦操作（`computer/`，
無障礙元件樹＋OCR＋滑鼠鍵盤）。連線設定、權限模式、審計、核准卡、定時任務、信件／問題收集、
「請 AI 操作」全部沿用。平台不限，三平台共用同一套邏輯。

## 使用者的決定

- 架構 A：同一份清單加 `kind: web | desktop`，不另開平行模組。
- 一個桌面連線綁**單一 App**，AI 的點擊、輸入、截圖只能作用在它；想碰別的 App 一律拒絕並記審計。
- 寫入判定：AI 每個動作宣告 `intent`（`讀：`／`寫：`），再用 `computer/policy.rs` 的危險字樣
  與危險按鍵交叉升級。沒有網路層可攔，漏判風險是已知代價，審計記實際動作。
- 登入：帳密存 OS 鑰匙串，AI 認出欄位、AITerm 填密碼，AI 看不到密碼。
- 無人執行（定時、信件／問題收集）：前置檢查，螢幕鎖定或使用者正在操作就延後，閒置後才開始，
  延後超過上限標記失敗並通知。
- 人機衝突：偵測到非 AITerm 發出的輸入就立即暫停任務，等使用者按「繼續」或「停止」。
- 第一期不做配方；定時與觸發直接讓 AI 依目標執行。

## 資料

`SystemConnection` 新增（全部 `#[serde(default)]`，既有資料不受影響）：

| 欄位 | 說明 |
|---|---|
| `kind` | `web`（預設）／`desktop` |
| `app` | 桌面類型：要綁定的 App（名稱、套件 ID，電腦操作用的那個識別字串） |
| `launch_app` | 桌面類型：App 沒開時是否由 AITerm 啟動，預設 true |

桌面類型不使用 `start_url`、`allowed_hosts`、`upload_dirs`（可為空）。帳密沿用
`login.rs` 存在鑰匙串的 `system-login:{id}`。`mode`、`rules`、`enabled`、`model` 照舊；
`rules` 在桌面類型的比對對象是 `method`＝動作（`click`／`type`／`key`／`login`／`*`）、
`url_pattern`＝元件標題（glob，`*` 任意），`allow`／`ask` 不變。

## 工具

`tools/list` 依連線種類給不同工具；`sys_list` 多回 `kind` 與 `app`。

桌面工具名稱前綴 `sys_app_`，每個都必填 `system`、`intent`（沿用網頁版規矩），其餘參數對應
`computer_*`（去掉 `app`，由連線綁定）：

`sys_app_snapshot`、`sys_app_find`、`sys_app_read_screen`、`sys_app_screenshot`、
`sys_app_click`、`sys_app_click_text`、`sys_app_type`、`sys_app_key`、`sys_app_scroll`、
`sys_app_login`、`sys_app_report_login_needed`。

不給：`computer_open_url`、`computer_open_app`（切到別的 App）、`computer_batch`（逐步判定與暫停都靠單步）。
`sys_app_login` 參數：`username_element`、`password_element`（元件 id，來自 snapshot／find），
`submit`（`return`＝送出、省略＝不送）。

## 寫入閘門（動作層）

純函式 `decide_action(conn, action, label, declared_write)`：

1. 連線停用 → `Deny(Disabled)`。
2. 是否寫入：`declared_write`（intent 以「寫」開頭）、`policy::is_dangerous_label(label)`、
   `policy::is_dangerous_key`、按鍵為 Enter／Return（會送出表單）、`login` 一律視為寫入。
3. 非寫入 → `Allow`。
4. 寫入 → 依 `mode`：`ReadOnly` → `Deny(ReadOnly)`；`ApproveEach` → `Ask`；`Autonomous` → `Allow`；
   `Rules` → 第一條命中的規則，沒命中 `Ask`。

`Ask` 走既有 `Approvals`／核准卡（`method`＝動作、`url`＝App 與元件描述）。逾時視為拒絕。
`declared_read` 同網頁版：AI 宣告「讀」卻被升級為寫入，核准卡標警示。

綁定 App 檢查：每個工具呼叫的目標一律是 `conn.app`；`computer` 層回報目標 pid 不是該 App
（例如點擊座標落在別的視窗）時由 `ComputerService` 的 `app` 參數固定為綁定值，AI 無法指定別的 App。
總開關（設定 → 電腦操作 → 允許 AI 操作這台電腦）仍是最外層，沒開就回明確說明，不繞過。

`ComputerService` 的改動（向後相容）：
- `authorize_any` 先看「連線預先授權」集合（使用者建立／啟用桌面連線＝已授權該 App），不再彈首次使用對話框。
- `confirm_destructive` 在 task-local `PRE_CONFIRMED` 為真時略過（閘門已自己判定過寫入，不重複問）。
- 新增 `element_label`（取元件 role／title 供閘門使用）、`type_secret`（填密碼，要求焦點為密碼欄位，
  不回傳內容、不進審計明文）。

## 登入

- 帳密設定沿用 `systems_login_get／set`（網頁與桌面共用）。
- AI 看到登入畫面：呼叫 `sys_app_login(username_element, password_element, submit)`。AITerm 從鑰匙串取值，
  帳號用 `type_text`，密碼用 `type_secret`（目標欄位不是密碼型就拒絕，避免填錯地方）；
  審計只記「登入」與元件，不記值。回傳給 AI 的文字不含任何帳密。
- 一分鐘內不重複登入（同網頁版，避免帳密錯時鎖帳號）。
- 沒存帳密、驗證碼、二階段驗證：AI 呼叫 `sys_app_report_login_needed` → 狀態 `needs_login`，
  任務停下並通知；使用者登入後按「繼續」。

## 執行：無人檢查與人機衝突

新模組 `systems/desktop/presence.rs`，介面 `InputMonitor`（測試用假實作）：
`idle_secs() -> Option<u64>`、`screen_locked() -> Option<bool>`。三平台實作各自放在後端旁邊，
取不到的環境（例如 Wayland）回 `None`，功能降級並在 UI 提示「這個環境無法偵測使用者操作」。

- **無人執行前置檢查**（`JobsRuntime::run_goal` 對桌面連線）：螢幕鎖定、或 `idle_secs < IDLE_NEEDED`（預設 60 秒）
  → 延後 30 秒再查；累計超過 `MAX_DEFER`（預設 30 分鐘）→ 該次記為失敗（原因「一直沒有閒置時間」）並發通知。
  鎖定且 App 需要解鎖才能操作時同樣處理。
- **人機衝突**：每次動作結束記下 `last_own_input_at`；下一個動作開始前讀 `idle_secs`，若
  `idle_secs + 容許誤差 < 距上次自己動作的秒數`，代表其間有別人的輸入 → 狀態設為 `paused`，
  工具回「已暫停：偵測到使用者操作」並等待。使用者在 UI 按「繼續」才解除；按「停止」同現有 `stop`。
  暫停期間的所有工具呼叫立即回同一段說明，不執行任何動作。
- 無法取得 `idle_secs`（`None`）時不做衝突偵測，只保留「停止」按鈕。

## 其他連動

- `SystemsState::call`：依 `kind` 分派；桌面走 `DesktopRunner`（持有 `Arc<ComputerService>`、
  `InputMonitor`、暫停狀態），網頁路徑完全不動。
- `ensure_ready`：桌面類型＝確保 App 在跑（`launch_app` 為真則 `open_app`）；自動登入改由 AI 驅動，所以不在這裡登入。
- `stop`／`close`／`set_manual`／`status`：桌面類型的對應行為——`stop` 拒絕所有等待中核准並中止執行中的任務，
  `close` 清狀態（不關使用者的 App），`manual` 等同暫停，`status` 的 `needs_login`、`pending` 照舊。
- `build_prompt`：桌面版提示說明可用工具、「只使用 sys_app_* 工具」、登入規則、暫停規則。
- 儲存檢查 `validate`：桌面類型要求 `app` 非空且是安全的 App 名稱，且不檢查網址與網域。
- 前端 `SystemsView/ConnectionForm`：新增類型選擇；桌面時顯示 App 欄位與「沒開時自動啟動」，隱藏網址與網域；
  列表顯示類型徽章；審計與核准卡的文字對桌面動作顯示「點擊 按鈕「刪除」」而非 HTTP 方法與網址。
- i18n：`zh-TW` 與 `en` 兩份都補，並過 locale 漂移測試。

## 不做

配方（桌面）、跨 App 操作、多視窗／多螢幕的精細控制、驗證碼自動處理、Wayland 的閒置偵測（降級）。

## 測試

- 純函式：`decide_action` 全模式矩陣、危險字樣升級、Enter 升級、規則比對、`validate`。
- 假 `Backend`＋假 `Approver`＋假 `InputMonitor`：綁定 App 強制、預先授權不彈首次對話框、
  destructive 不重複詢問、`type_secret` 拒絕非密碼欄位、登入流程不外洩密碼、暫停與繼續、無人檢查的延後與逾時。
- 回歸：網頁系統既有測試全數維持綠燈；前端 Vitest 與 locale 漂移測試。
- 人工驗收（三平台各一次）：真實 App 一個唯讀動作、一個寫入核准、人為動滑鼠觸發暫停、鎖屏觸發延後。
