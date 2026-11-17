# 系統連線（網頁系統）第一期：讓 AI 自主操作外部網頁系統

日期：2026-11-17　狀態：設計已與使用者逐段確認，使用者授權照建議一路做到完成

## 目標

讓 AITerm 能把「外部網頁系統」接進來，由 AI 自主操作（類似 Claude Cowork 的體驗）。
對象是**正式環境**，所以安全（寫入閘門、審計、一鍵停用）是第一優先，不是附加功能。

使用者的決定：
- 系統類型：網頁系統（API 系統不在這個範圍）。
- 觸發：用自然語言下目標（第一期）；定時、信件／Telegram 觸發是後面的期數。
- 寫入權限**可設定**，每個系統各自一個模式（見「權限模式」）。
- 登入方式因系統而異 → 統一做成「使用者手動登入一次，AITerm 保存 session」。
- 瀏覽器：AITerm 管理的獨立 Chrome（不借用使用者平常的瀏覽器）。
- 寫入判斷：**兩層**——瀏覽器網路層攔截（安全網）＋ AI 宣告每個動作的意圖（給人看）。
- 第一期要包含「即時看得到 AI 在做什麼、可隨時停止、可接手」。

## 分期（每期各自 spec → 計畫 → 實作）

1. **本文件：核心＋即時預覽與接手**
2. 配方沉澱：把成功的流程存成可重複執行的配方。
3. 定時執行（沿用 Agent 協作定時器的思路）。
4. 信件／Telegram 觸發（以問題收集 Intake 為入口）。

第一期**不做**：配方、定時、信件觸發、API 系統、多人共用憑證、App 內嵌的畫面串流預覽、
跨應用操作（檔案、桌面程式，現有電腦操作功能涵蓋一部分）。

## 架構

新增後端模組 `src-tauri/src/systems/`，不碰 Agent 協作。沿用資料庫技能（`tasks/db_skill.rs`）的形狀：
連線清單＋權限模式＋執行記錄。

```
AI（Ask AI 迴圈 / Claude Code）
   │ MCP 工具：sys_*（帶 system、intent）
   ▼
systems::gateway ── 驗證 intent、系統是否啟用、白名單 ──► 轉給 Playwright MCP（子行程）
   │                                                         │ --cdp-endpoint
   │                                                         ▼
systems::gate  ◄── CDP Fetch.requestPaused ───────────  AITerm 管理的 Chrome（有頭、獨立設定檔）
   │                                                         ▲
   └─ 判定：放行 / 拒絕 / 等人核准 ──► UI 核准卡 ─────────────┘ 使用者可直接在這個視窗接手
systems::audit：每個動作寫一筆記錄
```

關鍵取捨：
- **瀏覽器由 AITerm 啟動並開遠端偵錯埠，Playwright MCP 與 AITerm 的閘門各自用 CDP 連上同一個 Chrome。**
  這樣 AI 的每個請求都經過 AITerm 的攔截，不依賴 AI 守規矩；而且視窗是真的、看得到的，
  「即時預覽」與「接手」不用另做畫面串流——使用者直接用那個視窗。
- 工具介面**代理** Playwright MCP 的工具（snapshot、click、type、navigate、screenshot…），
  不自己重寫瀏覽器自動化。工具名加 `sys_` 前綴，並多兩個必填參數：`system`（哪個連線）、`intent`（見下）。
  預設不開放 `browser_evaluate`／`run_code`／檔案上傳這類能繞過畫面操作的工具。
- 偵錯埠只綁 127.0.0.1、隨機埠。這表示同一台電腦上的其他程式也連得上；列為已知限制。

## 系統連線設定

存在 AITerm 設定裡（不含任何憑證）：

