# 關閉視窗前的工作進行中確認 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 任一分頁有進行中的工作時，關閉視窗（✕／Alt+F4／Cmd+Q）先跳一個確認框，列出哪些分頁在忙什麼；全部閒置則直接關。

**Architecture:** 各分頁元件另外註冊「忙碌探針」（只問不彈，與既有 close guard 平行）；`TerminalApp` 持有探針表，新 hook `useWindowCloseGuard` 攔截 `onCloseRequested` 與 Rust 送來的 `app://quit-requested`，兩條入口共用同一個 `attempt()`：閒置就 `set_quit_confirmed` + `destroy()`，有忙碌就顯示 `CloseConfirmDialog`。Rust 端在 `RunEvent::ExitRequested`（Cmd+Q）先 `prevent_exit()` 並通知前端，旗標為 true 才放行。

**Tech Stack:** Tauri 2.10（Rust）、React 19、Vitest + RTL、`@tauri-apps/api` window/event。

Spec：`docs/superpowers/specs/2026-09-21-window-close-confirm-design.md`

## 檔案結構

| 檔案 | 動作 | 職責 |
|---|---|---|
| `src-tauri/src/quit.rs` | 新增 | `QuitState`、`should_intercept_exit`、`set_quit_confirmed`、`on_run_event` |
| `src-tauri/src/lib.rs` | 修改 | `pub mod quit`、`.manage(QuitState)`、註冊 command、run 回呼呼叫 `quit::on_run_event` |
| `src-tauri/capabilities/default.json` | 修改 | 加 `core:window:allow-destroy` |
| `src/lib/busyProbe.ts` (+`.test.ts`) | 新增 | `BusyReason`／`BusyProbe`／`BusyTab` 型別與純函式 `collectBusyTabs` |
| `src/ipc/quit.ts` (+`.test.ts`) | 新增 | `setQuitConfirmed`、`onQuitRequested` |
| `src/hooks/useWindowCloseGuard.ts` (+`.test.tsx`) | 新增 | 視窗關閉攔截與確認狀態 |
| `src/lib/i18n.ts` | 修改 | `win_close_*` 字串（zh-TW + en） |
| `src/components/TerminalView.tsx` | 修改 | 抽出 `getBusyReason`，guard 與探針共用 |
| `src/components/CodeAssistantView/index.tsx` | 修改 | 註冊探針 |
| `src/components/LoopStudio/index.tsx` | 修改 | 註冊探針 |
| `src/components/TerminalApp.tsx` | 修改 | 探針表、掛 hook、渲染確認框、傳 props |
| 5 個 `TerminalApp.*.test.tsx` | 修改 | window mock 補 `onCloseRequested`／`destroy` |
| `src/components/TerminalApp.windowCloseGuard.test.tsx` | 新增 | 整合測試 |

`cargo`／`vitest` 指令一律在 repo 根（Rust 在 `src-tauri/`）。本機已有 `src-tauri/binaries/uv-aarch64-apple-darwin`，不需重跑 setup 腳本。

---

### Task 1: Rust — 退出攔截判定與旗標

**Files:**
- Create: `src-tauri/src/quit.rs`
- Modify: `src-tauri/src/lib.rs`（`pub mod` 區、`.manage(...)` 區約 252 行、`generate_handler!` 約 662 行、`run` 回呼約 676 行）

- [ ] **Step 1: 寫失敗的測試（新檔 `quit.rs`，先只放測試與空殼）**

```rust
//! 退出確認：macOS 的 Cmd+Q 不經前端的 `onCloseRequested`，而走
//! `RunEvent::ExitRequested`。這裡先攔下來，交給前端決定是否真的退出。

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, State};

/// 攔下退出請求後通知前端的事件（無酬載）。
pub const QUIT_REQUESTED_EVENT: &str = "app://quit-requested";

/// 前端已確認「可以退出」的旗標。確認流程的最後一步（`destroy()` 視窗）
/// 會讓 runtime 再送一次 `ExitRequested`（最後一個視窗關閉），
/// 那一次必須放行，否則 App 會殘留成沒有視窗的殭屍程序。
#[derive(Default)]
pub struct QuitState {
    confirmed: AtomicBool,
}

/// 這次 `ExitRequested` 要不要攔下。
///
/// - `code` 是 `Some`：程式自己呼叫了 `app.exit(code)`（例如更新後重啟），不是
///   使用者要退出，一律放行。
/// - `confirmed`：前端已確認過，放行。
pub fn should_intercept_exit(code: Option<i32>, confirmed: bool) -> bool {
    let _ = (code, confirmed);
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::should_intercept_exit;

    #[test]
    fn user_initiated_quit_is_intercepted_until_confirmed() {
        assert!(should_intercept_exit(None, false));
    }

    #[test]
    fn confirmed_quit_passes_through() {
        assert!(!should_intercept_exit(None, true));
    }

    #[test]
    fn programmatic_exit_is_never_intercepted() {
        assert!(!should_intercept_exit(Some(0), false));
        assert!(!should_intercept_exit(Some(1), false));
    }
}
```

同時在 `lib.rs` 第 14 行 `pub mod launch;` 旁加上 `pub mod quit;`（否則測試不會被編譯）。

- [ ] **Step 2: 跑測試確認會紅**

Run: `cd src-tauri && cargo test --workspace --no-fail-fast -- quit::`
Expected: 三條 FAIL，訊息含 `not implemented`（若是編譯錯誤先修編譯，不能把編譯失敗當成紅燈）。

- [ ] **Step 3: 實作**

把 `should_intercept_exit` 換成：

```rust
pub fn should_intercept_exit(code: Option<i32>, confirmed: bool) -> bool {
    code.is_none() && !confirmed
}
```

並在同檔加入 command 與事件處理（放在 `mod tests` 之前）：

```rust
/// 前端在「確認可以退出」（或發現沒有任何忙碌分頁）之後、`destroy()` 視窗之前呼叫。
#[tauri::command]
pub fn set_quit_confirmed(state: State<'_, QuitState>) {
    state.confirmed.store(true, Ordering::SeqCst);
}

/// 接在 `App::run` 回呼裡。
pub fn on_run_event(app: &AppHandle, event: &tauri::RunEvent) {
    if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
        let confirmed = app.state::<QuitState>().confirmed.load(Ordering::SeqCst);
        if should_intercept_exit(*code, confirmed) {
            api.prevent_exit();
            if let Err(e) = app.emit(QUIT_REQUESTED_EVENT, ()) {
                log::warn!("emit {QUIT_REQUESTED_EVENT} failed: {e}");
            }
        }
    }
}
```

