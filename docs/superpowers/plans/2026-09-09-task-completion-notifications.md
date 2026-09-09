# 派工任務完成推播（桌面通知 + Telegram）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 工作看板卡片跑完（成功／失敗／取消）時，依設定分別彈出桌面通知與/或發送 Telegram 訊息，失敗時附上失敗原因。

**Architecture:** 後端把既有的 `task-finished` 事件酬載補上 `title`/`project_name`/`error_message` 三個欄位；前端新增一個永遠掛載在 `TerminalApp` 上的 hook，監聽這個事件，依 `TaskBoardConfig` 的兩個新開關分別呼叫既有的桌面通知管線（`sendNotification`/`ensureNotificationPermission`，`useMailSync.ts` 已驗證過）與既有的 Telegram 推播管線（`sendTelegramMessage`）。不新增任何新的通知傳輸邏輯，只接線既有管線。

**Tech Stack:** Rust（Tauri command / `tauri::Emitter`）、TypeScript/React、Vitest + React Testing Library、`@tauri-apps/plugin-notification`。

**Spec:** `docs/superpowers/specs/2026-09-09-task-completion-notifications-design.md`

---

## Task 1: 後端 — `TaskBoardConfig` 新增兩個通知開關

**Files:**
- Modify: `src-tauri/src/config/types.rs:190-224`（`TaskBoardConfig` struct + `Default` impl + 測試模組）

- [ ] **Step 1: 寫失敗的測試**

在 `src-tauri/src/config/types.rs` 的 `#[cfg(test)] mod tests`（約 872 行附近，`task_board_config_has_sane_defaults` 旁邊）加：

```rust
#[test]
fn task_board_config_notify_flags_default_to_true() {
    let c = TaskBoardConfig::default();
    assert!(c.notify_desktop_on_finish);
    assert!(c.notify_telegram_on_finish);
}

#[test]
fn a_config_written_before_notify_flags_existed_still_parses() {
    // 舊設定檔沒有這兩個欄位——必須不報錯，補成 true（維持原本「有通知」的行為）。
    let json = r#"{"max_concurrent":3,"claude_command":"claude"}"#;
    let c: TaskBoardConfig = serde_json::from_str(json).unwrap();
    assert!(c.notify_desktop_on_finish);
    assert!(c.notify_telegram_on_finish);
}
```

- [ ] **Step 2: 執行測試確認會紅**

Run: `cd src-tauri && cargo test task_board_config_notify_flags_default_to_true`
Expected: 編譯失敗（`notify_desktop_on_finish` 欄位不存在）

- [ ] **Step 3: 加欄位**

在 `TaskBoardConfig` struct（`src-tauri/src/config/types.rs:194-212`）的 `auto_close_finished_tabs` 欄位後面加：

```rust
    /// 派工卡片跑完（成功／失敗／取消）時，是否彈出桌面系統通知。若完成
    /// 的分頁正是使用者目前正在看的分頁則不彈——由前端 hook 判斷，這裡
    /// 只是總開關。
    #[serde(default = "default_true")]
    pub notify_desktop_on_finish: bool,
    /// 派工卡片跑完時，是否發送 Telegram 訊息。跟上面那個開關互相獨立，
    /// 且不看「目前在看哪個分頁」——人不在電腦前才是 Telegram 推播存在
    /// 的意義。
    #[serde(default = "default_true")]
    pub notify_telegram_on_finish: bool,
```

在 `impl Default for TaskBoardConfig`（`src-tauri/src/config/types.rs:216-224`）補：

```rust
impl Default for TaskBoardConfig {
    fn default() -> Self {
        Self {
            max_concurrent: default_task_board_max_concurrent(),
            claude_command: default_claude_command(),
            project_paths: Vec::new(),
            auto_close_finished_tabs: true,
            notify_desktop_on_finish: true,
            notify_telegram_on_finish: true,
        }
    }
}
```

- [ ] **Step 4: 執行測試確認會過**

