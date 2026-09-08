# 派工任務完成後自動清理終端機分頁 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 派工卡片跑完（成功或取消）之後，若不是使用者目前正在看的分頁，自動觸發既有的 `aiterm:close-tab` 機制關掉它（連帶砍掉背後的 PTY/claude 行程）；失敗的任務保留分頁方便除錯；整個行為可以在 Task Board 設定頁關掉。

**Architecture:** 不新增任何後端關閉分頁/砍行程的邏輯——`TaskCard` 刪除卡片時已經在用的 `aiterm:close-tab` 視窗事件、`TerminalApp.tsx` 既有的 `handleCloseTab`、以及 `TerminalView` 卸載時呼叫 `closePty()`，這條路徑已經存在且驗證過。這次只在後端 `task-finished` 事件補上 `outcome` 欄位，前端新增一個掛在永遠存在的 `TerminalApp.tsx` 上的 hook，收到 `task-finished` 時依 outcome／是否為目前分頁／設定開關三個條件決定要不要 dispatch 既有的 `aiterm:close-tab`。

**Tech Stack:** Rust（serde/sqlx 既有型別）、React 19 + TypeScript（Vitest + RTL）。

**規格：** `docs/superpowers/specs/2026-09-08-auto-close-finished-task-tabs-design.md`

---

### Task 1：Rust — `TaskFinishedEvent.outcome` 與 `TaskBoardConfig.auto_close_finished_tabs`

**Files:**
- Modify: `src-tauri/src/config/types.rs`（`TaskBoardConfig` 結構與 `Default` impl，約第 191-219 行）
- Modify: `src-tauri/src/commands/task_board_config.rs`（`apply_editable_task_board_fields` 與其測試模組）
- Modify: `src-tauri/src/tasks/scheduler.rs`（`TaskFinishedEvent`，約第 57-65 行；emit 處約第 176-183 行）

這個 task 是純資料結構改動（不是新行為），照使用者要求的順序先做，用既有測試 + 一個新增的小測試驗證即可，不需要大量新測試。

- [ ] **Step 1：`TaskBoardConfig` 加欄位**

把 `src-tauri/src/config/types.rs` 的：

```rust
pub struct TaskBoardConfig {
    /// Global cap on tasks in the `running` column at once. A per-card
    /// `parallel_ok = false` flag further restricts (a solo card waits for
    /// an empty running set and blocks others while it runs).
    #[serde(default = "default_task_board_max_concurrent")]
    pub max_concurrent: u32,
    /// The CLI launched in each dispatched tab. `claude` by default; a user
    /// could point this at another agent, but that's not a supported feature.
    #[serde(default = "default_claude_command")]
    pub claude_command: String,
    /// 已知專案資料夾的絕對路徑。只存路徑——名稱與 id 每次啟動時
    /// 從各自的 `.aitprj` 讀取，這樣使用者在 Finder 裡改了專案檔，
    /// App 下次啟動就會看到。
    #[serde(default)]
    pub project_paths: Vec<String>,
}

impl Default for TaskBoardConfig {
    fn default() -> Self {
        Self {
            max_concurrent: default_task_board_max_concurrent(),
            claude_command: default_claude_command(),
            project_paths: Vec::new(),
        }
    }
}
```

改成：

```rust
pub struct TaskBoardConfig {
    /// Global cap on tasks in the `running` column at once. A per-card
    /// `parallel_ok = false` flag further restricts (a solo card waits for
    /// an empty running set and blocks others while it runs).
    #[serde(default = "default_task_board_max_concurrent")]
    pub max_concurrent: u32,
    /// The CLI launched in each dispatched tab. `claude` by default; a user
    /// could point this at another agent, but that's not a supported feature.
    #[serde(default = "default_claude_command")]
    pub claude_command: String,
    /// 已知專案資料夾的絕對路徑。只存路徑——名稱與 id 每次啟動時
    /// 從各自的 `.aitprj` 讀取，這樣使用者在 Finder 裡改了專案檔，
    /// App 下次啟動就會看到。
    #[serde(default)]
    pub project_paths: Vec<String>,
    /// 派工卡片跑完（成功／取消）後，是否自動關閉它的終端機分頁（連帶
    /// 砍掉背後的 PTY/claude 行程）。失敗的卡片一律保留分頁，不受這個
    /// 設定影響。使用者目前正在看的分頁也不會被自動關閉。
    #[serde(default = "default_true")]
    pub auto_close_finished_tabs: bool,
}

impl Default for TaskBoardConfig {
    fn default() -> Self {
        Self {
            max_concurrent: default_task_board_max_concurrent(),
            claude_command: default_claude_command(),
            project_paths: Vec::new(),
            auto_close_finished_tabs: true,
        }
    }
}
```