- [ ] **Step 4: 接進 `lib.rs`**

1. `.manage(launch::LaunchQueue::default())` 下一行加 `.manage(quit::QuitState::default())`。
2. `generate_handler!` 的 `launch::take_launch_requests,` 下一行加 `quit::set_quit_confirmed,`。
3. `run` 回呼中，`launch::on_run_event(app_handle, &event);` 下一行加 `quit::on_run_event(app_handle, &event);`。**不要動**下方 `RunEvent::Exit` 的郵件登出區塊。

- [ ] **Step 5: 跑測試確認轉綠**

Run: `cd src-tauri && cargo test --workspace --no-fail-fast -- quit::`
Expected: `test result: ok. 3 passed`（看每一行 `test result:`，不是只看最後一行）。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/quit.rs src-tauri/src/lib.rs
git commit -m "feat(quit): intercept user-initiated exit until the frontend confirms

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 2: 前端 — 探針型別與 `collectBusyTabs`

**Files:**
- Create: `src/lib/busyProbe.ts`, `src/lib/busyProbe.test.ts`

- [ ] **Step 1: 寫失敗的測試**

```ts
import { describe, it, expect, vi } from "vitest";
import { collectBusyTabs, type BusyProbe } from "./busyProbe";

const titles: Record<string, string> = { a: "Terminal", b: "Loop Studio" };
const titleOf = (id: string) => titles[id];

describe("collectBusyTabs", () => {
  it("全部閒置：回空陣列", () => {
    const probes = new Map<string, BusyProbe>([["a", () => null], ["b", () => null]]);
    expect(collectBusyTabs(probes, titleOf)).toEqual([]);
  });

  it("只列出忙碌的分頁，順序為註冊順序，附標題與原因", () => {
    const probes = new Map<string, BusyProbe>([
      ["a", () => "command"],
      ["b", () => null],
      ["c", () => "loop"],
    ]);
    expect(collectBusyTabs(probes, titleOf)).toEqual([
      { tabId: "a", title: "Terminal", reason: "command" },
      { tabId: "c", title: "c", reason: "loop" }, // 查不到標題 → 退回 tabId
    ]);
  });

  it("探針丟例外：視為閒置並印警告，不能讓整個關視窗流程壞掉", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const probes = new Map<string, BusyProbe>([
      ["a", () => { throw new Error("boom"); }],
      ["b", () => "streaming"],
    ]);
    expect(collectBusyTabs(probes, titleOf)).toEqual([
      { tabId: "b", title: "Loop Studio", reason: "streaming" },
    ]);
    expect(warn).toHaveBeenCalled();
    warn.mockRestore();
  });
});
```

- [ ] **Step 2: 跑測試確認紅**

Run: `npx vitest run src/lib/busyProbe.test.ts`
Expected: FAIL（`Failed to resolve import "./busyProbe"`）。

- [ ] **Step 3: 實作 `src/lib/busyProbe.ts`**

```ts
/** 分頁「正在做什麼」。與 i18n 的 `win_close_reason_*` 一一對應。 */
export type BusyReason = "command" | "agent" | "task" | "loop" | "streaming";

/** 分頁註冊的探針：忙碌回傳原因，閒置回傳 null。只讀狀態，不可彈框、不可有副作用。 */
export type BusyProbe = () => BusyReason | null;

export interface BusyTab {
  tabId: string;
  title: string;
  reason: BusyReason;
}

/**
 * 逐一詢問所有探針，回傳忙碌的分頁（保持註冊順序）。
 * 單一探針丟例外時當作閒置——寧可漏報一個分頁，也不能讓「關視窗」本身失效。
 */
export function collectBusyTabs(
  probes: ReadonlyMap<string, BusyProbe>,
  titleOf: (tabId: string) => string | undefined,
): BusyTab[] {
  const busy: BusyTab[] = [];
  for (const [tabId, probe] of probes) {
    let reason: BusyReason | null;
    try {
      reason = probe();
    } catch (e) {
      console.warn(`busy probe of tab ${tabId} threw; treating it as idle:`, e);
      continue;
    }
    if (reason) busy.push({ tabId, title: titleOf(tabId) ?? tabId, reason });
  }
  return busy;
}
```

- [ ] **Step 4: 跑測試確認綠**

Run: `npx vitest run src/lib/busyProbe.test.ts`
Expected: 3 passed。

- [ ] **Step 5: Commit**

```bash
git add src/lib/busyProbe.ts src/lib/busyProbe.test.ts
git commit -m "feat(quit): busy probe types and collector

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 3: 前端 — IPC 封裝

**Files:**
- Create: `src/ipc/quit.ts`, `src/ipc/quit.test.ts`

- [ ] **Step 1: 寫失敗的測試**

```ts
import { describe, it, expect, vi, beforeEach } from "vitest";

const invokeMock = vi.fn();
const listenMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: (...a: unknown[]) => listenMock(...a) }));

import { setQuitConfirmed, onQuitRequested, QUIT_REQUESTED_EVENT } from "./quit";

beforeEach(() => {
  invokeMock.mockReset();
  listenMock.mockReset();
});

describe("quit ipc", () => {
  it("setQuitConfirmed 呼叫 set_quit_confirmed", async () => {
    invokeMock.mockResolvedValue(undefined);
    await setQuitConfirmed();
    expect(invokeMock).toHaveBeenCalledWith("set_quit_confirmed");
  });

  it("onQuitRequested 訂閱 Rust 端的事件名並轉呼叫 callback", async () => {
    const unlisten = vi.fn();
    listenMock.mockImplementation((_name: string, handler: () => void) => {
      handler();
      return Promise.resolve(unlisten);
    });
    const cb = vi.fn();
    const got = await onQuitRequested(cb);
    expect(listenMock.mock.calls[0][0]).toBe("app://quit-requested");
    expect(QUIT_REQUESTED_EVENT).toBe("app://quit-requested");
    expect(cb).toHaveBeenCalledTimes(1);
    expect(got).toBe(unlisten);
  });
});
```

- [ ] **Step 2: 跑測試確認紅**

Run: `npx vitest run src/ipc/quit.test.ts`
Expected: FAIL（找不到 `./quit`）。

- [ ] **Step 3: 實作 `src/ipc/quit.ts`**

```ts
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** 與 Rust `quit::QUIT_REQUESTED_EVENT` 相同。 */
export const QUIT_REQUESTED_EVENT = "app://quit-requested";

