# 關閉視窗前的工作進行中確認 — 設計

日期：2026-09-21

## 問題

分頁的 ✕ 已有關閉確認（`registerCloseGuard`，見 `TerminalApp.tsx`），但**關閉整個視窗**完全沒有攔截：

- 自繪標題列 ✕：`TitleBar/index.tsx` 直接 `getCurrentWindow().close()`（`decorations: false`）。
- Windows/Linux 的 Alt+F4、macOS 的 Cmd+Q：沒有任何處理。
- `lib.rs` 的 `run` 回呼只在 `RunEvent::Exit` 做郵件登出，沒有 `ExitRequested` 攔截。

結果：有指令、Agent、工作看板任務在跑時，一鍵關視窗就全部被殺，沒有任何提示。

## 目標與非目標

**目標**：任一分頁有「進行中的工作」時，關視窗（✕／Alt+F4／Cmd+Q）先跳**一個**確認框，列出哪些分頁在忙什麼；使用者確認才真的關。全部閒置則直接關，不打擾。

**非目標**：
- 不處理背景工作（知識庫同步、郵件同步、更新下載）——使用者已選擇「沿用分頁判定」。
- 不處理「有內容但沒在跑」的情境（有對話紀錄的程式庫協助、Loop 的 roster 未存檔）。那是分頁 ✕ 的「內容遺失」guard 的職責，不是「工作進行中」。
- 不修 `runningTaskTabRegistry` 已記載的漏報（工作看板沒開時新派工尚未登記）。

## 「進行中」的定義（沿用分頁判定的忙碌子集）

| 分頁類型 | 進行中訊號 | 來源 |
|---|---|---|
| 終端機 | 最後一個 block `status === "running"`（`isBusyRef`）；`agentMission?.active`；`isRunningTaskTab(sessionIdRef.current)` | `TerminalView.tsx` |
| Loop | `loop.isRunning` | `LoopStudio/index.tsx` |
| 程式庫協助 | `isStreamingRef.current` | `CodeAssistantView/index.tsx` |

閒置終端機、無串流的程式庫協助、未在跑的 Loop 都回報閒置。

## 設計

### 1. 忙碌探針（只問不彈）

既有 guard 是 `() => Promise<boolean>` 且會在**該分頁內**彈框，無法只查詢。新增一條並行的註冊：

```ts
// TerminalApp.tsx
type BusyReason = "command" | "agent" | "task" | "loop" | "streaming";
registerBusyProbe(tabId: string, probe: () => BusyReason | null): void;
unregisterBusyProbe(tabId: string): void;
```

- `TerminalApp` 以 `busyProbesRef: Map<string, () => BusyReason | null>` 持有，與 `closeGuardsRef` 平行。
- 三個消費端各註冊一支，放在既有 close guard effect 旁的**獨立 effect**（不動既有 guard 的 effect，降低回歸風險），deps 只放 `[tabId, registerBusyProbe, unregisterBusyProbe]`（終端機多一個穩定的 `getBusyReason`）。
- **探針必須讀 ref，不可閉包捕捉狀態**（見 memory `project-close-guard-mechanism` 陷阱 1：閉包會靜默放行）。`LoopStudio` 目前的 guard effect deps 含 `loop.isRunning`，探針要改讀 ref 以免重註冊窗口內漏報。
- `agentMission` 可為 `null`，一律 `agentMission?.active ?? false`。
- **避免判定漂移**：各消費端把「是否忙碌」抽成一個函式，分頁 ✕ 的 guard 與探針共用它。終端機的判定有註解說明的三訊號（isBusy／mission／running task），只能有一份。

### 2. 視窗關閉確認框

新增 `useWindowCloseGuard`（`src/hooks/`，或放在 `TerminalApp` 旁，實作時依現有慣例）：