`default_true` 已經是這個檔案第 322 行的既有函式，不用新寫。

- [ ] **Step 2：`apply_editable_task_board_fields` 帶上新欄位**

把 `src-tauri/src/commands/task_board_config.rs` 的：

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
    }
}
```

改成：

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
    }
}
```

跟 `max_concurrent`／`claude_command` 一樣是「使用者在設定頁可以改的欄位」，直接採用 `incoming` 的值——不像 `project_paths` 那樣需要保留 `current`。

- [ ] **Step 3：修既有測試裡的 5 個 `TaskBoardConfig { ... }` 字面值**

同一個檔案 `#[cfg(test)] mod tests` 裡，把這 5 處各自加一行 `auto_close_finished_tabs: true,`（`mk` 那個 closure 也一樣）：

```rust
    fn current() -> TaskBoardConfig {
        TaskBoardConfig {
            max_concurrent: 5,
            claude_command: "claude".to_string(),
            project_paths: vec!["/projects/a".to_string(), "/projects/b".to_string()],
            auto_close_finished_tabs: true,
        }
    }
```

```rust
    fn saving_settings_does_not_wipe_the_project_list() {
        let incoming = TaskBoardConfig {
            max_concurrent: 3,
            claude_command: "claude".to_string(),
            project_paths: Vec::new(),
            auto_close_finished_tabs: true,
        };
```

```rust
    fn project_paths_in_the_payload_are_ignored() {
        let incoming = TaskBoardConfig {
            max_concurrent: 5,
            claude_command: "claude".to_string(),
            project_paths: vec!["/injected".to_string()],
            auto_close_finished_tabs: true,
        };
```

```rust
    fn max_concurrent_is_clamped_to_1_16() {
        let mk = |n: u32| TaskBoardConfig {
            max_concurrent: n,
            claude_command: "claude".to_string(),
            project_paths: Vec::new(),
            auto_close_finished_tabs: true,
        };
```

```rust
    fn a_blank_claude_command_falls_back_to_claude() {
        let incoming = TaskBoardConfig {
            max_concurrent: 5,
            claude_command: "   ".to_string(),
            project_paths: Vec::new(),
            auto_close_finished_tabs: true,
        };
```

- [ ] **Step 4：新增一個小測試，證明開關真的會從 `incoming` 採用**

在同一個 `mod tests` 裡，`a_blank_claude_command_falls_back_to_claude` 後面加：

```rust
    #[test]
    fn auto_close_finished_tabs_is_taken_from_incoming() {
        let mut incoming = current();
        incoming.auto_close_finished_tabs = false;
        assert!(!apply_editable_task_board_fields(&current(), incoming).auto_close_finished_tabs);
    }
```

- [ ] **Step 5：執行測試確認全部通過**

Run: `cd src-tauri && cargo test task_board_config:: -- --nocapture`
Expected: PASS（既有 4 個 + 新增 1 個，共 5 個）

- [ ] **Step 6：`TaskFinishedEvent` 加 `outcome` 欄位**

把 `src-tauri/src/tasks/scheduler.rs` 的：

```rust
#[derive(Clone, serde::Serialize)]
struct TaskFinishedEvent {
    project_id: String,
    task_id: String,
    tab_id: String,
}
```

改成：

```rust
#[derive(Clone, serde::Serialize)]
struct TaskFinishedEvent {
    project_id: String,
    task_id: String,
    tab_id: String,
    outcome: String,
}
```

把 emit 的地方：

```rust
            let _ = app.emit(
                "task-finished",
                TaskFinishedEvent {
                    project_id: project_id.clone(),
                    task_id: task_id.clone(),
                    tab_id: tab_id.clone(),
                },
            );
```

改成：

```rust
            let _ = app.emit(
                "task-finished",
                TaskFinishedEvent {
                    project_id: project_id.clone(),
                    task_id: task_id.clone(),
                    tab_id: tab_id.clone(),
                    outcome: outcome.as_str().to_string(),
                },
            );
```

`outcome: monitor::TaskOutcome` 在這個 async block 裡已經在 scope（`monitor::watch(...)` 的回傳值），`as_str()` 回傳 `"success"`/`"failed"`/`"cancelled"`（見 `monitor.rs` 的 `TaskOutcome::as_str`）。