/**
 * 通知後端「可以退出了」。必須在 `destroy()` 視窗之前呼叫：最後一個視窗關閉
 * 會再觸發一次 ExitRequested，後端靠這個旗標放行。
 */
export function setQuitConfirmed(): Promise<void> {
  return invoke<void>("set_quit_confirmed");
}

/** 後端攔下使用者的退出請求（macOS Cmd+Q）時觸發，事件不帶資料。 */
export function onQuitRequested(cb: () => void): Promise<UnlistenFn> {
  return listen(QUIT_REQUESTED_EVENT, () => cb());
}
```

- [ ] **Step 4: 跑測試確認綠**

Run: `npx vitest run src/ipc/quit.test.ts`
Expected: 2 passed。

- [ ] **Step 5: Commit**

```bash
git add src/ipc/quit.ts src/ipc/quit.test.ts
git commit -m "feat(quit): ipc wrappers for quit confirmation

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 4: i18n 字串

**Files:**
- Modify: `src/lib/i18n.ts`（zh-TW：`term_close_discard` 後約 222 行；en：`term_close_discard` 後約 1818 行）

確認鈕與取消鈕沿用 `term_close_discard`（「關閉並中止」）與 `term_close_cancel`（「取消（繼續執行）」），不新增，避免重複。

- [ ] **Step 1: zh-TW 加 7 個 key**（插在 `term_close_discard: "關閉並中止",` 之後）

```ts
    win_close_title: "還有工作正在進行",
    win_close_body: "關閉 AITerm 會中止以下工作：",
    win_close_reason_command: "指令執行中",
    win_close_reason_agent: "Agent 任務進行中",
    win_close_reason_task: "工作看板任務進行中",
    win_close_reason_loop: "Loop 正在執行",
    win_close_reason_streaming: "AI 正在回應",
```

- [ ] **Step 2: en 加同樣 7 個 key**（插在英文 `term_close_discard: "Close and abort",` 之後）

```ts
    win_close_title: "Work is still in progress",
    win_close_body: "Closing AITerm will abort the following:",
    win_close_reason_command: "Command running",
    win_close_reason_agent: "Agent task in progress",
    win_close_reason_task: "Task Board task in progress",
    win_close_reason_loop: "Loop running",
    win_close_reason_streaming: "AI responding",
```

- [ ] **Step 3: 人工核對數量（en 缺 key 會靜默 fallback 成中文，`tsc` 抓不到）**

Run: `grep -c "win_close_" src/lib/i18n.ts`
Expected: `14`（7 × 2）。

- [ ] **Step 4: 型別檢查與 i18n 相關測試**

Run: `npx tsc -b && npx vitest run src/lib/i18n`
Expected: tsc 無輸出；vitest 通過（若無符合檔名的測試會顯示 no test files，改跑 `npx vitest run -t i18n`）。

- [ ] **Step 5: Commit**

```bash
git add src/lib/i18n.ts
git commit -m "feat(quit): i18n strings for the window-close confirmation

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 5: `useWindowCloseGuard` hook

**Files:**
- Create: `src/hooks/useWindowCloseGuard.ts`, `src/hooks/useWindowCloseGuard.test.tsx`

- [ ] **Step 1: 寫失敗的測試**

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import type { BusyTab } from "../lib/busyProbe";

// 捕捉 hook 註冊的兩個入口，測試手動觸發。
let closeCb: ((e: { preventDefault: () => void }) => void) | undefined;
let quitCb: (() => void) | undefined;
const destroy = vi.fn(() => Promise.resolve());
const unlistenClose = vi.fn();
const unlistenQuit = vi.fn();
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    onCloseRequested: (cb: typeof closeCb) => { closeCb = cb; return Promise.resolve(unlistenClose); },
    destroy: () => destroy(),
  }),
}));
const setQuitConfirmed = vi.fn(() => Promise.resolve());
vi.mock("../ipc/quit", () => ({
  QUIT_REQUESTED_EVENT: "app://quit-requested",
  setQuitConfirmed: () => setQuitConfirmed(),
  onQuitRequested: (cb: () => void) => { quitCb = cb; return Promise.resolve(unlistenQuit); },
}));

import { useWindowCloseGuard } from "./useWindowCloseGuard";

const busyTab: BusyTab = { tabId: "t1", title: "Terminal", reason: "command" };

beforeEach(() => {
  closeCb = undefined;
  quitCb = undefined;
  destroy.mockClear();
  setQuitConfirmed.mockClear();
  unlistenClose.mockClear();
  unlistenQuit.mockClear();
});

async function mount(getBusyTabs: () => BusyTab[]) {
  const hook = renderHook(() => useWindowCloseGuard(getBusyTabs));
  await act(async () => {}); // 讓 listen 的 promise resolve
  return hook;
}

function fireClose() {
  const preventDefault = vi.fn();
  return { preventDefault, run: () => act(async () => { closeCb!({ preventDefault }); }) };
}

describe("useWindowCloseGuard", () => {
  it("全部閒置：攔下原生關閉、先設旗標再 destroy，不出現確認狀態", async () => {
    const { result } = await mount(() => []);
    const ev = fireClose();
    await ev.run();

    expect(ev.preventDefault).toHaveBeenCalled(); // 由我們自己 destroy，不讓預設流程跑第二次
    expect(setQuitConfirmed).toHaveBeenCalledTimes(1);
    expect(destroy).toHaveBeenCalledTimes(1);
    expect(setQuitConfirmed.mock.invocationCallOrder[0]).toBeLessThan(destroy.mock.invocationCallOrder[0]);
    expect(result.current.pending).toBeNull();
  });

  it("有忙碌分頁：不 destroy，pending 帶出清單", async () => {
    const { result } = await mount(() => [busyTab]);
    await fireClose().run();

    expect(destroy).not.toHaveBeenCalled();
    expect(setQuitConfirmed).not.toHaveBeenCalled();
    expect(result.current.pending).toEqual([busyTab]);
  });

  it("確認：先 set_quit_confirmed 再 destroy", async () => {
    const { result } = await mount(() => [busyTab]);
    await fireClose().run();
    await act(async () => { result.current.confirm(); });

    expect(setQuitConfirmed).toHaveBeenCalledTimes(1);
    expect(destroy).toHaveBeenCalledTimes(1);
    expect(setQuitConfirmed.mock.invocationCallOrder[0]).toBeLessThan(destroy.mock.invocationCallOrder[0]);
  });

  it("取消：清掉 pending，且之後可以再次觸發關閉", async () => {
    const { result } = await mount(() => [busyTab]);
    await fireClose().run();
    act(() => result.current.cancel());
    expect(result.current.pending).toBeNull();

    await fireClose().run();
    expect(result.current.pending).toEqual([busyTab]);
    expect(destroy).not.toHaveBeenCalled();
  });

  it("重入：確認框已顯示時再次觸發，不覆蓋既有清單", async () => {
    let tabs: BusyTab[] = [busyTab];
    const { result } = await mount(() => tabs);
    await fireClose().run();
    const first = result.current.pending;

    tabs = [{ tabId: "t2", title: "Loop", reason: "loop" }];
    await fireClose().run();
    expect(result.current.pending).toBe(first);
  });

  it("Cmd+Q 入口（後端事件）走同一條路：忙碌時出現確認狀態", async () => {
    const { result } = await mount(() => [busyTab]);
    await act(async () => { quitCb!(); });
    expect(result.current.pending).toEqual([busyTab]);
    expect(destroy).not.toHaveBeenCalled();
  });

  it("Cmd+Q 入口：閒置時也要 set_quit_confirmed + destroy（後端已經 prevent_exit 了）", async () => {
    await mount(() => []);
    await act(async () => { quitCb!(); });
    expect(setQuitConfirmed).toHaveBeenCalledTimes(1);
    expect(destroy).toHaveBeenCalledTimes(1);
  });

  // 釘住最容易靜默失效的點：listener 只註冊一次，之後忙碌狀態才變化，仍要看得到。
  it("讀最新的 getBusyTabs（不可閉包捕捉註冊當下的版本）", async () => {
    const hook = renderHook(({ fn }) => useWindowCloseGuard(fn), { initialProps: { fn: (): BusyTab[] => [] } });
    await act(async () => {});
    hook.rerender({ fn: () => [busyTab] });

    await fireClose().run();
    expect(hook.result.current.pending).toEqual([busyTab]);
    expect(destroy).not.toHaveBeenCalled();
  });

  it("unmount 解除兩個監聽", async () => {
    const { unmount } = await mount(() => []);
    unmount();
    await act(async () => {});
    expect(unlistenClose).toHaveBeenCalledTimes(1);
    expect(unlistenQuit).toHaveBeenCalledTimes(1);
  });
});
```

