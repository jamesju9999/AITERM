# 派工任務完成推播（桌面通知 + Telegram）— 設計

日期：2026-09-09
狀態：待使用者複審

## 問題

工作看板的卡片跑完（成功／失敗／取消）之後，使用者只有切回那個分頁或看板才會知道。長時間背景派工（尤其開了好幾個專案分頁在跑）容易錯過完成或失敗的時機，尤其人離開電腦時完全沒有訊號。

## 現況調查（讀過程式碼確認，非推測）

- `src-tauri/src/tasks/scheduler.rs:135-187`：`RealDispatcher::dispatch` 在卡片跑完後 emit 兩個事件：`tasks-updated`（無酬載）與 `task-finished`（酬載 `{project_id, task_id, tab_id, outcome}`，`outcome` 是 `TaskOutcome::as_str()`，`"success" | "failed" | "cancelled"`）。這個事件是全域 emit（`app.emit`），不需要看板掛載就能收到。
- `TaskOutcome`（`src-tauri/src/tasks/monitor.rs:19-38`）已經有 `error_message(&self) -> Option<&str>`，`Failed(String)` 才回 `Some`，其他兩種回 `None`——目前這個訊息**沒有**被送進 `task-finished` 事件。
- `ProjectHandle`（`src-tauri/src/projects/mod.rs:136-140`）已經有 `name: String` 欄位（專案的人類可讀名稱，跟 `id`/`path` 分開）；`scheduler.rs` 的 `dispatch` 裡 `project: &ProjectHandle` 在 spawn 前可以直接取用。
- `TaskRow`（`src-tauri/src/tasks/store.rs:15-29`）有 `title: String`。`dispatch` 裡的 `task: &TaskRow` 同樣在 spawn 前可以取用；`task_id`/`work_dir` 目前就是這樣先 clone 出來再 move 進 async block 的（`scheduler.rs:149-153`）。
- **既有的「跨元件完成事件」監聽模式已經存在，且已驗證可用**：`useAutoCloseFinishedTabs.ts`（`src/components/TaskBoard/useAutoCloseFinishedTabs.ts`）掛在永遠存在的 `TerminalApp.tsx` 上監聽 `task-finished`，理由是看板只有在該專案是當前分頁時才掛載，別的專案完成時沒人在聽（`feedback_mount_scope_for_completion_hooks` 這條教訓）。這次直接沿用同一個掛載位置與監聽模式。
- **桌面通知管線已存在**：`src/lib/notifyPermission.ts` 的 `ensureNotificationPermission()`（整個 app 生命週期只實際問一次權限，desktop 上永遠 resolve `true`，因為外掛桌面實作不會真的問 OS）+ `@tauri-apps/plugin-notification` 的 `sendNotification()`。`src/hooks/useMailSync.ts:71-90` 是現有唯一的使用範例。
- **Telegram 主動推播管線已存在**：`src/ipc/telegram.ts` 的 `sendTelegramMessage(text: string)` → `invoke("telegram_send_message", { text })` → `src-tauri/src/telegram/mod.rs:182-200`，內部直接用設定裡的 `bot_token`/`chat_id` 打 Telegram Bot API，不依賴「正在回覆某則收到的訊息」這個前提，本來就能主動推播。`getTelegramConfig()` 回傳 `{ bot_token, chat_id }`（皆為 `string | null`），可以拿來判斷「有沒有設定過」。

## 範圍（brainstorming 已確認的決定）

1. **兩個管道都做**：桌面原生通知 + Telegram 推播，各自獨立開關。
2. **三種結局都通知**（成功／失敗／取消），不做「只通知失敗」的篩選。
3. **桌面通知**：完成的分頁若正是使用者目前正在看的分頁，不彈通知（跟 `useAutoCloseFinishedTabs` 的 active-tab 排除規則相同語意）。
4. **Telegram**：不跡「目前在看哪個分頁」這條規則，一律發送——Telegram 存在的意義就是人不在電腦前，跟前景分頁無關。
5. **失敗要帶原因**：通知內容包含 `error_message`（例如「claude 以 exit code 1 結束」「疑似卡住（120 秒無輸出）」），讓使用者不開 App 就知道發生什麼事。
6. **設定分兩個獨立開關**，都在「設定 → 工作看板」頁：
   - 「完成時發桌面通知」，預設開。
   - 「完成時發 Telegram 通知」，預設開，但只在 Telegram 已設定（`bot_token` 與 `chat_id` 皆非空）時才顯示這個 checkbox；未設定時整列隱藏，避免使用者誤以為勾了會生效。