- [ ] **Step 7：整個 crate 編譯並跑一次完整測試**

Run: `cd src-tauri && cargo build && cargo test`
Expected: 編譯成功，全部 PASS，沒有既有測試因為這次改動失敗。

- [ ] **Step 8：Commit**

```bash
git add src-tauri/src/config/types.rs src-tauri/src/commands/task_board_config.rs src-tauri/src/tasks/scheduler.rs
git commit -m "$(cat <<'EOF'
feat(tasks): add auto_close_finished_tabs config + outcome on task-finished

Pure data-structure changes: TaskBoardConfig gets a new user-editable
toggle (default true), and TaskFinishedEvent now carries the task's
outcome so the frontend doesn't need an extra round-trip to learn it.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KCiBfURnSHp4US8zS9dCFk
EOF
)"
```

---

### Task 2：前端型別 — `TaskBoardConfig` 加欄位，修既有測試 fixture

**Files:**
- Modify: `src/ipc/tasks.ts`
- Modify: `src/components/Settings/TaskBoardPage.test.tsx`
- Modify: `src/components/TerminalApp.taskBoard.test.tsx`

- [ ] **Step 1：`TaskBoardConfig` 型別加欄位**

把 `src/ipc/tasks.ts` 的：

```typescript
export interface TaskBoardConfig {
  max_concurrent: number;
  claude_command: string;
}
```

改成：

```typescript
export interface TaskBoardConfig {
  max_concurrent: number;
  claude_command: string;
  auto_close_finished_tabs: boolean;
}
```

- [ ] **Step 2：型別檢查，確認紅燈落在預期的兩個測試 fixture**

Run: `npx tsc -b`
Expected: 報錯，各一處在 `TaskBoardPage.test.tsx` 與 `TerminalApp.taskBoard.test.tsx`——兩邊的 `getTaskBoardConfig` mock 回傳值都缺 `auto_close_finished_tabs`。

- [ ] **Step 3：修 `TaskBoardPage.test.tsx` 的 mock**

把：

```typescript
  vi.mocked(getTaskBoardConfig).mockResolvedValue({ max_concurrent: 2, claude_command: "claude" });
```

改成：

```typescript
  vi.mocked(getTaskBoardConfig).mockResolvedValue({
    max_concurrent: 2,
    claude_command: "claude",
    auto_close_finished_tabs: true,
  });
```

- [ ] **Step 4：修 `TerminalApp.taskBoard.test.tsx` 的 mock**

把：

```typescript
  getTaskBoardConfig: vi.fn().mockResolvedValue({ max_concurrent: 2, claude_command: "claude" }),
```

改成：

```typescript
  getTaskBoardConfig: vi.fn().mockResolvedValue({
    max_concurrent: 2,
    claude_command: "claude",
    auto_close_finished_tabs: true,
  }),
```

- [ ] **Step 5：型別檢查確認乾淨**

Run: `npx tsc -b`
Expected: 無錯誤。

- [ ] **Step 6：跑這兩個測試檔確認沒被連帶弄壞**

Run: `npx vitest run src/components/Settings/TaskBoardPage.test.tsx src/components/TerminalApp.taskBoard.test.tsx`
Expected: PASS。

- [ ] **Step 7：Commit**

```bash
git add src/ipc/tasks.ts src/components/Settings/TaskBoardPage.test.tsx src/components/TerminalApp.taskBoard.test.tsx
git commit -m "$(cat <<'EOF'
feat(tasks): add auto_close_finished_tabs to the frontend TaskBoardConfig type

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KCiBfURnSHp4US8zS9dCFk
EOF
)"
```

---

### Task 3：新 hook `useAutoCloseFinishedTabs`

**Files:**
- Create: `src/components/TaskBoard/useAutoCloseFinishedTabs.ts`
- Test: `src/components/TaskBoard/useAutoCloseFinishedTabs.test.ts`

跟 `useTranscriptUpgrader.ts` 同一個資料夾、同一種「單一職責、可獨立測試」的模式。用 `@testing-library/react` 的 `renderHook` 測試，不需要掛整個 `TerminalApp`。

- [ ] **Step 1：寫失敗測試**