- 掛載時 `getCurrentWindow().onCloseRequested(async (event) => { ... })`。
- 遍歷 `busyProbesRef` 收集 `{ tabId, title, reason }`；空 → 不 `preventDefault`，直接放行。
- 非空 → `event.preventDefault()`，`setPendingQuit(list)` 顯示 `CloseConfirmDialog`（重用，`src/components/CloseConfirmDialog/`）。body 列出「分頁名稱 — 正在做什麼」。
- 確認 → `getCurrentWindow().destroy()`（不用 `close()`，避免再次觸發 `onCloseRequested` 造成迴圈）；取消 → 清掉 `pendingQuit`。單一視窗 App，`destroy()` 之後 runtime 會自行走退出流程（真機驗證：行程正常結束、exit code 0，不留無視窗的殭屍程序）。原生關閉一律 `preventDefault()` 再自己 `destroy()`，閒置與確認兩條路徑因此走同一段。
- **重入保護**：框已顯示時再收到關閉請求（連按 ✕）直接忽略，不覆蓋狀態。既有分頁 guard 有「連點覆蓋 `closeResolveRef`」的已知競態，這裡不能複製；用 `pendingQuit !== null` 判斷即可，不需要 promise resolver。
- 確認鈕語意是「關閉並中止」；不需要逐一停止 Loop（退出時行程本來就會被終止）。
- `unlisten` 必須在 cleanup 呼叫並吞掉 rejection（repo 內 Tauri `unlisten()` 已知會 reject）。
- 事件 listener 用 ref 讀最新的探針表，不重註冊。

capabilities：`src-tauri/capabilities/default.json` 加 `core:window:allow-destroy`。（`allow-close` 已存在；`onCloseRequested` 本身需要的權限於實作時以實測確認，缺權限會是靜默失敗。）

### 3. Cmd+Q（macOS）：自訂 Quit 選單項目

**原設計（在 `RunEvent::ExitRequested` 攔截並 `prevent_exit()`）不可行，已捨棄。**
真機實測（tauri 2.10、macOS，在有 `sleep 300` 執行中按 Cmd+Q，並在 `RunEvent` 回呼裡
記錄每一筆事件）：只看到 `Exit`，**`ExitRequested` 從未送出**，`sleep` 被無聲殺掉。
預設選單的 Quit 走原生 `terminate:`，直接進入 `Exit`，Rust 端無從攔截。

**實作**（`src-tauri/src/quit.rs`）：
- `setup` 裡（僅 macOS，`#[cfg(target_os = "macos")]`）沿用 `Menu::default()`——複製貼上等
  快速鍵都靠它——只把 App 選單最後一項預設 Quit 換成自訂 `MenuItem`（id `aiterm-quit`，
  快速鍵 `Cmd+Q`）。找不到預設 Quit（預設選單結構變了）時不換，只記 `warn`，退回原本行為。
- `Builder::on_menu_event`：收到該 id 就 emit `app://quit-requested`（無酬載），**不結束程式**。
- 前端 `useWindowCloseGuard` 收到事件後走與視窗 ✕ 完全相同的 `attempt()`。
- 因此**不需要**旗標、`set_quit_confirmed` command 或 `ExitRequested` 處理：Rust 端不再攔截
  任何退出，只負責把 Cmd+Q 變成事件。`RunEvent::Exit` 的郵件登出邏輯**不動**。

Windows/Linux 沒有這個選單項目，✕／Alt+F4 走 `onCloseRequested`（第 2 節）。

**已知缺口**：Dock 圖示右鍵「結束」與系統登出／關機同樣走原生 `terminate:`，不會經過這個選單項目，
不受保護（見已知限制）。

### 4. i18n

`src/lib/i18n.ts` 新增（zh-TW 與 en 各一份）：確認框標題、內文開頭、各 `BusyReason` 的說明、確認鈕（「關閉並中止」）、取消鈕。en 是 `{ ...zhTW, ...enRaw }` 合併，`tsc -b` 抓不到缺漏，完成後用 `grep -c "<prefix>_" src/lib/i18n.ts` 人工核對（應為 key 數 × 2）。