Run: `cd src-tauri && cargo test task_board_config`
Expected: PASS（含新的兩個測試與既有的 `task_board_config_*`/`task_board_*` 測試）

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/config/types.rs
git commit -m "feat(taskboard): add notify_desktop_on_finish / notify_telegram_on_finish config flags

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012Cbm9KPo2DAenqgoCk4hYH"
```

---

## Task 2: 後端 — 設定對話框存讀新開關

**Files:**
- Modify: `src-tauri/src/commands/task_board_config.rs`

**注意**：這個檔案目前有 5 處手寫的 `TaskBoardConfig { ... }` struct literal（`current()` 輔助函式 + 4 個測試的 `incoming`/`mk`），Rust struct literal 必須列出所有欄位（沒有 `..Default::default()`），所以 Task 1 加欄位後這個檔案現在編譯不過。這個 Task 把它們全部補上，並新增一個驗證新欄位確實從 `incoming` 取值的測試。

- [ ] **Step 1: 寫失敗的測試**

在 `mod tests`（`task_board_config.rs:54` 起）的 `auto_close_finished_tabs_is_taken_from_incoming` 測試後面加：

```rust
#[test]
fn notify_flags_are_taken_from_incoming() {
    let mut incoming = current();
    incoming.notify_desktop_on_finish = false;
    incoming.notify_telegram_on_finish = false;
    let merged = apply_editable_task_board_fields(&current(), incoming);
    assert!(!merged.notify_desktop_on_finish);
    assert!(!merged.notify_telegram_on_finish);
}
```

- [ ] **Step 2: 執行測試確認會紅**

Run: `cd src-tauri && cargo test --lib commands::task_board_config`
Expected: 編譯失敗——`current()` 等既有的 5 個 `TaskBoardConfig` struct literal 缺 `notify_desktop_on_finish`/`notify_telegram_on_finish` 兩個欄位

- [ ] **Step 3: 補齊 `apply_editable_task_board_fields` 與所有既有 struct literal**

`apply_editable_task_board_fields`（`task_board_config.rs:26-39`）回傳的 struct 補兩行：

```rust
pub(crate) fn apply_editable_task_board_fields(
    current: &TaskBoardConfig,
    incoming: TaskBoardConfig,
) -> TaskBoardConfig {
    TaskBoardConfig {
        max_concurrent: incoming.max_concurrent.clamp(1, 16),
        claude_command: {
            let c = incoming.claude_command.trim();
            if c.is_empty() { "claude".to_string() } else { c.to_string() }
        },
        project_paths: current.project_paths.clone(),
        auto_close_finished_tabs: incoming.auto_close_finished_tabs,
        notify_desktop_on_finish: incoming.notify_desktop_on_finish,
        notify_telegram_on_finish: incoming.notify_telegram_on_finish,
    }
}
```

`current()`（`task_board_config.rs:58-65`）：

```rust
fn current() -> TaskBoardConfig {
    TaskBoardConfig {
        max_concurrent: 5,
        claude_command: "claude".to_string(),
        project_paths: vec!["/projects/a".to_string(), "/projects/b".to_string()],
        auto_close_finished_tabs: true,
        notify_desktop_on_finish: true,
        notify_telegram_on_finish: true,
    }
}
```

`saving_settings_does_not_wipe_the_project_list` 的 `incoming`（原 `task_board_config.rs:72-77`）：

```rust
let incoming = TaskBoardConfig {
    max_concurrent: 3,
    claude_command: "claude".to_string(),
    project_paths: Vec::new(),
    auto_close_finished_tabs: true,
    notify_desktop_on_finish: true,
    notify_telegram_on_finish: true,
};
```

`project_paths_in_the_payload_are_ignored` 的 `incoming`（原 `task_board_config.rs:90-95`）：

```rust
let incoming = TaskBoardConfig {
    max_concurrent: 5,
    claude_command: "claude".to_string(),
    project_paths: vec!["/injected".to_string()],
    auto_close_finished_tabs: true,
    notify_desktop_on_finish: true,
    notify_telegram_on_finish: true,
};
```

`max_concurrent_is_clamped_to_1_16` 的 `mk` 閉包（原 `task_board_config.rs:105-110`）：

```rust
let mk = |n: u32| TaskBoardConfig {
    max_concurrent: n,
    claude_command: "claude".to_string(),
    project_paths: Vec::new(),
    auto_close_finished_tabs: true,
    notify_desktop_on_finish: true,
    notify_telegram_on_finish: true,
};
```

`a_blank_claude_command_falls_back_to_claude` 的 `incoming`（原 `task_board_config.rs:118-123`）：

```rust
let incoming = TaskBoardConfig {
    max_concurrent: 5,
    claude_command: "   ".to_string(),
    project_paths: Vec::new(),
    auto_close_finished_tabs: true,
    notify_desktop_on_finish: true,
    notify_telegram_on_finish: true,
};
```

- [ ] **Step 4: 執行測試確認會過**

Run: `cd src-tauri && cargo test --lib commands::task_board_config`
Expected: PASS（6 個測試，含新增的 `notify_flags_are_taken_from_incoming`）

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/task_board_config.rs
git commit -m "feat(taskboard): wire notify flags through the settings save path

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012Cbm9KPo2DAenqgoCk4hYH"
```

---

## Task 3: 後端 — `task-finished` 事件補上標題/專案名稱/失敗原因

**Files:**
- Modify: `src-tauri/src/tasks/scheduler.rs:57-187`

- [ ] **Step 1: 改 `TaskFinishedEvent` struct**（`scheduler.rs:61-66`）