- [ ] **Step 2: 跑測試確認紅**

Run: `npx vitest run src/hooks/useWindowCloseGuard.test.tsx`
Expected: FAIL（找不到 `./useWindowCloseGuard`）。

- [ ] **Step 3: 實作 `src/hooks/useWindowCloseGuard.ts`**

```ts
import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { onQuitRequested, setQuitConfirmed, QUIT_REQUESTED_EVENT } from "../ipc/quit";
import { unlistenOnCleanup } from "../lib/eventSubscription";
import type { BusyTab } from "../lib/busyProbe";

/**
 * 攔截整個視窗的關閉。兩個入口——原生關閉（✕／Alt+F4，`onCloseRequested`）與
 * 後端攔下的 Cmd+Q（`app://quit-requested`）——共用同一個 `attempt()`：
 * 沒有忙碌分頁就直接退出，否則交給呼叫端顯示確認框（`pending`）。
 *
 * 原生關閉一律 `preventDefault()` 再自己 `destroy()`：Tauri 預設在 handler 結束後
 * 也會 destroy，兩條路徑各關一次沒有意義，統一由 `quit()` 負責。
 */
export function useWindowCloseGuard(getBusyTabs: () => BusyTab[]) {
  const [pending, setPending] = useState<BusyTab[] | null>(null);
  const pendingRef = useRef<BusyTab[] | null>(null);

  // listener 只註冊一次；忙碌狀態要靠 ref 讀最新的，閉包捕捉會靜默放行。
  const getBusyTabsRef = useRef(getBusyTabs);
  useEffect(() => {
    getBusyTabsRef.current = getBusyTabs;
  });

  const quit = useCallback(async () => {
    // 必須在 destroy 之前：最後一個視窗關閉會再觸發 ExitRequested，後端靠旗標放行。
    try {
      await setQuitConfirmed();
    } catch (e) {
      console.error("set_quit_confirmed 失敗:", e);
    }
    try {
      await getCurrentWindow().destroy();
    } catch (e) {
      console.error("destroy 視窗失敗:", e);
    }
  }, []);

  const attempt = useCallback(async () => {
    if (pendingRef.current) return; // 確認框已顯示，連按 ✕ 不重複處理
    const busy = getBusyTabsRef.current();
    if (busy.length === 0) {
      await quit();
      return;
    }
    pendingRef.current = busy;
    setPending(busy);
  }, [quit]);

  useEffect(() => {
    const unCloseRequested = unlistenOnCleanup(
      getCurrentWindow().onCloseRequested((event) => {
        event.preventDefault();
        void attempt();
      }),
      "tauri://close-requested",
    );
    const unQuitRequested = unlistenOnCleanup(
      onQuitRequested(() => { void attempt(); }),
      QUIT_REQUESTED_EVENT,
    );
    return () => {
      unCloseRequested();
      unQuitRequested();
    };
  }, [attempt]);

  const confirm = useCallback(() => {
    // 不清 pending：視窗即將消失，清掉只會讓連按 ✕ 又跳出第二個框。
    void quit();
  }, [quit]);

  const cancel = useCallback(() => {
    pendingRef.current = null;
    setPending(null);
  }, []);

  return { pending, confirm, cancel };
}
```

- [ ] **Step 4: 跑測試確認綠**

Run: `npx vitest run src/hooks/useWindowCloseGuard.test.tsx`
Expected: 9 passed。

- [ ] **Step 5: 變異驗證（證明「讀最新」那題真的會紅）**

暫時把 `attempt` 內的 `getBusyTabsRef.current()` 改成 `getBusyTabs()`，並把 `attempt` 的 deps 保持 `[quit]`。

Run: `npx vitest run src/hooks/useWindowCloseGuard.test.tsx -t "讀最新"`
Expected: FAIL。驗完**還原**，再跑一次全檔確認 9 passed。

- [ ] **Step 6: Commit**

```bash
git add src/hooks/useWindowCloseGuard.ts src/hooks/useWindowCloseGuard.test.tsx
git commit -m "feat(quit): useWindowCloseGuard hook shared by ✕ and Cmd+Q

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 6: 三個消費端註冊探針