## 測試

前端（Vitest + RTL，依 `reference-frontend-test-mounting` mock 三個 Tauri 入口）：

1. 全閒置：`onCloseRequested` handler 不呼叫 `preventDefault`，不出現框。
2. 任一探針回報忙碌：`preventDefault` 被呼叫、框出現、列出分頁與原因。
3. 確認：呼叫 `destroy`；取消：不呼叫，且可再次觸發關閉。
4. 重入：框已顯示時再次觸發，狀態不被覆蓋。
5. 探針讀最新狀態：註冊後才變忙碌，探針仍回報忙碌（釘住 ref 而非閉包，須以變異驗證：改讀閉包值時此測試要紅）。
6. 三個消費端各自的探針：終端機三訊號各一、Loop、串流；`agentMission === null` 不丟例外（用真的 hook 而非 mock 回物件——mock 會掩蓋這個 bug）。
7. 每個新測試要先證明會紅。

Rust：選單項目替換與事件 emit 是薄膠水程式，沒有可獨立單元測試的邏輯，以真機驗收為準（下節）。

## 手動驗收（真機）

方法：`npx tauri dev`，以 `--config '{"identifier":"com.aiterm.quittest"}'`、隔離的 `HOME`（並 `env -u APPLE_SIGNING_IDENTITY`，否則 dev runner 的 `codesign` 找不到鑰匙圈會在啟動前失敗）啟動，不碰使用者正在跑的那份；用 osascript 送 Cmd+Q／點紅燈，逐步截圖與查行程。
注意：`tauri dev` 的 webview 與正式版不同 origin，但 `RunEvent`／選單／視窗行為與正式版同一套 Tauri 執行期，這裡驗的是關閉流程而非通知等 dev 專屬差異。

| 情境 | 結果 |
|---|---|
| 終端機跑 `sleep 300`，Cmd+Q | ✅ 出現確認框（覆蓋整個視窗含標題列），列出「Terminal — 指令執行中」；App 與 `sleep` 皆存活 |
| 按「取消（繼續執行）」 | ✅ 框消失，`sleep` 仍在跑 |
| 取消後點視窗 ✕（紅燈） | ✅ 再次出現同一個確認框 |
| 按「關閉並中止」 | ✅ App 結束（`tauri dev` exit 0）、`sleep` 被終止、無殘留行程 |
| 閒置，Cmd+Q | ✅ 直接結束，exit 0 |
| 閒置，點視窗 ✕ | ✅ 直接結束，exit 0 |
| 修正前（基準線，同樣情境 Cmd+Q） | ❌ 無提示，`sleep` 被無聲殺掉（即使用者回報的問題） |

**未驗證**（明確標示，不宣稱通過）：
- Windows／Linux：✕、Alt+F4（無實機）。
- 工作看板任務執行中的情境、郵件登出（`RunEvent::Exit`）在確認流程後是否仍執行——Exit 事件在基準線實測中確實會送出，但未實際帶郵件帳號驗證登出。
- Dock 右鍵「結束」與系統登出／關機。

## 已知限制

- 工作看板沒開著時新派工的任務尚未登記於 `runningTaskTabRegistry`，關視窗可能漏警告（沿用該模組已記載的限制）。
- 對話框沒有 Escape 關閉與 focus trap（沿用 `CloseConfirmDialog` 現況）。
- 保護依賴前端存活：Cmd+Q 與 ✕ 都是「Rust／視窗事件 → 前端決定」。前端整個掛掉（白畫面、無回應）時，✕ 與 Cmd+Q 會沒有反應，需強制結束。
- Dock 右鍵「結束」與系統登出／關機走原生 `terminate:`，不經自訂選單項目，不受保護；要涵蓋需攔 NSApplicationDelegate 的 `applicationShouldTerminate`，超出本次範圍。