```rust
#[derive(Clone, serde::Serialize)]
struct TaskFinishedEvent {
    project_id: String,
    task_id: String,
    tab_id: String,
    outcome: String,
    title: String,
    project_name: String,
    error_message: Option<String>,
}
```

- [ ] **Step 2: 在 spawn 前多 clone 兩個值**

在 `scheduler.rs:149-153` 附近（`let task_id = task.id.clone();` / `let work_dir = ...` 那一段）加兩行：

```rust
let task_id = task.id.clone();
let task_title = task.title.clone();
let project_name = project.name.clone();
let work_dir = std::path::PathBuf::from(&task.project_dir);
```

- [ ] **Step 3: emit 時填入新欄位**

`scheduler.rs:177-185` 的 emit 改成：

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

（`outcome: monitor::TaskOutcome` 在這個 async block 裡本來就在 scope 中，`error_message()` 是既有方法，見 `src-tauri/src/tasks/monitor.rs:35-38`。）

- [ ] **Step 4: 編譯確認過**

Run: `cd src-tauri && cargo build`
Expected: 編譯成功，無警告

這個改動是純資料結構擴充，`scheduler.rs` 目前沒有針對 `TaskFinishedEvent` 內容的既有測試（`app.emit` 需要完整的 `AppHandle`，這個檔案的測試都是走 `Dispatcher` trait 的假實作，見 `scheduler.rs:473` 起的 `mod tests`），所以不新增行為測試——跟 `2026-09-08-auto-close-finished-task-tabs-design.md` 對同一個事件加欄位時的判斷一致。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/tasks/scheduler.rs
git commit -m "feat(taskboard): include title, project name, and error message in task-finished

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012Cbm9KPo2DAenqgoCk4hYH"
```

---

## Task 4: 前端 — `TaskBoardConfig` 型別補欄位，修好因此變紅的既有測試

**Files:**
- Modify: `src/ipc/tasks.ts:56-60`
- Modify: `src/components/TerminalApp.taskBoard.test.tsx:152-153`
- Modify: `src/components/TaskBoard/useAutoCloseFinishedTabs.test.ts:29-33,102-106`
- Modify: `src/components/Settings/TaskBoardPage.test.tsx:16-20`

- [ ] **Step 1: 補 TS 型別**

`src/ipc/tasks.ts:56-60`：

```ts
export interface TaskBoardConfig {
  max_concurrent: number;
  claude_command: string;
  auto_close_finished_tabs: boolean;
  notify_desktop_on_finish: boolean;
  notify_telegram_on_finish: boolean;
}
```

- [ ] **Step 2: 執行型別檢查確認會紅**

Run: `npx tsc -b`
Expected: 錯誤——以下三個檔案裡 `mockResolvedValue({...})` / 物件字面量缺少 `notify_desktop_on_finish`/`notify_telegram_on_finish`，不符合 `TaskBoardConfig` 型別：
`src/components/TerminalApp.taskBoard.test.tsx`、`src/components/TaskBoard/useAutoCloseFinishedTabs.test.ts`、`src/components/Settings/TaskBoardPage.test.tsx`

- [ ] **Step 3: 補齊三個測試檔案的物件字面量**

`src/components/TerminalApp.taskBoard.test.tsx:150-154` 附近：

```ts
    claude_command: "claude",
    auto_close_finished_tabs: true,
    notify_desktop_on_finish: true,
    notify_telegram_on_finish: true,
```

`src/components/TaskBoard/useAutoCloseFinishedTabs.test.ts:29-34`（`beforeEach` 裡的預設 mock）：

```ts
  getTaskBoardConfig.mockResolvedValue({
    max_concurrent: 1,
    claude_command: "claude",
    auto_close_finished_tabs: true,
    notify_desktop_on_finish: true,
    notify_telegram_on_finish: true,
  });
```

同檔案 `102-107`（「設定關閉時不關」那個測試裡的 mock）：

```ts
    getTaskBoardConfig.mockResolvedValue({
      max_concurrent: 1,
      claude_command: "claude",
      auto_close_finished_tabs: false,
      notify_desktop_on_finish: true,
      notify_telegram_on_finish: true,
    });
```

`src/components/Settings/TaskBoardPage.test.tsx:16-20`（`beforeEach`）：

```ts
  vi.mocked(getTaskBoardConfig).mockResolvedValue({
    max_concurrent: 2,
    claude_command: "claude",
    auto_close_finished_tabs: true,
    notify_desktop_on_finish: true,
    notify_telegram_on_finish: true,
  });