**Files:**
- Modify: `src/components/TerminalView.tsx`（props 約 150 行、guard effect 約 512–530 行）
- Modify: `src/components/CodeAssistantView/index.tsx`（props 約 30 行、guard effect 約 83–95 行）
- Modify: `src/components/LoopStudio/index.tsx`（props 約 65 行、guard effect 約 180–200 行）
- Test: `src/components/TerminalView.closeGuard.test.tsx`、`src/components/CodeAssistantView/closeGuard.test.tsx`、`src/components/LoopStudio/closeGuard.test.tsx`

三個元件的 props 各加：

```ts
registerBusyProbe?: (tabId: string, probe: BusyProbe) => void;
unregisterBusyProbe?: (tabId: string) => void;
```
並 `import type { BusyProbe } from "../lib/busyProbe";`（子目錄元件用 `"../../lib/busyProbe"`）。

#### 6a. CodeAssistantView

- [ ] **Step 1: 在 `closeGuard.test.tsx` 末尾新增失敗的測試**

```tsx
function mountAndCaptureProbe() {
  let probe: (() => string | null) | undefined;
  const register = (_id: string, p: () => string | null) => { probe = p; };
  const unregister = vi.fn();
  const renderUi = () => (
    <LocaleProvider>
      <CodeAssistantView
        isActive
        tabId="tab-1"
        registerBusyProbe={register}
        unregisterBusyProbe={unregister}
      />
    </LocaleProvider>
  );
  const view = render(renderUi());
  if (!probe) throw new Error("CodeAssistantView 沒有註冊 busy probe");
  return { probe, view, unregister, renderUi };
}

describe("Agent 分頁 busy probe", () => {
  it("閒置：回 null；有對話但沒在串流也回 null（那是內容遺失，不是工作進行中）", () => {
    fakeAssistant.messages = [{ role: "user", content: "hi" }];
    const { probe } = mountAndCaptureProbe();
    expect(probe()).toBeNull();
  });

  it("串流中：回 streaming；註冊之後才開始串流也看得到", () => {
    const { probe, view, renderUi } = mountAndCaptureProbe();
    expect(probe()).toBeNull();
    fakeAssistant.isStreaming = true;
    view.rerender(renderUi());
    expect(probe()).toBe("streaming");
  });

  it("unmount 時解除註冊", () => {
    const { view, unregister } = mountAndCaptureProbe();
    view.unmount();
    expect(unregister).toHaveBeenCalledWith("tab-1");
  });
});
```

- [ ] **Step 2: 確認紅**

Run: `npx vitest run src/components/CodeAssistantView/closeGuard.test.tsx -t "busy probe"`
Expected: FAIL（`沒有註冊 busy probe`）。

- [ ] **Step 3: 實作**（緊接在既有 guard effect 之後）

```ts
  // 視窗關閉用的忙碌探針：只回報「正在做事」（串流中），不含「有對話」——
  // 後者是分頁 ✕ 的內容遺失 guard 的職責。同樣讀 ref，不可閉包捕捉。
  useEffect(() => {
    if (!tabId || !registerBusyProbe) return;
    registerBusyProbe(tabId, () => (isStreamingRef.current ? "streaming" : null));
    return () => { unregisterBusyProbe?.(tabId); };
  }, [tabId, registerBusyProbe, unregisterBusyProbe]);
```

並在解構 props 處加上 `registerBusyProbe, unregisterBusyProbe`。

- [ ] **Step 4: 確認綠**

Run: `npx vitest run src/components/CodeAssistantView/closeGuard.test.tsx`
Expected: 全部通過（舊 7 + 新 3）。

#### 6b. LoopStudio

- [ ] **Step 5: 在 `LoopStudio/closeGuard.test.tsx` 末尾新增失敗的測試**

```tsx
function mountAndCaptureProbe() {
  let probe: (() => string | null) | undefined;
  const unregister = vi.fn();
  const ui = () => (
    <LocaleProvider>
      <LoopStudioView
        tabId="tab-1"
        registerBusyProbe={(_id, p) => { probe = p; }}
        unregisterBusyProbe={unregister}
      />
    </LocaleProvider>
  );
  const view = render(ui());
  if (!probe) throw new Error("LoopStudio 沒有註冊 busy probe");
  return { probe, view, unregister, ui };
}

describe("LoopStudio busy probe", () => {
  it("未執行：回 null", () => {
    expect(mountAndCaptureProbe().probe()).toBeNull();
  });

  it("執行中：回 loop", () => {
    fakeLoop.isRunning = true;
    expect(mountAndCaptureProbe().probe()).toBe("loop");
  });

  it("註冊之後才開始執行，探針仍看得到（讀 ref，不可閉包捕捉）", () => {
    const { probe, view, ui } = mountAndCaptureProbe();
    expect(probe()).toBeNull();
    fakeLoop.isRunning = true;
    view.rerender(ui());
    expect(probe()).toBe("loop");
  });

  it("unmount 時解除註冊", () => {
    const { view, unregister } = mountAndCaptureProbe();
    view.unmount();
    expect(unregister).toHaveBeenCalledWith("tab-1");
  });
});
```

- [ ] **Step 6: 確認紅**

Run: `npx vitest run src/components/LoopStudio/closeGuard.test.tsx -t "busy probe"`
Expected: FAIL。

- [ ] **Step 7: 實作**

在 `loop` 取得之後（guard effect 之前）加 ref 與 effect：

```ts
  const loopRunningRef = useRef(false);
  loopRunningRef.current = loop.isRunning;

  // 視窗關閉用的忙碌探針。分頁 ✕ 的 guard 因 deps 含 loop.isRunning 會反覆重註冊，
  // 探針改讀 ref，註冊一次即可。
  useEffect(() => {
    if (!tabId || !registerBusyProbe) return;
    registerBusyProbe(tabId, () => (loopRunningRef.current ? "loop" : null));
    return () => { unregisterBusyProbe?.(tabId); };
  }, [tabId, registerBusyProbe, unregisterBusyProbe]);
```