| 欄位 | 說明 |
|---|---|
| `id`、`name` | 識別與顯示名稱 |
| `start_url` | 登入與起始網址 |
| `allowed_hosts` | 允許頂層導覽的網域白名單（含 `start_url` 的網域）。導向白名單外一律擋下並記錄 |
| `mode` | `read_only` / `approve_each` / `rules` / `autonomous` |
| `rules` | `rules` 模式用：`{method, url_pattern, action: allow\|ask}` 清單，沒命中的一律 `ask` |
| `enabled` | 一鍵停用；停用時立即關閉該系統的瀏覽器並拒絕後續工具呼叫 |

登入狀態：瀏覽器設定檔放在 AITerm 資料夾下每個系統各自一個目錄（權限限本人）。
**密碼不經過 AITerm 也不經過 AI**：使用者在可見視窗自己登入，AITerm 只保留 session（cookie）。
session 過期（偵測到被導回登入頁）→ 任務暫停並通知使用者重新登入，AI 不得嘗試自己填密碼。

## 寫入閘門（兩層）

**第一層：網路攔截（不依賴 AI）。** 對該 Chrome 的每個分頁啟用 `Fetch.requestPaused`：
- GET／HEAD／OPTIONS → 放行。
- 其他方法（POST／PUT／PATCH／DELETE…）→ 依 `mode` 判定：
  - `read_only`：直接拒絕（`BlockedByClient`），並回報 AI「被拒絕，不可重試」。
  - `approve_each`：暫停請求，產生核准卡，等使用者決定；逾時（預設 5 分鐘）視為拒絕。
  - `rules`：命中 `allow` 放行、命中 `ask` 或沒命中則同 `approve_each`。
  - `autonomous`：放行，仍寫審計。
- 頂層導覽到 `allowed_hosts` 以外 → 拒絕。非 GET 請求送往白名單以外網域 → 一律拒絕（不論模式）。

**第二層：意圖宣告（給人看）。** 每個會動作的 `sys_*` 工具都必須帶 `intent`（一句話：要做什麼、是讀還是寫）。
缺少就直接回錯誤。閘門把最近一次的 `intent` 附在隨後被攔下的請求上，核准卡因此顯示
「AI 想做：送出訂單 A-102　→　POST /orders」，而不是只有一條技術性的請求。
AI 宣告為「讀」卻觸發非 GET 請求，核准卡上標示警示（宣告與實際不符）。

核准卡：任何分頁都能看到（通知＋系統頁面），內容含 intent、方法、網址、當前頁面截圖，按鈕「核准／拒絕／拒絕並停用系統」。

已知限制（文件與畫面都要寫明）：
- 用 GET 改資料的舊系統，第一層攔不到，只剩第二層宣告與審計。
- 用 POST 做純查詢的系統（例如 GraphQL 查詢）在 `approve_each` 會很吵；用 `rules` 解。
- 偵錯埠對本機其他程式可見。

## 即時預覽、停止、接手

- 瀏覽器是有頭視窗，使用者本來就看得到 AI 的每個動作；AITerm 的系統頁面顯示目前狀態
  （執行中／等核准／已暫停）與最近動作清單。
- **停止**：按鈕立即中止目前這個 AI 任務、拒絕所有等待中的核准、保留瀏覽器開著。
- **接手**：按「我來接手」→ AI 的工具呼叫進入暫停（回覆「使用者接手中」）；使用者在視窗裡直接操作，
  做完按「交還 AI」，AI 取得最新畫面快照後繼續。接手期間的網路請求不經閘門判定，但仍寫審計（標示 `manual`）。
- App 內嵌的畫面串流預覽不在第一期。

## 審計

應用層 SQLite（`systems.db`，放在 app 資料夾）。每個動作一筆：時間、系統、工具、`intent`、
網址、請求方法、閘門判定（allow／deny／approved／rejected／timeout）、判定來源（rule／user）、截圖路徑、結果摘要。
系統頁面可查；截圖限制總量並輪替清除。

## 錯誤處理

