# 派工任務完成後自動清理終端機分頁 — 設計

日期：2026-09-08
狀態：待使用者複審

## 問題

工作看板派工的卡片跑完之後，它開的終端機分頁跟底層 PTY/claude 行程完全不會自動收掉——只有使用者自己手動關分頁（Ctrl+W 或分頁上的 ✕）才會清掉。長時間掛著跑，分頁跟行程會一路累積；外觀上也容易被誤認成「同時開了兩個 Agent」。

## 現況調查（讀過程式碼確認，非推測）

- `src-tauri/src/tasks/monitor.rs:94-147` 的 `watch()` 只回傳 `TaskOutcome`（`Success`/`Failed(String)`/`Cancelled`），從不碰 `pty`。互動模式（`WatchMode::Interactive`）一樣會走到終局——靠使用者按「標記完成」或 claude 行程真的結束（硬訊號 ③④，兩種模式都信任），不是只有自動模式才會完成。
- `src-tauri/src/tasks/scheduler.rs:88-186`：`RealDispatcher::dispatch` 派工後，`await monitor::watch()` 拿到結果，依序寫 transcript、`persist_outcome`（呼叫 `store::finish_task` 更新 DB）、移除 cancel handle、emit `tasks-updated` 與 `task-finished`。全程沒有呼叫 `pty.close(&tab_id)` 或任何 kill。
- 整個 `tasks/` 模組唯一呼叫 `pty.close()` 的地方是 `dispatch.rs:368`，且只在「剛建立的 PTY 第一次寫入就失敗」（spawn 失敗）時觸發，跟任務正常完成無關。
- **既有的「跨元件關分頁」機制已經存在，且已驗證可用**：
  - `src/components/TaskBoard/TaskCard.tsx:46-48`——刪除卡片並勾選「連分頁一起關閉」時：`window.dispatchEvent(new CustomEvent("aiterm:close-tab", { detail: { tabId: card.tab_id } }))`。
  - `src/components/TerminalApp.tsx` 的 `handleCloseTab`（唯一會移除分頁陣列項目的地方）已經監聽 `aiterm:close-tab` 視窗事件（`useEffect` 裡的 `onCloseTab`），呼叫 `handleCloseTab(tab.id)`。
  - `TerminalView.tsx:1273/1782` 在分頁卸載的 cleanup 路徑呼叫 `closePty(id)`——這是真正砍掉背後 PTY/claude 行程的地方。分頁從陣列移除 → `TerminalView` 卸載 → `closePty()` 自動觸發，全程不需要新的後端關閉邏輯。
- `src/components/TaskBoard/useTranscriptUpgrader.ts` 是目前唯一監聽 `task-finished`（後端 Tauri 事件，酬載 `{project_id, task_id, tab_id}`，**不含結局**）的地方，掛在永遠存在的 `TerminalApp.tsx` 上（不是掛在會被卸載的 `TaskBoardView`），這個掛載位置選擇本身就有明確的既有理由（見該檔案 doc comment）：看板只有在該專案是當前分頁時才掛載，別的專案完成時沒人在聽。
- 相關既有教訓（`feedback_mount_scope_for_completion_hooks` 記憶）：完成時要做的事不能放在條件掛載的元件裡，要用「帶酬載的事件＋永遠掛載的監聽器」，這次設計直接沿用同一個已驗證正確的模式。

## 範圍（brainstorming 已確認的三個決定）

1. **做成可關的設定**：`task_board` 設定新增一個開關，預設開。
2. **只有成功／取消才自動關；失敗保留分頁**，方便使用者直接在分頁裡看錯誤輸出，不用另外翻封存的對話記錄。
3. **使用者目前正在看的分頁（active tab）例外，不自動關**——避免正盯著看的畫面突然消失；只有背景完成的分頁會被自動收掉。這個排除只在完成當下判斷一次，之後使用者切走也不會回頭補關。

## 架構

**不新增任何後端關閉分頁/砍行程的邏輯**——`aiterm:close-tab` → `handleCloseTab` → `TerminalView` 卸載 → `closePty()` 這條路徑已經存在且已被 `TaskCard` 刪除流程驗證過。這次只需要在正確的時機、正確的條件下觸發同一個事件。

### 後端改動

**`TaskFinishedEvent`**（`scheduler.rs:57-65`）新增一個欄位：

```rust
struct TaskFinishedEvent {
    project_id: String,
    task_id: String,
    tab_id: String,
    outcome: String,  // 新增："success" | "failed" | "cancelled"
}
```

`scheduler.rs:177-183` emit 的地方，`outcome` 欄位填 `outcome.as_str().to_string()`——`outcome: monitor::TaskOutcome` 這個變數在該處已經在 scope 裡，是 `monitor::watch()` 剛回傳的值，不需要另外查。