props 加 `registerBusyProbe, unregisterBusyProbe`。

- [ ] **Step 8: 確認綠**

Run: `npx vitest run src/components/LoopStudio/closeGuard.test.tsx`
Expected: 全部通過。

#### 6c. TerminalView（判定只能有一份）

- [ ] **Step 9: 在 `TerminalView.closeGuard.test.tsx` 新增失敗的測試**

`mountAndCaptureGuard` 旁新增：

```tsx
import { setRunningTaskTabs } from "../lib/runningTaskTabRegistry";

function mountAndCaptureProbe() {
  let probe: (() => string | null) | undefined;
  const register = (_id: string, p: () => string | null) => { probe = p; };
  const unregister = vi.fn();
  const renderUi = () => (
    <LocaleProvider>
      <MemoryRouter>
        <TerminalView tabId="tab-1" registerBusyProbe={register} unregisterBusyProbe={unregister} />
      </MemoryRouter>
    </LocaleProvider>
  );
  const view = render(renderUi());
  if (!probe) throw new Error("TerminalView 沒有註冊 busy probe");
  return { probe, view, unregister, renderUi };
}

describe("TerminalView busy probe", () => {
  it("閒置（含 agentMission 為 null）：回 null", () => {
    expect(mountAndCaptureProbe().probe()).toBeNull();
  });

  it("有指令執行中：回 command", () => {
    fakeBlocksState.value = [{ id: "b1", command: "npm test", status: "running", startTime: 0, rawOutput: "" }];
    expect(mountAndCaptureProbe().probe()).toBe("command");
  });

  it("Agent 任務進行中：回 agent", () => {
    fakeMissionState.value = { active: true, goal: "g", stepCount: 1, maxSteps: 5, tokensUsed: 0, history: [] };
    expect(mountAndCaptureProbe().probe()).toBe("agent");
  });

  it("註冊之後才開始跑指令，探針仍看得到（不可閉包捕捉）", () => {
    const { probe, view, renderUi } = mountAndCaptureProbe();
    expect(probe()).toBeNull();
    fakeBlocksState.value = [{ id: "b1", command: "sleep 9", status: "running", startTime: 0, rawOutput: "" }];
    view.rerender(renderUi());
    expect(probe()).toBe("command");
  });

  it("unmount 時解除註冊", () => {
    const { view, unregister } = mountAndCaptureProbe();
    view.unmount();
    expect(unregister).toHaveBeenCalledWith("tab-1");
  });
});
```

工作看板任務那一題需要知道 `sessionIdRef` 怎麼被設定——先讀 `TerminalView.tsx` 中 `sessionIdRef.current =` 的賦值處與既有測試如何讓它有值；若既有 closeGuard 測試沒有覆蓋 `isRunningTaskTab` 分支，這一題以 `vi.mock("../lib/runningTaskTabRegistry", ...)` 讓 `isRunningTaskTab` 回 true，斷言探針回 `task`。**不得省略此題**——它是三訊號中唯一不靠 mock 的 hook 就能造成的 shell 外訊號。

- [ ] **Step 10: 確認紅**

Run: `npx vitest run src/components/TerminalView.closeGuard.test.tsx -t "busy probe"`
Expected: FAIL。

- [ ] **Step 11: 實作**

在 `isBusyRef`／`missionActiveRef` 之後、既有 guard effect 之前，抽出**唯一一份**判定，並讓既有 guard 改用它：

```ts
  // 「這個終端機分頁現在在忙什麼」——分頁 ✕ 的 guard 與視窗關閉的探針共用這一份，
  // 判定不能有兩套。優先序與確認框標題一致：mission → 工作看板任務 → 一般指令。
  // 全部讀 ref，所以這個函式是穩定的、可以安全地只註冊一次。
  const getBusyReason = useCallback((): BusyReason | null => {
    if (missionActiveRef.current) return "agent";
    if (isRunningTaskTab(sessionIdRef.current)) return "task";
    if (isBusyRef.current) return "command";
    return null;
  }, []);
```

既有 guard 中的條件

```ts
if (!isBusyRef.current && !missionActiveRef.current && !isRunningTaskTab(sessionIdRef.current)) {
```
改為
```ts
if (getBusyReason() === null) {
```
（保留原有那段長註解，它解釋的是為什麼三個訊號缺一不可。）guard effect 的 deps 加上 `getBusyReason`（穩定引用，不影響重註冊）。

其後新增：

```ts
  useEffect(() => {
    if (!tabId || !registerBusyProbe) return;
    registerBusyProbe(tabId, getBusyReason);
    return () => { unregisterBusyProbe?.(tabId); };
  }, [tabId, registerBusyProbe, unregisterBusyProbe, getBusyReason]);
```

props 加 `registerBusyProbe, unregisterBusyProbe`；`import type { BusyReason, BusyProbe } from "../lib/busyProbe";`。

- [ ] **Step 12: 確認綠且既有 guard 行為未變**

Run: `npx vitest run src/components/TerminalView.closeGuard.test.tsx`
Expected: 全部通過（舊有 guard 測試一條都不能紅）。

- [ ] **Step 13: 變異驗證探針讀 ref**

暫時把探針註冊改成 `registerBusyProbe(tabId, () => (blocks[blocks.length - 1]?.status === "running" ? "command" : null))`（閉包捕捉 `blocks`）。
Run: `npx vitest run src/components/TerminalView.closeGuard.test.tsx -t "註冊之後才開始"`
Expected: FAIL。驗完還原。

- [ ] **Step 14: 型別檢查並 Commit**

Run: `npx tsc -b`
Expected: 無輸出。