```typescript
// src/components/TaskBoard/useAutoCloseFinishedTabs.test.ts
import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RefObject } from "react";

const getTaskBoardConfig = vi.fn();
vi.mock("../../ipc/tasks", () => ({
  getTaskBoardConfig: (...a: unknown[]) => getTaskBoardConfig(...a),
}));

let taskFinishedHandler: ((e: { payload: unknown }) => void) | null = null;
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((event: string, handler: (e: { payload: unknown }) => void) => {
    if (event === "task-finished") taskFinishedHandler = handler;
    return Promise.resolve(() => {});
  }),
}));

import { useAutoCloseFinishedTabs } from "./useAutoCloseFinishedTabs";

function activeRef(id: string): RefObject<string> {
  return { current: id };
}

const PAYLOAD = { project_id: "p1", task_id: "t1", tab_id: "tab-9", outcome: "success" };

beforeEach(() => {
  vi.clearAllMocks();
  taskFinishedHandler = null;
  getTaskBoardConfig.mockResolvedValue({
    max_concurrent: 1,
    claude_command: "claude",
    auto_close_finished_tabs: true,
  });
});

describe("useAutoCloseFinishedTabs", () => {
  it("成功、非目前分頁、設定開啟 → dispatch aiterm:close-tab", async () => {
    renderHook(() => useAutoCloseFinishedTabs(activeRef("other-tab")));
    expect(taskFinishedHandler).not.toBeNull();

    const events: CustomEvent<{ tabId?: string }>[] = [];
    const onClose = (e: Event) => events.push(e as CustomEvent<{ tabId?: string }>);
    window.addEventListener("aiterm:close-tab", onClose);
    try {
      taskFinishedHandler!({ payload: PAYLOAD });
      await waitFor(() => expect(events).toHaveLength(1));
      expect(events[0].detail.tabId).toBe("tab-9");
    } finally {
      window.removeEventListener("aiterm:close-tab", onClose);
    }
  });

  it("取消也會關", async () => {
    renderHook(() => useAutoCloseFinishedTabs(activeRef("other-tab")));
    const events: CustomEvent<{ tabId?: string }>[] = [];
    const onClose = (e: Event) => events.push(e as CustomEvent<{ tabId?: string }>);
    window.addEventListener("aiterm:close-tab", onClose);
    try {
      taskFinishedHandler!({ payload: { ...PAYLOAD, outcome: "cancelled" } });
      await waitFor(() => expect(events).toHaveLength(1));
    } finally {
      window.removeEventListener("aiterm:close-tab", onClose);
    }
  });

  it("失敗不關", async () => {
    renderHook(() => useAutoCloseFinishedTabs(activeRef("other-tab")));
    const events: CustomEvent[] = [];
    const onClose = (e: Event) => events.push(e as CustomEvent);
    window.addEventListener("aiterm:close-tab", onClose);
    try {
      taskFinishedHandler!({ payload: { ...PAYLOAD, outcome: "failed" } });
      await new Promise((r) => setTimeout(r, 0));
      expect(events).toHaveLength(0);
      expect(getTaskBoardConfig).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener("aiterm:close-tab", onClose);
    }
  });

  it("是目前正在看的分頁時不關", async () => {
    renderHook(() => useAutoCloseFinishedTabs(activeRef("tab-9")));
    const events: CustomEvent[] = [];
    const onClose = (e: Event) => events.push(e as CustomEvent);
    window.addEventListener("aiterm:close-tab", onClose);
    try {
      taskFinishedHandler!({ payload: PAYLOAD });
      await new Promise((r) => setTimeout(r, 0));
      expect(events).toHaveLength(0);
      expect(getTaskBoardConfig).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener("aiterm:close-tab", onClose);
    }
  });

  it("設定關閉時不關", async () => {
    getTaskBoardConfig.mockResolvedValue({
      max_concurrent: 1,
      claude_command: "claude",
      auto_close_finished_tabs: false,
    });
    renderHook(() => useAutoCloseFinishedTabs(activeRef("other-tab")));
    const events: CustomEvent[] = [];
    const onClose = (e: Event) => events.push(e as CustomEvent);
    window.addEventListener("aiterm:close-tab", onClose);
    try {
      taskFinishedHandler!({ payload: PAYLOAD });
      await waitFor(() => expect(getTaskBoardConfig).toHaveBeenCalled());
      await new Promise((r) => setTimeout(r, 0));
      expect(events).toHaveLength(0);
    } finally {
      window.removeEventListener("aiterm:close-tab", onClose);
    }
  });
});
```

- [ ] **Step 2：執行測試確認失敗**