- 被拒絕或使用者不核准：工具回覆明確的「被拒絕」，AI 必須改走別的做法或回報，不可繞過重試同一請求。
- 登入失效、驗證碼、元素找不到、頁面改版：暫停並回報，不盲目重試。
- 核准等待時工具呼叫自己逾時：閘門要把 Playwright 的逾時錯誤改寫成「正在等人核准」（見探勘項目 3），避免 AI 以為失敗而重送。
- 步數與時間上限沿用現有 Agent 迴圈機制。
- Chrome 沒安裝：沿用 `tasks::ask` 的偵測，沒有就提示安裝（Chrome for Testing 的安裝指令已存在）。

## 跨平台

macOS／Windows／Linux 都要能用：Chrome 啟動參數、設定檔路徑、行程清理都走既有的平台抽象；
不使用平台專屬 API。Playwright MCP 版本沿用 `tasks::ask::PLAYWRIGHT_MCP`（釘死）。

## 探勘結果（2026-11-17，macOS、真的 Chrome、`npx -y @playwright/mcp@0.0.83`，腳本在 scratchpad 的 `probe/probe.mjs`）

已證實：
1. `--cdp-endpoint http://127.0.0.1:<埠>` 能連上 AITerm 自己啟動的 Chrome（`--remote-debugging-port`＋獨立 `--user-data-dir`）。
2. 同一個 Chrome 有 Playwright MCP 與 AITerm 兩個 CDP 用戶端時，AITerm 用 `Target.setAutoAttach`（flatten）＋ `Fetch.enable` 能攔到 Playwright 觸發的 POST：
   - 回 `Fetch.failRequest(BlockedByClient)` → **伺服器的計數器維持 0**，被拒絕的寫入確實沒有送出。
   - 回 `Fetch.continueRequest` → 伺服器收到 1 次。
   - 實作注意：同一個分頁不可重複附加（探勘時對既有分頁附加了兩次，同一個 POST 被看到兩次）；每個 target 只建一個 session。
3. 請求被保留（等核准）期間，`browser_click` 會在約 5 秒後回 `TimeoutError`（Playwright 內部逾時，無法調長），請求仍被保留著。
   → 閘門代理必須：工具回 `TimeoutError` 且該系統有等待中的核准時，**不要把逾時回給 AI**；改成等核准結果
   （上限＝核准逾時），之後回覆「已核准／已拒絕」加上最新的畫面快照。
4. 被拒絕的 POST 會讓分頁落到 `chrome-error://chromewebdata/`。代理要偵測這種結果並改寫成
   「這個動作被閘門拒絕，不可重試」，並把分頁導回上一頁，免得 AI 看到一頁 Chrome 錯誤畫面而困惑。
5. 工具清單（0.0.83）：`browser_navigate`、`_click`（參數是 `target`，不是 `ref`／`element`）、`_type`、`_fill_form`、
   `_press_key`、`_select_option`、`_hover`、`_drag`、`_snapshot`、`_take_screenshot`、`_find`、`_wait_for`、`_tabs`、
   `_navigate_back`、`_handle_dialog`、`_console_messages`、`_network_requests`、`_network_request`、`_resize`、
   `_emulate_media`、`_close`，以及會繞過畫面操作的 `_evaluate`、`_run_code_unsafe`、`_file_upload`、`_drop`。
   最後四個預設不開放。代理從子行程的 `tools/list` 動態取得工具與 schema，只多加 `system`、`intent` 兩個參數，
   不寫死參數名稱。

尚未驗證（計畫的第一步要補）：Windows／Linux 上的 Chrome 啟動與偵錯埠；有頭模式下的行為（探勘用 headless）。

## 測試

- 閘門判定是純函式：方法 × 模式 × 規則 × 網域的組合用單元測試窮舉。
- 端到端：本機假網站（有登入頁、會寫入的表單、計數器），確認**被拒絕或未核准的寫入，伺服器的計數器沒有增加**——這是最重要的一條。
  需要真的 Chrome，預設忽略、以環境變數啟用（跟現有被忽略的測試一樣）。
- 一鍵停用、停止、接手、session 過期各一條。
- 前端：核准卡、系統頁面的狀態顯示（Vitest＋Testing Library）。