```bash
git add src/components/TerminalView.tsx src/components/TerminalView.closeGuard.test.tsx \
  src/components/CodeAssistantView src/components/LoopStudio
git commit -m "feat(quit): tabs report what they are busy with via a shared busy probe

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 7: `TerminalApp` 接線、權限、既有測試 mock

**Files:**
- Modify: `src/components/TerminalApp.tsx`
- Modify: `src-tauri/capabilities/default.json`
- Modify: `src/components/TerminalApp.{taskBoard,launchRequest,routeHintCloseGuard,remoteReconnect,autoCloseSkipGuard}.test.tsx`
- Create: `src/components/TerminalApp.windowCloseGuard.test.tsx`

- [ ] **Step 1: 既有 5 個測試的 `@tauri-apps/api/window` mock 補兩個方法**

新的 hook 會在 `TerminalApp` 掛載時呼叫 `getCurrentWindow().onCloseRequested(...)`。這 5 個檔的 mock 沒有它，不補整批會因 `onCloseRequested is not a function` 全紅。在各檔 `getCurrentWindow: () => ({ ... })` 物件內加：

```ts
    onCloseRequested: () => Promise.resolve(() => {}),
    destroy: () => Promise.resolve(),
```

- [ ] **Step 2: 先跑一次確認這 5 檔目前仍綠（尚未改 TerminalApp）**

Run: `npx vitest run src/components/TerminalApp.`
Expected: 通過。

- [ ] **Step 3: 寫整合測試（新檔 `TerminalApp.windowCloseGuard.test.tsx`）**

沿用 `TerminalApp.routeHintCloseGuard.test.tsx` 的 mock 骨架（invoke/listen/homeDir/window/notification、`../ipc/provider`、`useOrchestratorLoop` fake、jsdom polyfill、`useTerminalBlocks`／`useAgentMission` mock、`SESSION_TABS_KEY` 播種 Loop Studio 分頁；不需要 HomeView stub）。與該檔的差異：

```tsx
let closeCb: ((e: { preventDefault: () => void }) => Promise<void> | void) | undefined;
const destroy = vi.fn(() => Promise.resolve());
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isFocused: () => Promise.resolve(true),
    onFocusChanged: () => Promise.resolve(() => {}),
    isMaximized: () => Promise.resolve(false),
    onResized: () => Promise.resolve(() => {}),
    maximize: () => Promise.resolve(),
    unmaximize: () => Promise.resolve(),
    minimize: () => Promise.resolve(),
    close: () => Promise.resolve(),
    startDragging: () => Promise.resolve(),
    onCloseRequested: (cb: typeof closeCb) => { closeCb = cb; return Promise.resolve(() => {}); },
    destroy: () => destroy(),
  }),
}));
// 該檔的 invoke 永遠不 resolve，會卡住 setQuitConfirmed；改直接 mock 這層。
const setQuitConfirmed = vi.fn(() => Promise.resolve());
vi.mock("../ipc/quit", () => ({
  QUIT_REQUESTED_EVENT: "app://quit-requested",
  setQuitConfirmed: () => setQuitConfirmed(),
  onQuitRequested: () => Promise.resolve(() => {}),
}));
```

`beforeEach` 重設 `fakeLoop.isRunning=false`、`destroy`／`setQuitConfirmed` 的 mock、`closeCb = undefined`、`localStorage` 播種一個 loop-studio 分頁。測試：

```tsx
async function fireWindowClose() {
  await waitFor(() => expect(closeCb).toBeDefined());
  const preventDefault = vi.fn();
  await act(async () => { await closeCb!({ preventDefault }); });
  return preventDefault;
}

describe("TerminalApp: closing the window while work is running", () => {
  it("全部閒置：不出現確認框，直接 set_quit_confirmed + destroy", async () => {
    renderApp();
    await fireWindowClose();
    expect(screen.queryByRole("heading", { name: "還有工作正在進行" })).not.toBeInTheDocument();
    expect(setQuitConfirmed).toHaveBeenCalledTimes(1);
    expect(destroy).toHaveBeenCalledTimes(1);
  });

  it("Loop 執行中：出現一個確認框，列出分頁名稱與原因，且不 destroy", async () => {
    fakeLoop.isRunning = true;
    renderApp();
    await fireWindowClose();

    expect(screen.getByRole("heading", { name: "還有工作正在進行" })).toBeInTheDocument();
    expect(screen.getByText(/Loop Studio/)).toBeInTheDocument();
    expect(screen.getByText(/Loop 正在執行/)).toBeInTheDocument();
    expect(destroy).not.toHaveBeenCalled();
  });

  it("按「取消（繼續執行）」：確認框消失、不 destroy", async () => {
    fakeLoop.isRunning = true;
    renderApp();
    await fireWindowClose();
    await userEvent.click(screen.getByRole("button", { name: "取消（繼續執行）" }));
    expect(screen.queryByRole("heading", { name: "還有工作正在進行" })).not.toBeInTheDocument();
    expect(destroy).not.toHaveBeenCalled();
  });

  it("按「關閉並中止」：set_quit_confirmed 再 destroy", async () => {
    fakeLoop.isRunning = true;
    renderApp();
    await fireWindowClose();
    await userEvent.click(screen.getByRole("button", { name: "關閉並中止" }));
    await waitFor(() => expect(destroy).toHaveBeenCalledTimes(1));
    expect(setQuitConfirmed).toHaveBeenCalledTimes(1);
  });

  it("確認框覆蓋整個視窗（掛在最外層容器，不是某個分頁內）", async () => {
    fakeLoop.isRunning = true;
    const { container } = renderApp();
    await fireWindowClose();
    const overlay = document.querySelector(".aiterm-close-overlay");
    expect(overlay).not.toBeNull();
    // 最外層容器的直接子節點，而非嵌在分頁內容裡
    expect(overlay!.parentElement).toBe(container.firstElementChild!.firstElementChild);
  });
});
```

（若最後一題的父層斷言與實際 DOM 不符，以「`overlay.parentElement` 是 `TitleBar` 的兄弟節點」改寫，目的是證明它掛在根容器；先實際印出 DOM 再定斷言，不要猜。）

- [ ] **Step 4: 確認紅**

Run: `npx vitest run src/components/TerminalApp.windowCloseGuard.test.tsx`
Expected: FAIL（`closeCb` 永遠 undefined，`waitFor` 逾時——因為還沒接 hook）。

- [ ] **Step 5: 實作 `TerminalApp.tsx`**

1. import：
```ts
import { CloseConfirmDialog } from "./CloseConfirmDialog";
import { collectBusyTabs, type BusyProbe } from "../lib/busyProbe";
import { useWindowCloseGuard } from "../hooks/useWindowCloseGuard";
```
（`CloseConfirmDialog` 先 grep 確認 `TerminalApp.tsx` 尚未 import。）

2. 緊接 `unregisterCloseGuard` 之後：
```ts
  // 視窗關閉用的忙碌探針表，與 closeGuardsRef 平行。
  const busyProbesRef = useRef<Map<string, BusyProbe>>(new Map());
  const registerBusyProbe = useCallback((tabId: string, probe: BusyProbe) => {
    busyProbesRef.current.set(tabId, probe);
  }, []);
  const unregisterBusyProbe = useCallback((tabId: string) => {
    busyProbesRef.current.delete(tabId);
  }, []);
  const getBusyTabs = useCallback(
    () => collectBusyTabs(busyProbesRef.current, (id) => tabsRef.current.find((tb) => tb.id === id)?.title),
    [],
  );
  const windowClose = useWindowCloseGuard(getBusyTabs);