## 架構

**不新增任何新的通知傳輸邏輯**——桌面通知沿用 `sendNotification`/`ensureNotificationPermission`（`useMailSync.ts` 已驗證的路徑），Telegram 沿用 `sendTelegramMessage`（既有 command，本來就能主動推播）。這次只是把 `task-finished` 事件的酬載補齊，並在正確的時機、正確的條件下呼叫這兩條已存在的管線。

### 後端改動

**`TaskFinishedEvent`**（`scheduler.rs:61-66`）新增三個欄位：

```rust
struct TaskFinishedEvent {
    project_id: String,
    task_id: String,
    tab_id: String,
    outcome: String,        // 既有
    title: String,          // 新增：卡片標題
    project_name: String,   // 新增：專案名稱
    error_message: Option<String>, // 新增：失敗原因，只有 Failed 才有值
}
```

在 `dispatch` 裡，跟 `task_id`/`work_dir` 一樣，spawn 前多 clone 兩個值：

```rust
let task_title = task.title.clone();
let project_name = project.name.clone();
```

emit 那段（`scheduler.rs:177-185`）補上：

```rust
let _ = app.emit(
    "task-finished",
    TaskFinishedEvent {
        project_id: project_id.clone(),
        task_id: task_id.clone(),
        tab_id: tab_id.clone(),
        outcome: outcome.as_str().to_string(),
        title: task_title.clone(),
        project_name: project_name.clone(),
        error_message: outcome.error_message().map(str::to_string),
    },
);
```

**`TaskBoardConfig`**（`src-tauri/src/config/types.rs:194-212`）新增兩個欄位，比照 `auto_close_finished_tabs` 用 `default_true`：

```rust
#[serde(default = "default_true")]
pub notify_desktop_on_finish: bool,
#[serde(default = "default_true")]
pub notify_telegram_on_finish: bool,
```

`Default for TaskBoardConfig` 與 `apply_editable_task_board_fields`（`src-tauri/src/commands/task_board_config.rs:24-38`）都要跟著補上這兩個欄位（後者直接取 `incoming` 的值，不像 `project_paths` 需要保留 `current`）。

### 前端改動

**`TaskFinishedPayload` / `TaskBoardConfig` 型別**（`src/ipc/tasks.ts`、`useAutoCloseFinishedTabs.ts` 裡各自定義的 `TaskFinishedPayload`）比照後端補齊新欄位。

**新 hook** `src/components/TaskBoard/useTaskCompletionNotifications.ts`，跟 `useAutoCloseFinishedTabs.ts` 同一種「單一職責、掛在永遠存在的元件上」模式：

```ts
export function useTaskCompletionNotifications(activeIdRef: RefObject<string>): void {
  useEffect(() => {
    const un = listen<TaskFinishedPayload>("task-finished", (e) => {
      const { tab_id, outcome, title, project_name, error_message } = e.payload;
      const body =
        outcome === "success" ? `${project_name} · 完成` :
        outcome === "cancelled" ? `${project_name} · 已取消` :
        `${project_name} · 失敗：${error_message ?? ""}`;

      void getTaskBoardConfig().then((cfg) => {
        if (cfg.notify_desktop_on_finish && tab_id !== activeIdRef.current) {
          void ensureNotificationPermission().then((granted) => {
            if (granted) sendNotification({ title, body });
          });
        }
        if (cfg.notify_telegram_on_finish) {
          const icon = outcome === "success" ? "✅" : outcome === "cancelled" ? "⏹️" : "❌";
          const line2 = outcome === "failed" ? `失敗：${error_message ?? ""}` : body.split(" · ")[1];
          void sendTelegramMessage(`${icon} ${project_name} — ${title}\n${line2}`).catch(() => {});
        }
      });
    });
    return unlistenOnCleanup(un, "task-finished");
  }, [activeIdRef]);
}
```

（上面是設計骨架，實作時把桌面/Telegram 兩段訊息組字各自抽成小函式，避免在事件 callback 裡塞太多字串邏輯。）

`sendNotification` 呼叫失敗（例如桌面通知權限被拒）沿用 `useMailSync.ts` 的既有態度：不用 try/catch 包，`ensureNotificationPermission()` 本身永不 reject，`granted` 是 false 就單純不發。`sendTelegramMessage` 是 async command，未設定 bot 時後端會回 `Err`，前端 `.catch(() => {})` 吞掉——不應該讓一次推播失敗影響任何其他行為。