Run: `npx vitest run src/components/TaskBoard/useAutoCloseFinishedTabs.test.ts`
Expected: FAIL — 找不到 `./useAutoCloseFinishedTabs` 模組。

- [ ] **Step 3：實作 hook**

```typescript
// src/components/TaskBoard/useAutoCloseFinishedTabs.ts
import { useEffect } from "react";
import type { RefObject } from "react";
import { listen } from "@tauri-apps/api/event";

import { unlistenOnCleanup } from "../../lib/eventSubscription";
import { getTaskBoardConfig } from "../../ipc/tasks";

interface TaskFinishedPayload {
  project_id: string;
  task_id: string;
  tab_id: string;
  outcome: string;
}

/**
 * 派工卡片跑完後，若不是使用者目前正在看的分頁、結局不是失敗、且設定
 * 開著，就 dispatch 既有的 `aiterm:close-tab`（TaskCard 刪除卡片時已經
 * 在用的同一個事件）——不新增任何關閉分頁/砍行程的邏輯，只是在對的
 * 時機觸發已經存在、已經驗證過的路徑（見 `TerminalApp.tsx` 的
 * `aiterm:close-tab` 監聽器與 `TerminalView` 卸載時的 `closePty()`）。
 *
 * 必須掛在**永遠存在**的元件上（TerminalApp），理由跟 `useTranscriptUpgrader`
 * 完全一樣——看板只有在該專案是當前分頁時才掛載，別的專案完成時沒人在聽。
 *
 * `activeIdRef` 由呼叫端傳入並在事件觸發當下讀 `.current`，避免閉包
 * 捕捉到掛載當時的舊值。
 */
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

- [ ] **Step 4：執行測試確認通過**

Run: `npx vitest run src/components/TaskBoard/useAutoCloseFinishedTabs.test.ts`
Expected: PASS（5 個測試）

- [ ] **Step 5：Commit**

```bash
git add src/components/TaskBoard/useAutoCloseFinishedTabs.ts src/components/TaskBoard/useAutoCloseFinishedTabs.test.ts
git commit -m "$(cat <<'EOF'
feat(tasks): add useAutoCloseFinishedTabs hook

Listens for task-finished; on success/cancelled (not failed), when the
tab isn't the one currently being viewed, and when the setting is on,
dispatches the existing aiterm:close-tab event — the same bridge
TaskCard's delete-with-tab flow already uses. No new close/kill logic.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KCiBfURnSHp4US8zS9dCFk
EOF
)"
```

---

### Task 4：接進 `TerminalApp.tsx`

**Files:**
- Modify: `src/components/TerminalApp.tsx`

沒有新的自動化測試——`TerminalApp.tsx` 是這個 app 的殼元件，掛整個元件測試成本很高，而且核心決策邏輯已經在 Task 3 完整測過。這裡純粹是一行接線，用型別檢查＋既有測試套件確認沒弄壞任何東西。

- [ ] **Step 1：加 import**

把 `src/components/TerminalApp.tsx` 第 28 行：

```typescript
import { useTranscriptUpgrader } from "./TaskBoard/useTranscriptUpgrader";
```

改成：

```typescript
import { useTranscriptUpgrader } from "./TaskBoard/useTranscriptUpgrader";
import { useAutoCloseFinishedTabs } from "./TaskBoard/useAutoCloseFinishedTabs";
```

- [ ] **Step 2：呼叫 hook**

`activeIdRef` 在第 100 行 `const activeIdRef = useRef(activeId);` 定義，`useTranscriptUpgrader()` 在第 77 行呼叫——`activeIdRef` 定義在它之後，所以新的呼叫要放在 `activeIdRef` 宣告之後，不能跟 `useTranscriptUpgrader()` 放在一起。

在 `const activeIdRef = useRef(activeId);`（第 100 行）後面找一個方便的空位——直接接在它後面加一行即可：

```typescript
  const activeIdRef = useRef(activeId);
  useAutoCloseFinishedTabs(activeIdRef);