```
放在 `tabsRef` 宣告（約 106 行）之後即可，`t` 在 65 行已可用。

3. 三處消費端（`LoopStudioView`、`CodeAssistantView`、`TerminalView`，約 829／836／910 行）各加：
```tsx
registerBusyProbe={registerBusyProbe}
unregisterBusyProbe={unregisterBusyProbe}
```

4. 在根容器內、`<TitleBar .../>` 之後渲染確認框（根容器是 `position: relative`，`.aiterm-close-overlay` 是 `position:absolute; inset:0`，正好覆蓋整個視窗含標題列）：
```tsx
      {windowClose.pending && (
        <CloseConfirmDialog
          title={t.win_close_title}
          body={
            <>
              <p style={{ margin: "0 0 8px" }}>{t.win_close_body}</p>
              <ul style={{ margin: 0, paddingLeft: 18 }}>
                {windowClose.pending.map((b) => (
                  <li key={b.tabId}>
                    {b.title} — {t[`win_close_reason_${b.reason}` as const]}
                  </li>
                ))}
              </ul>
            </>
          }
          confirmLabel={t.term_close_discard}
          cancelLabel={t.term_close_cancel}
          onConfirm={windowClose.confirm}
          onCancel={windowClose.cancel}
        />
      )}
```

5. `src-tauri/capabilities/default.json`：在 `"core:window:allow-close",` 後加 `"core:window:allow-destroy",`。

- [ ] **Step 6: 確認綠**

Run: `npx vitest run src/components/TerminalApp.windowCloseGuard.test.tsx`
Expected: 5 passed。

- [ ] **Step 7: 型別、Lint、全部前端測試**

Run: `npx tsc -b && npm run lint && npm run test`
Expected: tsc 無輸出；lint 無新錯誤；vitest 全綠（有既知 flaky `MailView`，只在記憶體吃緊時紅，單獨重跑確認即可，見 memory `project-mailview-flaky-unreproduced`）。

- [ ] **Step 8: 變異驗證整合層的「忙碌才擋」**

暫時把 `useWindowCloseGuard` 的 `attempt` 改成永遠 `await quit()`（無視忙碌）。
Run: `npx vitest run src/components/TerminalApp.windowCloseGuard.test.tsx`
Expected: 「Loop 執行中」等題 FAIL。驗完還原並確認 5 passed。

- [ ] **Step 9: Commit**

```bash
git add src/components/TerminalApp.tsx src/components/TerminalApp.*.test.tsx src-tauri/capabilities/default.json
git commit -m "feat(quit): confirm before closing the window while work is running

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 8: 整體驗證與真機驗收

**Files:** 無新增；必要時回頭修 spec 的假設。

- [ ] **Step 1: 完整 Rust 測試**

Run: `cd src-tauri && cargo test --workspace --no-fail-fast 2>&1 | grep -E "test result:|FAILED|error"`
Expected: 每一行 `test result:` 都是 `ok`。`aiterm-core` 的 pty 測試若因 openpty 競爭失敗屬環境問題（見 CLAUDE.md），需明確回報而不是略過；`app` 那一行必須真的出現。

- [ ] **Step 2: 完整前端驗證**

Run: `npx tsc -b && npm run lint && npm run test`
Expected: 同 Task 7 Step 7。

- [ ] **Step 3: 真機驗收（依 memory `reference-mac-isolated-release-verification`，不碰使用者正在跑的那份 AITerm）**

用不同 identifier、獨立 `CARGO_TARGET_DIR`、隔離 HOME 建置並啟動測試版（release 要 `CARGO_PROFILE_RELEASE_STRIP=none`）。以 osascript（memory `reference-mac-ui-automation`）分別在下列狀態操作並截圖／查證：

| 狀態 | 操作 | 預期 |
|---|---|---|
| 閒置 | 點視窗 ✕ | 直接關閉，程序結束 |
| 閒置 | Cmd+Q | 直接結束，程序結束 |
| 終端機跑 `sleep 60` | 點 ✕ | 出現確認框，列出 Terminal — 指令執行中；取消後 `sleep` 仍在 |
| 終端機跑 `sleep 60` | Cmd+Q | 同上 |
| 確認框上按「關閉並中止」 | — | 視窗與程序都結束（不留沒有視窗的殭屍程序：`pgrep` 確認） |

同時確認 spec 第 3 節四項假設，並把實測結果寫回 spec：
1. Cmd+Q 是否觸發 `ExitRequested` 且 `code == None`；
2. 最後視窗關閉是否再送一次 `ExitRequested`；
3. `set_quit_confirmed` → `destroy()` 後 `RunEvent::Exit`（郵件登出）是否仍執行；
4. Windows/Linux 無實機，於 spec 註明未驗證，不宣稱通過。

螢幕鎖定時無法截圖，改用行程與 cwd 佐證，且**不可把「查不到」記成已驗證**。完事：`kill` 自己啟動的 PID、`lsregister -u` 測試 .app、刪測試 target。

- [ ] **Step 4: 若假設不成立**

回到 spec 第 3 節與 `quit.rs` 修正，補上對應測試後再驗收，不硬套。

- [ ] **Step 5: 更新 spec 實測結果與「已知限制」**（Rust 端沒有前端存活檢查：前端整個掛掉時 Cmd+Q 無效；spec 註明），Commit。

```bash
git add docs/superpowers/specs/2026-09-21-window-close-confirm-design.md
git commit -m "docs: record verified behavior for the window-close confirmation

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

- [ ] **Step 6: CHANGELOG 由使用者決定何時發版；本輪不打 tag、不 push。**