**`TerminalApp.tsx`** 在既有的 `useAutoCloseFinishedTabs(activeIdRef);` 旁邊加一行：

```ts
useTaskCompletionNotifications(activeIdRef);
```

**Settings 頁面** `src/components/Settings/TaskBoardPage.tsx` 在 `auto_close_finished_tabs` 的 checkbox 後面加兩個：

```tsx
<label className="task-board-field task-board-field--checkbox">
  <input
    type="checkbox"
    className="task-board-checkbox"
    checked={cfg.notify_desktop_on_finish}
    onChange={(e) => {
      setSaved(false);
      setCfg({ ...cfg, notify_desktop_on_finish: e.target.checked });
    }}
  />
  <span>{t.board_settings_notify_desktop}</span>
  <span className="task-board-hint">{t.board_settings_notify_desktop_hint}</span>
</label>

{telegramConfigured && (
  <label className="task-board-field task-board-field--checkbox">
    <input
      type="checkbox"
      className="task-board-checkbox"
      checked={cfg.notify_telegram_on_finish}
      onChange={(e) => {
        setSaved(false);
        setCfg({ ...cfg, notify_telegram_on_finish: e.target.checked });
      }}
    />
    <span>{t.board_settings_notify_telegram}</span>
    <span className="task-board-hint">{t.board_settings_notify_telegram_hint}</span>
  </label>
)}
```

`telegramConfigured` 是 `TaskBoardPage` 掛載時額外呼叫 `getTelegramConfig()` 算出的 `boolean`（`Boolean(cfg.bot_token) && Boolean(cfg.chat_id)`），跟 `cfg`（task board 設定）分開的一個 state。

## i18n

新增（en / zh-TW 各一份）：
- `board_settings_notify_desktop`（「完成時發桌面通知」）
- `board_settings_notify_desktop_hint`（「卡片跑完（成功／失敗／取消）時彈出系統通知。你正在看的分頁不會重複彈。」）
- `board_settings_notify_telegram`（「完成時發 Telegram 通知」）
- `board_settings_notify_telegram_hint`（「失敗時會附上失敗原因。需要先在設定 → Telegram 設定 bot 才會出現這個選項。」）

## 明確不做的部分

- 不做「點桌面通知跳回對應分頁」——`tauri-plugin-notification` 的點擊回呼跨平台行為不一致，且沒人要求，先不做。
- 不做「成功／失敗分開設定要不要通知」——brainstorming 已確認三種結局一視同仁。
- 不做通知節流／去重——`task-finished` 本來就是每張卡片跑完剛好觸發一次，天然沒有重複問題。
- 不改 Telegram 設定頁本身——沿用現有 bot token / chat id 設定流程，不新增任何 Telegram 專屬設定 UI。
- 互動模式卡片不做特殊處理，規則跟其他卡片一致（呼應 `2026-09-08-auto-close-finished-task-tabs-design.md` 的同一個決定）。

## 測試

- **Rust**：`TaskFinishedEvent` 多三個欄位是純資料結構改動；`apply_editable_task_board_fields` 補一組「`notify_desktop_on_finish`/`notify_telegram_on_finish` 從 `incoming` 取值」的斷言，比照現有 `auto_close_finished_tabs_is_taken_from_incoming` 測試（`task_board_config.rs:131-134`）。`TaskBoardConfig::default()` 補「新資料庫沒有這兩個欄位時預設是 true」的斷言，比照現有同類測試。
- **前端**：新增 `useTaskCompletionNotifications.test.ts`，比照 `useAutoCloseFinishedTabs.test.ts` 的骨架，涵蓋：
  - 三種 `outcome`（success/failed/cancelled）分別產生正確的桌面通知文字與 Telegram 文字
  - `outcome: "failed"` 時兩個管道的文字都包含 `error_message`
  - `tab_id === activeIdRef.current` → 不呼叫 `sendNotification`，但仍呼叫 `sendTelegramMessage`
  - `notify_desktop_on_finish: false` → 不呼叫 `sendNotification`
  - `notify_telegram_on_finish: false` → 不呼叫 `sendTelegramMessage`
  - `ensureNotificationPermission()` 回 `false` → 不呼叫 `sendNotification`
  - `sendTelegramMessage` reject → 不拋出、不影響桌面通知那條路徑
- `TaskBoardPage.test.tsx` 補兩個新 checkbox 的存讀測試，並補「`getTelegramConfig()` 回傳未設定時，Telegram checkbox 不顯示」的測試。