```

- [ ] **Step 4: 執行型別檢查與既有測試確認會過**

Run: `npx tsc -b && npm run test -- useAutoCloseFinishedTabs TerminalApp.taskBoard TaskBoardPage`
Expected: 型別檢查通過；三個測試檔案全綠（跟改動前行為完全相同，這步只是修型別）

- [ ] **Step 5: Commit**

```bash
git add src/ipc/tasks.ts src/components/TerminalApp.taskBoard.test.tsx src/components/TaskBoard/useAutoCloseFinishedTabs.test.ts src/components/Settings/TaskBoardPage.test.tsx
git commit -m "chore(taskboard): add notify flags to TaskBoardConfig TS type and test fixtures

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012Cbm9KPo2DAenqgoCk4hYH"
```

---

## Task 5: 前端 — 新 hook `useTaskCompletionNotifications`

**Files:**
- Create: `src/components/TaskBoard/useTaskCompletionNotifications.ts`
- Create: `src/components/TaskBoard/useTaskCompletionNotifications.test.ts`

- [ ] **Step 1: 寫失敗的測試**

建立 `src/components/TaskBoard/useTaskCompletionNotifications.test.ts`：

```ts
import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RefObject } from "react";

const getTaskBoardConfig = vi.fn();
vi.mock("../../ipc/tasks", () => ({
  getTaskBoardConfig: (...a: unknown[]) => getTaskBoardConfig(...a),
}));

const sendTelegramMessage = vi.fn();
vi.mock("../../ipc/telegram", () => ({
  sendTelegramMessage: (...a: unknown[]) => sendTelegramMessage(...a),
}));

const sendNotification = vi.fn();
vi.mock("@tauri-apps/plugin-notification", () => ({
  sendNotification: (...a: unknown[]) => sendNotification(...a),
}));

const ensureNotificationPermission = vi.fn();
vi.mock("../../lib/notifyPermission", () => ({
  ensureNotificationPermission: (...a: unknown[]) => ensureNotificationPermission(...a),
}));

let taskFinishedHandler: ((e: { payload: unknown }) => void) | null = null;
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((event: string, handler: (e: { payload: unknown }) => void) => {
    if (event === "task-finished") taskFinishedHandler = handler;
    return Promise.resolve(() => {});
  }),
}));

import { useTaskCompletionNotifications } from "./useTaskCompletionNotifications";

function activeRef(id: string): RefObject<string> {
  return { current: id };
}

const PAYLOAD = {
  project_id: "p1",
  task_id: "t1",
  tab_id: "tab-9",
  outcome: "success" as const,
  title: "修 bug",
  project_name: "AITerm",
  error_message: null as string | null,
};

beforeEach(() => {
  vi.clearAllMocks();
  taskFinishedHandler = null;
  getTaskBoardConfig.mockResolvedValue({
    max_concurrent: 1,
    claude_command: "claude",
    auto_close_finished_tabs: true,
    notify_desktop_on_finish: true,
    notify_telegram_on_finish: true,
  });
  ensureNotificationPermission.mockResolvedValue(true);
  sendTelegramMessage.mockResolvedValue(undefined);
});

