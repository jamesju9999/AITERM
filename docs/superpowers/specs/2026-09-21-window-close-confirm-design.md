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
- 三個消費端各註冊一支，**沿用同一個 effect**（與 `registerCloseGuard` 同處註冊／清理），deps 一樣只放 `[tabId, registerBusyProbe, unregisterBusyProbe]`。
- **探針必須讀 ref，不可閉包捕捉狀態**（見 memory `project-close-guard-mechanism` 陷阱 1：閉包會靜默放行）。`LoopStudio` 目前的 guard effect deps 含 `loop.isRunning`，探針要改讀 ref 以免重註冊窗口內漏報。
- `agentMission` 可為 `null`，一律 `agentMission?.active ?? false`。
- **避免判定漂移**：各消費端把「是否忙碌」抽成一個函式，分頁 ✕ 的 guard 與探針共用它。終端機的判定有註解說明的三訊號（isBusy／mission／running task），只能有一份。

### 2. 視窗關閉確認框

新增 `useWindowCloseGuard`（`src/hooks/`，或放在 `TerminalApp` 旁，實作時依現有慣例）：

- 掛載時 `getCurrentWindow().onCloseRequested(async (event) => { ... })`。
- 遍歷 `busyProbesRef` 收集 `{ tabId, title, reason }`；空 → 不 `preventDefault`，直接放行。
- 非空 → `event.preventDefault()`，`setPendingQuit(list)` 顯示 `CloseConfirmDialog`（重用，`src/components/CloseConfirmDialog/`）。body 列出「分頁名稱 — 正在做什麼」。
- 確認 → 先 `invoke("set_quit_confirmed")`（第 3 節），再 `getCurrentWindow().destroy()`（不用 `close()`，避免再次觸發 `onCloseRequested` 造成迴圈）；取消 → 清掉 `pendingQuit`。單一視窗 App，`destroy()` 之後 runtime 會自行走退出流程。
- **重入保護**：框已顯示時再收到關閉請求（連按 ✕）直接忽略，不覆蓋狀態。既有分頁 guard 有「連點覆蓋 `closeResolveRef`」的已知競態，這裡不能複製；用 `pendingQuit !== null` 判斷即可，不需要 promise resolver。
- 確認鈕語意是「關閉並中止」；不需要逐一停止 Loop（退出時行程本來就會被終止）。
- `unlisten` 必須在 cleanup 呼叫並吞掉 rejection（repo 內 Tauri `unlisten()` 已知會 reject）。
- 事件 listener 用 ref 讀最新的探針表，不重註冊。

capabilities：`src-tauri/capabilities/default.json` 加 `core:window:allow-destroy`。（`allow-close` 已存在；`onCloseRequested` 本身需要的權限於實作時以實測確認，缺權限會是靜默失敗。）

### 3. Cmd+Q（macOS）

macOS 預設選單的 Quit 不經 `onCloseRequested`，而走 `RunEvent::ExitRequested`。專案沒有自訂選單。

- `lib.rs` 的 `run` 回呼加入 `RunEvent::ExitRequested { code, api, .. }` 處理：當 `code.is_none()`（使用者觸發、非程式呼叫 `app.exit`）**且**尚未確認過時，`api.prevent_exit()` 並 emit `app://quit-requested` 給前端，前端走同一個確認框流程（第 2 節）。
- 前端確認後呼叫新 command `set_quit_confirmed`：只把 `QUIT_CONFIRMED` 旗標設為 true，然後前端 `destroy()` 視窗，由最後一個視窗關閉觸發退出。✕ 與 Cmd+Q 兩條路徑因此共用同一段確認後流程。
- **注意「最後一個視窗關閉」也會觸發 `ExitRequested`（code 為 None）**：`destroy()` 之後 runtime 會再送一次，若被攔下 App 就關不掉。所以兩條路徑都必須先 `set_quit_confirmed`，Rust 端旗標為 true 時一律放行。
- 旗標用 `AtomicBool` 放在 Tauri managed state。
- 閒置時（前端回報無忙碌分頁）：前端也要呼叫 `set_quit_confirmed` 再 `destroy()`，不能只靠「不攔」，否則 Rust 端已經 `prevent_exit()` 了。也就是說 Rust 端對 Cmd+Q 一律先攔、由前端決定，前端閒置就立即 `set_quit_confirmed` 並 `destroy()`。這讓「判斷忙不忙」只存在前端一處。
- `RunEvent::Exit` 的郵件登出邏輯**不動**，且仍必須在最終真的退出時執行（註解已說明為何用 `Exit` 而非 `ExitRequested`）。

**此節的行為假設必須在實作第一步實測**（沿用「沒有失敗訊號不等於正確」的教訓）：
1. Cmd+Q 是否觸發 `ExitRequested` 且 `code == None`。
2. 視窗最後關閉後是否再送一次 `ExitRequested`。
3. `set_quit_confirmed` → `destroy()` 之後 `RunEvent::Exit` 是否仍執行（郵件登出）。
4. Windows/Linux 的 Alt+F4 是否只走 `onCloseRequested`。

若任一假設不成立，回頭修正本節再繼續，不硬套。

### 4. i18n

`src/lib/i18n.ts` 新增（zh-TW 與 en 各一份）：確認框標題、內文開頭、各 `BusyReason` 的說明、確認鈕（「關閉並中止」）、取消鈕。en 是 `{ ...zhTW, ...enRaw }` 合併，`tsc -b` 抓不到缺漏，完成後用 `grep -c "<prefix>_" src/lib/i18n.ts` 人工核對（應為 key 數 × 2）。

## 測試

前端（Vitest + RTL，依 `reference-frontend-test-mounting` mock 三個 Tauri 入口）：

1. 全閒置：`onCloseRequested` handler 不呼叫 `preventDefault`，不出現框。
2. 任一探針回報忙碌：`preventDefault` 被呼叫、框出現、列出分頁與原因。
3. 確認：依序呼叫 `set_quit_confirmed` 與 `destroy`；取消：不呼叫，且可再次觸發關閉。
4. 重入：框已顯示時再次觸發，狀態不被覆蓋。
5. 探針讀最新狀態：註冊後才變忙碌，探針仍回報忙碌（釘住 ref 而非閉包，須以變異驗證：改讀閉包值時此測試要紅）。
6. 三個消費端各自的探針：終端機三訊號各一、Loop、串流；`agentMission === null` 不丟例外（用真的 hook 而非 mock 回物件——mock 會掩蓋這個 bug）。
7. 每個新測試要先證明會紅。

Rust：`ExitRequested` 的旗標邏輯抽成純函式（輸入：`code`、`confirmed`；輸出：攔或放行）做單元測試。

## 手動驗收（真機）

- macOS：正式 build（dev 模式的行為不同），分別測 ✕、Cmd+Q，各在「閒置」「終端機跑 `sleep 60`」「工作看板任務執行中」三種狀態。
- 關閉後確認郵件登出仍有執行。
- Windows/Linux：✕、Alt+F4（若無實機，於 spec 結案註明未驗證，不宣稱通過）。
- 跨平台要求：本設計只用 Tauri 視窗 API 與 `RunEvent`，無平台專屬路徑；Cmd+Q 節僅 macOS 有意義，其他平台該事件路徑不應造成行為差異，需驗證。

## 已知限制

- 工作看板沒開著時新派工的任務尚未登記於 `runningTaskTabRegistry`，關視窗可能漏警告（沿用該模組已記載的限制）。
- 對話框沒有 Escape 關閉與 focus trap（沿用 `CloseConfirmDialog` 現況）。