```

- [ ] **Step 3：型別檢查**

Run: `npx tsc -b`
Expected: 無錯誤。

- [ ] **Step 4：跑跟 TerminalApp 相關的既有測試，確認沒有連帶壞掉**

Run: `npx vitest run src/components/TerminalApp`
Expected: PASS（既有所有 `TerminalApp.*.test.tsx` 全部綠燈）。

- [ ] **Step 5：Commit**

```bash
git add src/components/TerminalApp.tsx
git commit -m "$(cat <<'EOF'
feat(tasks): wire useAutoCloseFinishedTabs into TerminalApp

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KCiBfURnSHp4US8zS9dCFk
EOF
)"
```

---

### Task 5：Settings 頁面 checkbox + i18n

**Files:**
- Modify: `src/components/Settings/TaskBoardPage.tsx`
- Modify: `src/components/Settings/TaskBoardPage.css`
- Modify: `src/components/Settings/TaskBoardPage.test.tsx`
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1：i18n 加字串**

在 `src/lib/i18n.ts`，找 `board_settings_claude_command_hint` 那一行（zh-TW 區塊），在它後面加：

```typescript
    board_settings_auto_close: "任務完成後自動關閉分頁",
    board_settings_auto_close_hint: "只有成功或取消的任務會自動關閉；失敗的任務會保留分頁方便除錯。你正在看的分頁不會被自動關閉。",
```

在 en 區塊對應位置（用 `grep -n "board_settings_claude_command_hint" src/lib/i18n.ts` 找實際行號）加：

```typescript
    board_settings_auto_close: "Auto-close tabs when tasks finish",
    board_settings_auto_close_hint: "Only successful or cancelled tasks close automatically; failed tasks keep their tab open for debugging. The tab you're currently viewing is never auto-closed.",
```

- [ ] **Step 2：寫失敗測試**

在 `src/components/Settings/TaskBoardPage.test.tsx`，把 `beforeEach` 裡的 mock（已經在 Task 2 加過 `auto_close_finished_tabs: true`）保持不變，在檔案最後一個 `it` 後面加：

```typescript
  it("toggling the checkbox sends the new value", async () => {
    const user = userEvent.setup();
    view();
    await waitFor(() => screen.getByDisplayValue("2"));
    const checkbox = screen.getByRole("checkbox");
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

- [ ] **Step 3：執行測試確認失敗**

Run: `npx vitest run src/components/Settings/TaskBoardPage.test.tsx`
Expected: FAIL — 找不到 `role: checkbox`（頁面上還沒有這個 checkbox）。

- [ ] **Step 4：`TaskBoardPage.tsx` 加 checkbox**

把 `src/components/Settings/TaskBoardPage.tsx` 的：

```tsx
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
      </section>
```

改成：

```tsx
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
      </section>
```

- [ ] **Step 5：`TaskBoardPage.css` 加 checkbox 樣式**

在 `src/components/Settings/TaskBoardPage.css` 檔案最後加：

```css
.task-board-field--checkbox {
  flex-direction: row;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
}

.task-board-checkbox {
  accent-color: var(--accent, #a855f7);
  width: 15px;
  height: 15px;
  margin: 0;
}
```

- [ ] **Step 6：執行測試確認通過**

Run: `npx vitest run src/components/Settings/TaskBoardPage.test.tsx`
Expected: PASS（既有 2 個 + 新增 1 個，共 3 個）

- [ ] **Step 7：型別檢查**

Run: `npx tsc -b`
Expected: 無錯誤。

- [ ] **Step 8：Commit**

```bash
git add src/components/Settings/TaskBoardPage.tsx src/components/Settings/TaskBoardPage.css src/components/Settings/TaskBoardPage.test.tsx src/lib/i18n.ts
git commit -m "$(cat <<'EOF'
feat(tasks): add auto-close-finished-tabs checkbox to Task Board settings

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KCiBfURnSHp4US8zS9dCFk
EOF
)"
```

---

### Task 6：全套驗證

**Files:** 無新增/修改，純驗證。

- [ ] **Step 1：Rust 全套測試**

Run: `cd src-tauri && cargo test`
Expected: 全部 PASS。

- [ ] **Step 2：前端全套測試**

Run: `npm run test`
Expected: 全部 PASS。

- [ ] **Step 3：型別檢查**

Run: `npx tsc -b`
Expected: 無錯誤。

- [ ] **Step 4：Lint**

Run: `npm run lint`
Expected: 這次改動的檔案不應出現新的 lint 錯誤（`useAutoCloseFinishedTabs.ts`、`TerminalApp.tsx`、`TaskBoardPage.tsx`、`ipc/tasks.ts`、`i18n.ts`、Rust 那三個檔案不受 `npm run lint` 涵蓋）。

- [ ] **Step 5：若任一步驟失敗，回到對應 Task 修正，不要略過**

- [ ] **Step 6：全部通過後跟使用者回報完成狀態**，附上每個驗證指令的結果摘要。每個 Task 已經各自 commit 過，這步不需要額外 commit。