describe("useTaskCompletionNotifications", () => {
  it("成功、非目前分頁 → 桌面通知與 Telegram 都發", async () => {
    renderHook(() => useTaskCompletionNotifications(activeRef("other-tab")));
    expect(taskFinishedHandler).not.toBeNull();

    taskFinishedHandler!({ payload: PAYLOAD });

    await waitFor(() => expect(sendNotification).toHaveBeenCalledWith({
      title: "修 bug",
      body: "AITerm · 完成",
    }));
    expect(sendTelegramMessage).toHaveBeenCalledWith("✅ AITerm — 修 bug\n完成");
  });

  it("是目前正在看的分頁 → 不發桌面通知，但仍發 Telegram", async () => {
    renderHook(() => useTaskCompletionNotifications(activeRef("tab-9")));
    taskFinishedHandler!({ payload: PAYLOAD });

    await waitFor(() => expect(sendTelegramMessage).toHaveBeenCalled());
    expect(sendNotification).not.toHaveBeenCalled();
  });

  it("失敗時兩個管道都帶上失敗原因", async () => {
    renderHook(() => useTaskCompletionNotifications(activeRef("other-tab")));
    taskFinishedHandler!({
      payload: { ...PAYLOAD, outcome: "failed", error_message: "claude 以 exit code 1 結束" },
    });

    await waitFor(() => expect(sendNotification).toHaveBeenCalledWith({
      title: "修 bug",
      body: "AITerm · 失敗：claude 以 exit code 1 結束",
    }));
    expect(sendTelegramMessage).toHaveBeenCalledWith(
      "❌ AITerm — 修 bug\n失敗：claude 以 exit code 1 結束",
    );
  });

  it("取消的文字", async () => {
    renderHook(() => useTaskCompletionNotifications(activeRef("other-tab")));
    taskFinishedHandler!({ payload: { ...PAYLOAD, outcome: "cancelled" } });

    await waitFor(() => expect(sendNotification).toHaveBeenCalledWith({
      title: "修 bug",
      body: "AITerm · 已取消",
    }));
    expect(sendTelegramMessage).toHaveBeenCalledWith("⏹️ AITerm — 修 bug\n已取消");
  });

  it("notify_desktop_on_finish 關閉時不發桌面通知，Telegram 照發", async () => {
    getTaskBoardConfig.mockResolvedValue({
      max_concurrent: 1,
      claude_command: "claude",
      auto_close_finished_tabs: true,
      notify_desktop_on_finish: false,
      notify_telegram_on_finish: true,
    });
    renderHook(() => useTaskCompletionNotifications(activeRef("other-tab")));
    taskFinishedHandler!({ payload: PAYLOAD });

    await waitFor(() => expect(sendTelegramMessage).toHaveBeenCalled());
    expect(sendNotification).not.toHaveBeenCalled();
  });

  it("notify_telegram_on_finish 關閉時不發 Telegram，桌面通知照發", async () => {
    getTaskBoardConfig.mockResolvedValue({
      max_concurrent: 1,
      claude_command: "claude",
      auto_close_finished_tabs: true,
      notify_desktop_on_finish: true,
      notify_telegram_on_finish: false,
    });
    renderHook(() => useTaskCompletionNotifications(activeRef("other-tab")));
    taskFinishedHandler!({ payload: PAYLOAD });

    await waitFor(() => expect(sendNotification).toHaveBeenCalled());
    expect(sendTelegramMessage).not.toHaveBeenCalled();
  });

  it("沒有通知權限時不發桌面通知", async () => {
    ensureNotificationPermission.mockResolvedValue(false);
    renderHook(() => useTaskCompletionNotifications(activeRef("other-tab")));
    taskFinishedHandler!({ payload: PAYLOAD });

    await waitFor(() => expect(sendTelegramMessage).toHaveBeenCalled());
    expect(sendNotification).not.toHaveBeenCalled();
  });

  it("Telegram 發送失敗不拋出、不影響桌面通知", async () => {
    sendTelegramMessage.mockRejectedValue(new Error("no bot token configured"));
    renderHook(() => useTaskCompletionNotifications(activeRef("other-tab")));
    taskFinishedHandler!({ payload: PAYLOAD });

    await waitFor(() => expect(sendNotification).toHaveBeenCalled());
  });
});
```

- [ ] **Step 2: 執行測試確認會紅**

Run: `npm run test -- useTaskCompletionNotifications`
Expected: FAIL — `Cannot find module './useTaskCompletionNotifications'`

- [ ] **Step 3: 寫實作**

建立 `src/components/TaskBoard/useTaskCompletionNotifications.ts`：

```ts
import { useEffect } from "react";
import type { RefObject } from "react";
import { listen } from "@tauri-apps/api/event";
import { sendNotification } from "@tauri-apps/plugin-notification";

import { unlistenOnCleanup } from "../../lib/eventSubscription";
import { ensureNotificationPermission } from "../../lib/notifyPermission";
import { getTaskBoardConfig, type TaskOutcome } from "../../ipc/tasks";
import { sendTelegramMessage } from "../../ipc/telegram";

interface TaskFinishedPayload {
  project_id: string;
  task_id: string;
  tab_id: string;
  outcome: TaskOutcome;
  title: string;
  project_name: string;
  error_message: string | null;
}

function desktopBody(p: TaskFinishedPayload): string {
  if (p.outcome === "success") return `${p.project_name} · 完成`;
  if (p.outcome === "cancelled") return `${p.project_name} · 已取消`;
  return `${p.project_name} · 失敗：${p.error_message ?? ""}`;
}

function telegramText(p: TaskFinishedPayload): string {
  const icon = p.outcome === "success" ? "✅" : p.outcome === "cancelled" ? "⏹️" : "❌";
  const line2 =
    p.outcome === "success" ? "完成" :
    p.outcome === "cancelled" ? "已取消" :
    `失敗：${p.error_message ?? ""}`;
  return `${icon} ${p.project_name} — ${p.title}\n${line2}`;
}

/**
 * 派工卡片跑完（成功／失敗／取消）時，依設定分別發桌面通知與/或 Telegram
 * 訊息。掛在永遠存在的元件上（TerminalApp），理由跟 `useAutoCloseFinishedTabs`
 * 完全一樣——看板只有在該專案是當前分頁時才掛載，別的專案完成時沒人在聽。
 *
 * 桌面通知在完成的分頁正是使用者目前正在看的分頁時跳過；Telegram 不看這條
 * 規則，一律發送——人不在電腦前才是它存在的意義。兩條管道各自失敗都不拋出，
 * 不影響對方或卡片本身的完成流程。
 */