**`TaskBoardConfig`**（`config/types.rs:194-209`）新增：

```rust
#[serde(default = "default_true")]
pub auto_close_finished_tabs: bool,
```

（`config/types.rs:322` 已經有一個 `fn default_true() -> bool { true }`，直接沿用，不要重寫一個同名函式。）

### 前端改動

**`TaskBoardConfig` 型別**（`src/ipc/tasks.ts`）比照後端加 `auto_close_finished_tabs: boolean`。

**新 hook** `src/components/TaskBoard/useAutoCloseFinishedTabs.ts`（獨立檔案，跟 `useTranscriptUpgrader.ts`同一種「單一職責、掛在永遠存在的元件上」的模式）：

```ts
export function useAutoCloseFinishedTabs(activeIdRef: RefObject<string>): void {
  useEffect(() => {
    const un = listen<TaskFinishedPayload>("task-finished", (e) => {
      const { tab_id, outcome } = e.payload;
      if (outcome === "failed") return;
      if (tab_id === activeIdRef.current) return;
      void getTaskBoardConfig().then((cfg) => {
        if (!cfg.auto_close_finished_tabs) return;
        window.dispatchEvent(new CustomEvent("aiterm:close-tab", { detail: { tabId: tab_id } }));
      });
    });
    return unlistenOnCleanup(un, "task-finished");
  }, [activeIdRef]);
}
```

`activeIdRef` 由 `TerminalApp.tsx` 傳入（它本來就持有這個 ref），在事件觸發當下讀 `.current`——避免閉包捕捉到掛載當時的舊值（呼應前面提到的既有教訓）。判斷順序刻意是「先看 outcome/active，通過才去查設定」：多數事件會在第一關就被擋掉（尤其 active tab 那條，是最常見情況），不用每次都白跑一次 IPC。

**`TerminalApp.tsx`** 在既有的 `useTranscriptUpgrader();` 旁邊加一行：

```ts
useAutoCloseFinishedTabs(activeIdRef);
```

**Settings 頁面** `src/components/Settings/TaskBoardPage.tsx` 在 `claude_command` 欄位後面加一個 checkbox，比照 `parallel_ok`/`interactive` 那類 checkbox 欄位的既有寫法：

```tsx
<label className="task-board-field task-board-field--checkbox">
  <input
    type="checkbox"
    checked={cfg.auto_close_finished_tabs}
    onChange={(e) => {
      setSaved(false);
      setCfg({ ...cfg, auto_close_finished_tabs: e.target.checked });
    }}
  />
  <span>{t.board_settings_auto_close}</span>
  <span className="task-board-hint">{t.board_settings_auto_close_hint}</span>
</label>
```

## i18n

新增（en / zh-TW 各一份）：`board_settings_auto_close`（「任務完成後自動關閉分頁」）、`board_settings_auto_close_hint`（「只有成功或取消的任務會自動關閉；失敗的任務會保留分頁方便除錯。你正在看的分頁不會被自動關閉。」）。

## 明確不做的部分

- 不做「延遲幾秒再關」的緩衝——只排除目前正在看的分頁，背景完成的分頁立即關閉，不額外加時間緩衝。
- 不做「使用者切走之後回頭補關已排除的分頁」——排除只在完成當下判斷一次。
- 不改變失敗任務的既有行為——分頁保留，跟現在一樣要手動關。
- 不對互動模式的卡片做特殊處理——它們一樣照 outcome/active 規則走，沒有另外的例外規則（互動模式卡片多半是使用者自己按「標記完成」才會走到這裡，本質上已經是一次主動的「我做完了」訊號）。

## 測試

- **Rust**：`TaskFinishedEvent` 多一個欄位是純資料結構改動，不需要新增行為測試；若既有測試斷言過這個事件的序列化形狀（例如某個 snapshot 或欄位比對），要一併更新。`default_true`／`TaskBoardConfig` 的預設值可以照現有 `TaskBoardConfig` 的既有測試模式補一個「新資料庫沒有這個設定時預設是 true」的斷言（如果既有測試檔案有這種模式）。
- **前端**：新增 `useAutoCloseFinishedTabs.test.ts`（或 `.tsx`，視是否要用 `renderHook`），涵蓋：
  - `outcome: "success"` 且非 active 分頁、設定開啟 → dispatch `aiterm:close-tab`，`detail.tabId` 等於事件的 `tab_id`
  - `outcome: "cancelled"` → 同上會關
  - `outcome: "failed"` → 不 dispatch
  - `tab_id === activeIdRef.current` → 不 dispatch（就算 outcome 是 success）
  - `getTaskBoardConfig()` 回傳 `auto_close_finished_tabs: false` → 不 dispatch
- `TaskBoardPage.test.tsx` 補新 checkbox 的存讀測試，比照既有欄位的測試模式。