export function useTaskCompletionNotifications(activeIdRef: RefObject<string>): void {
  useEffect(() => {
    const un = listen<TaskFinishedPayload>("task-finished", (e) => {
      const payload = e.payload;
      void getTaskBoardConfig().then((cfg) => {
        if (cfg.notify_desktop_on_finish && payload.tab_id !== activeIdRef.current) {
          void ensureNotificationPermission().then((granted) => {
            if (granted) sendNotification({ title: payload.title, body: desktopBody(payload) });
          });
        }
        if (cfg.notify_telegram_on_finish) {
          void sendTelegramMessage(telegramText(payload)).catch(() => {});
        }
      });
    });
    return unlistenOnCleanup(un, "task-finished");
  }, [activeIdRef]);
}
```

- [ ] **Step 4: 執行測試確認會過**

Run: `npm run test -- useTaskCompletionNotifications`
Expected: PASS（8 個測試全過）

- [ ] **Step 5: Commit**

```bash
git add src/components/TaskBoard/useTaskCompletionNotifications.ts src/components/TaskBoard/useTaskCompletionNotifications.test.ts
git commit -m "feat(taskboard): notify on task completion via desktop + Telegram

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012Cbm9KPo2DAenqgoCk4hYH"
```

---

## Task 6: 前端 — 掛進 `TerminalApp.tsx`

**Files:**
- Modify: `src/components/TerminalApp.tsx:29,101-102`

- [ ] **Step 1: 加 import 與呼叫**

`src/components/TerminalApp.tsx:29` 附近（`useAutoCloseFinishedTabs` 的 import 旁邊）：

```ts
import { useAutoCloseFinishedTabs } from "./TaskBoard/useAutoCloseFinishedTabs";
import { useTaskCompletionNotifications } from "./TaskBoard/useTaskCompletionNotifications";
```

`src/components/TerminalApp.tsx:101-102`（`useAutoCloseFinishedTabs(activeIdRef);` 旁邊）：

```ts
  const activeIdRef = useRef(activeId);
  useAutoCloseFinishedTabs(activeIdRef);
  useTaskCompletionNotifications(activeIdRef);
```

- [ ] **Step 2: 確認既有的 TerminalApp 測試仍過**

Run: `npm run test -- TerminalApp`
Expected: PASS——`TerminalApp.taskBoard.test.tsx` 等既有測試檔案沒有 mock `useTaskCompletionNotifications` 用到的模組時，因為這些模組（`../../ipc/tasks`、`../../ipc/telegram`、`@tauri-apps/plugin-notification`、`../../lib/notifyPermission`、`@tauri-apps/api/event`）本來就已經被這些測試檔案 mock 過（同一批 `TerminalApp` 依賴），新 hook 不會引入新的未 mock 模組。若某個 `TerminalApp` 測試檔案沒有 mock `../../ipc/telegram`，會在這一步報錯——照其他測試檔案的既有 mock 方式（回傳 `Promise.resolve(undefined)` 之類的最小 stub）補上即可。

- [ ] **Step 3: Commit**

```bash
git add src/components/TerminalApp.tsx
git commit -m "feat(taskboard): wire useTaskCompletionNotifications into TerminalApp

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012Cbm9KPo2DAenqgoCk4hYH"
```

---

## Task 7: 前端 — 設定頁兩個新 checkbox

**Files:**
- Modify: `src/components/Settings/TaskBoardPage.tsx`
- Modify: `src/components/Settings/TaskBoardPage.test.tsx`

- [ ] **Step 1: 寫失敗的測試**

在 `src/components/Settings/TaskBoardPage.test.tsx` 頂部，`vi.mock("../../ipc/tasks", ...)` 旁邊加一個新 mock：

```ts
vi.mock("../../ipc/telegram", () => ({
  getTelegramConfig: vi.fn(),
}));
```

import 那行加：

```ts
import { getTaskBoardConfig, setTaskBoardConfig } from "../../ipc/tasks";
import { getTelegramConfig } from "../../ipc/telegram";
```

`beforeEach` 補上 Telegram 的預設 mock（未設定）：

```ts
beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(getTaskBoardConfig).mockResolvedValue({
    max_concurrent: 2,
    claude_command: "claude",
    auto_close_finished_tabs: true,
    notify_desktop_on_finish: true,
    notify_telegram_on_finish: true,
  });
  vi.mocked(getTelegramConfig).mockResolvedValue({ bot_token: null, chat_id: null });
});
```

把原本第 47-60 行「toggling the checkbox sends the new value」改成明確指名 auto-close 那個 checkbox（因為現在頁面上不再只有一個 checkbox）：

```ts
  it("toggling the auto-close checkbox sends the new value", async () => {
    const user = userEvent.setup();
    view();
    await waitFor(() => screen.getByDisplayValue("2"));
    const checkbox = screen.getByRole("checkbox", { name: /自動關閉分頁|Auto-close tabs/ });
    expect(checkbox).toBeChecked();
    await user.click(checkbox);
    await user.click(screen.getByRole("button", { name: /儲存|Save/ }));
    await waitFor(() =>
      expect(setTaskBoardConfig).toHaveBeenCalledWith(
        expect.objectContaining({ auto_close_finished_tabs: false }),
      ),
    );
  });
```

在同一個 `describe` 區塊最後加三個新測試：

```ts
  it("預設顯示桌面通知 checkbox，已勾選", async () => {
    view();
    await waitFor(() => screen.getByDisplayValue("2"));
    expect(
      screen.getByRole("checkbox", { name: /完成時發桌面通知|Desktop notification/ }),
    ).toBeChecked();
  });

  it("Telegram 未設定時不顯示 Telegram checkbox", async () => {
    view();
    await waitFor(() => screen.getByDisplayValue("2"));
    expect(
      screen.queryByRole("checkbox", { name: /Telegram/ }),
    ).not.toBeInTheDocument();
  });

  it("Telegram 已設定時顯示 checkbox，已勾選", async () => {
    vi.mocked(getTelegramConfig).mockResolvedValue({ bot_token: "abc", chat_id: "123" });
    view();
    await waitFor(() =>
      expect(screen.getByRole("checkbox", { name: /Telegram/ })).toBeChecked(),
    );
  });
```

- [ ] **Step 2: 執行測試確認會紅**

Run: `npm run test -- TaskBoardPage`
Expected: FAIL——`getTelegramConfig` 不存在於 `../../ipc/telegram` 的 mock 外的實際 import 檢查（此時 `TaskBoardPage.tsx` 還沒 import 它），且畫面上還沒有新 checkbox

- [ ] **Step 3: 改 `TaskBoardPage.tsx`**

```tsx
import { useCallback, useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { getTaskBoardConfig, setTaskBoardConfig, type TaskBoardConfig } from "../../ipc/tasks";
import { getTelegramConfig } from "../../ipc/telegram";
import "./TaskBoardPage.css";

export function TaskBoardPage() {
  const { t } = useLocale();
  const [cfg, setCfg] = useState<TaskBoardConfig | null>(null);
  const [telegramConfigured, setTelegramConfigured] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    void getTaskBoardConfig().then(setCfg);
    void getTelegramConfig().then((tc) =>
      setTelegramConfigured(Boolean(tc.bot_token) && Boolean(tc.chat_id)),
    );
  }, []);

  const save = useCallback(async () => {
    if (!cfg) return;
    setSaving(true);
    try {
      const max = Number.isNaN(cfg.max_concurrent) ? 1 : cfg.max_concurrent;
      await setTaskBoardConfig({ ...cfg, max_concurrent: max });
      setSaved(true);
    } finally {
      setSaving(false);
    }
  }, [cfg]);

  if (!cfg) return <div className="task-board-page" />;

  return (
    <div className="task-board-page">
      <h2>{t.board_settings_title}</h2>
      <p className="task-board-desc">{t.board_settings_desc}</p>

      <section className="task-board-section">
        <label className="task-board-field">
          <span>{t.board_settings_max_concurrent}</span>
          <input
            type="number"
            min={1}
            max={16}
            value={Number.isNaN(cfg.max_concurrent) ? "" : cfg.max_concurrent}
            onChange={(e) => {
              setSaved(false);
              setCfg({ ...cfg, max_concurrent: e.target.valueAsNumber });
            }}
          />
          <span className="task-board-hint">{t.board_settings_max_concurrent_hint}</span>
        </label>

        <label className="task-board-field">
          <span>{t.board_settings_claude_command}</span>
          <input
            type="text"
            value={cfg.claude_command}
            onChange={(e) => {
              setSaved(false);
              setCfg({ ...cfg, claude_command: e.target.value });
            }}
          />
          <span className="task-board-hint">{t.board_settings_claude_command_hint}</span>
        </label>

        <label className="task-board-field task-board-field--checkbox">
          <input
            type="checkbox"
            className="task-board-checkbox"
            checked={cfg.auto_close_finished_tabs}
            onChange={(e) => {
              setSaved(false);
              setCfg({ ...cfg, auto_close_finished_tabs: e.target.checked });
            }}
          />
          <span>{t.board_settings_auto_close}</span>
          <span className="task-board-hint">{t.board_settings_auto_close_hint}</span>
        </label>

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
      </section>

      <div className="task-board-actions">
        <button onClick={() => void save()} disabled={saving}>
          {saved ? `${t.board_settings_saved} ✓` : t.board_save}
        </button>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: 執行測試確認會過**

Run: `npm run test -- TaskBoardPage`
Expected: FAIL——`t.board_settings_notify_desktop` 等 i18n key 還不存在，畫面上文字是 `undefined`，`getByRole("checkbox", { name: /.../ })` 找不到符合的 accessible name。這是預期中的紅燈，Task 8 補上 i18n 後才會轉綠，先繼續往下不要在這一步卡住。

- [ ] **Step 5: Commit**（先不 commit，跟 Task 8 一起——這個 Task 的測試要等 i18n key 補齊才會真的綠燈；如果你的流程要求每個 Task 都要綠燈才能 commit，直接把 Task 7 跟 Task 8 合併執行）

---

## Task 8: i18n — 四個新 key（en / zh-TW）

**Files:**
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: zh-TW 區塊加 key**

`src/lib/i18n.ts:83-84`（`board_settings_auto_close_hint` 後面）加：

```ts
    board_settings_notify_desktop: "完成時發桌面通知",
    board_settings_notify_desktop_hint: "卡片跑完（成功／失敗／取消）時彈出系統通知。你正在看的分頁不會重複彈。",
    board_settings_notify_telegram: "完成時發 Telegram 通知",
    board_settings_notify_telegram_hint: "失敗時會附上失敗原因。需要先在設定 →「Telegram 整合」設定 bot 才會出現這個選項。",
```

- [ ] **Step 2: en 區塊加對應 key**

`src/lib/i18n.ts:1591-1592`（`board_settings_auto_close_hint` 的 en 版本後面）加：

```ts
    board_settings_notify_desktop: "Desktop notification when a task finishes",
    board_settings_notify_desktop_hint: "Shows a system notification when a card finishes (success, failure, or cancellation). The tab you're currently viewing never pops a duplicate notification.",
    board_settings_notify_telegram: "Telegram notification when a task finishes",
    board_settings_notify_telegram_hint: "Failures include the error reason. This option only appears once a Telegram bot is configured under Settings → \"Telegram Integration\".",
```

- [ ] **Step 3: 執行 Task 7 + 8 的測試確認會過**

Run: `npx tsc -b && npm run test -- TaskBoardPage`
Expected: PASS——所有 `TaskBoardPage` 測試（含 Task 7 新增的三個）全綠

- [ ] **Step 4: Commit（涵蓋 Task 7 + 8）**

```bash
git add src/components/Settings/TaskBoardPage.tsx src/components/Settings/TaskBoardPage.test.tsx src/lib/i18n.ts
git commit -m "feat(taskboard): add settings UI for completion notifications

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012Cbm9KPo2DAenqgoCk4hYH"
```

---

## Task 9: 全專案驗證

**Files:** 無新增/修改——純驗證步驟。

- [ ] **Step 1: 後端完整測試（不是 `--lib`）**

Run: `cd src-tauri && cargo test`
Expected: 全綠。**必須是完整的 `cargo test`，不是 `--lib`**——`--lib` 不會編譯 `src-tauri/tests/` 底下的整合測試，之前有過整個分支全綠、推上去才在三平台炸編譯錯誤的教訓。

- [ ] **Step 2: 前端完整測試**

Run: `npm run test`
Expected: 全綠

- [ ] **Step 3: 型別檢查**

Run: `npx tsc -b`
Expected: 無錯誤

- [ ] **Step 4: Lint**

Run: `npm run lint`
Expected: 無錯誤

- [ ] **Step 5: 手動驗證的已知限制**

`tauri-plugin-notification` 在 `tauri:dev` 模式下（未打包的 dev binary）一律以「終端機」身分送出桌面通知（`desktop.rs:207-214`，`useMailSync.ts` 的既有註解也記錄了這件事），且外掛本身吞掉 `notification.show()` 的回傳值，失敗不可觀察。**這代表這個功能沒辦法在 `npm run tauri:dev` 底下真的看到桌面通知彈出**，只能：
  1. 靠 Task 5 的單元測試驗證「呼叫了 `sendNotification`，且參數正確」；
  2. 若要肉眼確認真的會彈通知，需要一次 `npm run tauri:build` 出正式簽章的 bundle 才有意義。
  Telegram 那條路徑沒有這個限制——`sendTelegramMessage` 是純 HTTP 呼叫，`tauri:dev` 下也會真的打到 Telegram Bot API，可以在 dev 模式下用一組測試用的 bot token/chat id 手動觸發一張卡片跑完，確認訊息真的送到 Telegram。

這一步不阻塞完成——只是明確記錄「單元測試綠燈」跟「肉眼確認桌面通知真的會彈」是兩件不同的事，後者需要正式 build 才能做，不在這次執行範圍內自動完成。
