# Agent 分頁關閉確認 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 關閉 Agent（程式庫助手）分頁時，若有進行中的回應或已存在的對話，先跳確認框，避免一次誤觸就永久遺失整段對話。

**Architecture:** 沿用 `TerminalApp` 既有的 close guard 機制（`registerCloseGuard` / `await guard()`），不新增跨層概念。把 LoopStudio 目前私有的確認 Modal 抽成共用元件供兩者使用，並修掉「對非當前分頁觸發 guard 會讓 Modal 畫在隱藏容器裡、Promise 永遠 pending」這個既有缺陷。

**Tech Stack:** React 19 + TypeScript、Vitest + React Testing Library + jsdom、既有的 `useLocale` i18n。

設計文件：`docs/superpowers/specs/2026-08-25-tab-close-confirm-design.md`

---

## 給執行者的重要前提

**這個 repo 的 `docs/` 被 `.gitignore:47` 排除。** 本計畫不需要新增 docs，但若要更新設計文件，必須 `git add -f`。

**測試 harness 已實地驗證過**（不是憑推測寫的）：

- LoopStudio 只需 mock `../../ipc/provider` 與 `../../hooks/useOrchestratorLoop` 兩個模組即可掛載。
- CodeAssistantView 只需 mock `../../ipc/provider`、`../../ipc/config`、`../../hooks/useCodeAssistant` 三個模組即可掛載；且在「沒有 projectRoot」的空狀態分支下渲染，不會拉進 `ModelPickerButton` 的配額 IPC。
- 呼叫 guard 會觸發 `setState`，**必須包在 `act()` 裡**，否則 Modal 不會被 flush，斷言會找不到元素。
- 預設語系是 zh-TW，斷言可直接比對中文字串。

**執行測試：** `npx vitest run <路徑>`。全套：`npm run test`。型別檢查：`npx tsc -b`（**不要**用 `tsc --noEmit`，根 tsconfig 是 solution file，永遠回傳 0）。

---

## File Structure

| 檔案 | 責任 |
|---|---|
| `src/components/CloseConfirmDialog/index.tsx`（新增） | 純呈現的確認框。不知道 close guard 存在，無內部狀態。 |
| `src/components/CloseConfirmDialog/styles.css`（新增） | 上者的樣式，由 LoopStudio 搬移而來。 |
| `src/components/CloseConfirmDialog/index.test.tsx`（新增） | 上者的測試。 |
| `src/components/LoopStudio/closeGuard.test.tsx`（新增） | 釘住 LoopStudio 既有 guard 行為的特徵測試，作為重構的保護網。 |
| `src/components/LoopStudio/index.tsx`（修改） | 改用共用元件；判斷邏輯與文案不動。 |
| `src/components/LoopStudio/styles.css`（修改） | 刪除已搬走的 `.ls-close-*` 規則。 |
| `src/lib/i18n.ts`（修改） | 新增 Agent 確認框的 6 個字串（中英各一份）。 |
| `src/components/CodeAssistantView/index.tsx`（修改） | 接上 close guard 並渲染共用元件。 |
| `src/components/CodeAssistantView/closeGuard.test.tsx`（新增） | Agent guard 的測試。 |
| `src/lib/closeTabGuard.ts`（新增） | 純函式：決定「要不要先切分頁、要不要放行」。抽出來才能不渲染整個 `TerminalApp` 就測到。 |
| `src/lib/closeTabGuard.test.ts`（新增） | 上者的測試。 |
| `src/components/TerminalApp.tsx`（修改） | `handleCloseTab` 改用純函式；把 guard props 傳給 `CodeAssistantView`。 |

---

## Task 1: `CloseConfirmDialog` 共用元件

**Files:**
- Create: `src/components/CloseConfirmDialog/index.tsx`
- Create: `src/components/CloseConfirmDialog/styles.css`
- Test: `src/components/CloseConfirmDialog/index.test.tsx`

- [ ] **Step 1: 寫會紅的測試**

建立 `src/components/CloseConfirmDialog/index.test.tsx`：

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { CloseConfirmDialog } from "./index";

describe("CloseConfirmDialog", () => {
  it("顯示傳入的標題、內文與兩個按鈕文字", () => {
    render(
      <CloseConfirmDialog
        title="標題在這"
        body={<>內文在這</>}
        confirmLabel="關掉"
        cancelLabel="不要"
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />
    );
    expect(screen.getByRole("heading", { level: 3 })).toHaveTextContent("標題在這");
    expect(screen.getByText("內文在這")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "關掉" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "不要" })).toBeInTheDocument();
  });

  it("按確認只呼叫 onConfirm 一次，且不呼叫 onCancel", async () => {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    render(
      <CloseConfirmDialog
        title="t" body={<>b</>} confirmLabel="關掉" cancelLabel="不要"
        onConfirm={onConfirm} onCancel={onCancel}
      />
    );
    await userEvent.click(screen.getByRole("button", { name: "關掉" }));
    expect(onConfirm).toHaveBeenCalledTimes(1);
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("按取消只呼叫 onCancel 一次，且不呼叫 onConfirm", async () => {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    render(
      <CloseConfirmDialog
        title="t" body={<>b</>} confirmLabel="關掉" cancelLabel="不要"
        onConfirm={onConfirm} onCancel={onCancel}
      />
    );
    await userEvent.click(screen.getByRole("button", { name: "不要" }));
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onConfirm).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: 跑測試確認會紅**

執行：`npx vitest run src/components/CloseConfirmDialog/index.test.tsx`

預期：FAIL，訊息為找不到模組 `./index`（元件尚未建立）。

- [ ] **Step 3: 建立元件**

建立 `src/components/CloseConfirmDialog/index.tsx`：

```tsx
import type { ReactNode } from "react";
import "./styles.css";

interface CloseConfirmDialogProps {
  title: string;
  /** 允許多行／`<br />`，沿用 LoopStudio 既有的內文寫法。 */
  body: ReactNode;
  confirmLabel: string;
  cancelLabel: string;
  onConfirm: () => void;
  onCancel: () => void;
}

/**
 * 分頁關閉確認框。純呈現：不知道 close guard 存在，也不持有狀態，
 * 由呼叫端決定何時掛載、按鈕按下後要 resolve 成什麼。
 */
export function CloseConfirmDialog({
  title,
  body,
  confirmLabel,
  cancelLabel,
  onConfirm,
  onCancel,
}: CloseConfirmDialogProps) {
  return (
    <div className="aiterm-close-overlay">
      <div className="aiterm-close-dialog">
        <h3 className="aiterm-close-dialog-title">{title}</h3>
        <p className="aiterm-close-dialog-body">{body}</p>
        <div className="aiterm-close-dialog-actions">
          <button
            type="button"
            className="aiterm-btn aiterm-btn--secondary"
            onClick={onCancel}
          >
            {cancelLabel}
          </button>
          <button
            type="button"
            className="aiterm-btn aiterm-btn--danger-solid"
            onClick={onConfirm}
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: 建立樣式**

建立 `src/components/CloseConfirmDialog/styles.css`。內容是 `src/components/LoopStudio/styles.css:1052-1090` **原樣搬移**，只改 class 前綴，數值一律不動：

```css
/* ── 分頁關閉確認框（共用） ──────────────────────────────────────
   overlay 是 position:absolute，定位基準是 TerminalApp 那個
   position:absolute 的分頁容器（.ls-root / .ca-view 皆未設 position），
   因此正好覆蓋分頁區域而非整個視窗。不要改成 fixed。 */
.aiterm-close-overlay {
  position: absolute;
  inset: 0;
  background: rgba(0, 0, 0, 0.6);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
}

.aiterm-close-dialog {
  background: #1e1e1e;
  border: 1px solid #444;
  border-radius: 8px;
  padding: 24px;
  max-width: 360px;
  width: 90%;
  box-shadow: 0 8px 32px rgba(0,0,0,0.5);
}

.aiterm-close-dialog-title {
  font-size: 15px;
  font-weight: 600;
  margin: 0 0 10px;
  color: #f59e0b;
}

.aiterm-close-dialog-body {
  font-size: 13px;
  color: #bbb;
  margin: 0 0 20px;
  line-height: 1.6;
}

.aiterm-close-dialog-actions {
  display: flex;
  gap: 8px;
  justify-content: flex-end;
}
```

- [ ] **Step 5: 跑測試確認轉綠**

執行：`npx vitest run src/components/CloseConfirmDialog/index.test.tsx`

預期：PASS，3 個測試全過。

- [ ] **Step 6: Commit**

```bash
git add src/components/CloseConfirmDialog/
git commit -m "feat(ui): add shared CloseConfirmDialog component"
```

---

## Task 2: LoopStudio 特徵測試（重構的保護網）

這個任務**不改任何實作**，只補測試把目前正常運作的行為釘住，讓 Task 3 的重構有依據。

**注意：這是特徵測試（characterization test），寫完會立刻是綠的**——因為行為早就存在。所以「先看它紅」在這裡不適用；改用 Step 3 的**刻意破壞**來證明測試真的有在測東西，而不是空過。

**Files:**
- Test: `src/components/LoopStudio/closeGuard.test.tsx`

- [ ] **Step 1: 寫特徵測試**

建立 `src/components/LoopStudio/closeGuard.test.tsx`：

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// 掛載 LoopStudio 只需要這兩個 mock：listProviders 是唯一在 mount 時
// 被呼叫的 IPC，其餘都是事件驅動的。
vi.mock("../../ipc/provider", () => ({ listProviders: () => Promise.resolve([]) }));

const fakeLoop = {
  trace: [] as unknown[],
  isRunning: false,
  iteration: 0,
  start: vi.fn(),
  stop: vi.fn(),
  resume: vi.fn(),
  pendingConfirmation: null as unknown,
};
vi.mock("../../hooks/useOrchestratorLoop", () => ({
  useOrchestratorLoop: () => fakeLoop,
}));

import { LoopStudioView } from "./index";
import { LocaleProvider } from "../../contexts/LocaleContext";

/** 掛載元件，回傳被註冊的 close guard。 */
function mountAndCaptureGuard() {
  let guard: (() => Promise<boolean>) | undefined;
  render(
    <LocaleProvider>
      <LoopStudioView
        tabId="tab-1"
        registerCloseGuard={(_id, g) => { guard = g; }}
        unregisterCloseGuard={vi.fn()}
      />
    </LocaleProvider>
  );
  if (!guard) throw new Error("LoopStudio 沒有註冊 close guard");
  return guard;
}

beforeEach(() => {
  fakeLoop.isRunning = false;
  fakeLoop.stop.mockClear();
});

describe("LoopStudio close guard", () => {
  it("Loop 執行中：跳確認框，Promise 保持未定", async () => {
    fakeLoop.isRunning = true;
    const guard = mountAndCaptureGuard();

    let settled: unknown = "pending";
    // guard() 內部會 setState，必須包在 act 裡才會 flush 出 Modal。
    await act(async () => { void guard().then((v) => { settled = v; }); });

    expect(screen.getByRole("heading", { level: 3 })).toHaveTextContent("Loop 正在執行中");
    expect(settled).toBe("pending");
  });

  it("乾淨狀態：直接放行且不跳確認框", async () => {
    const guard = mountAndCaptureGuard();
    await expect(guard()).resolves.toBe(true);
    expect(screen.queryByRole("heading", { level: 3 })).not.toBeInTheDocument();
  });

  it("執行中按「關閉不儲存」：resolve true 並停止 loop", async () => {
    fakeLoop.isRunning = true;
    const guard = mountAndCaptureGuard();

    let settled: unknown = "pending";
    await act(async () => { void guard().then((v) => { settled = v; }); });
    await userEvent.click(screen.getByRole("button", { name: "關閉不儲存" }));

    expect(settled).toBe(true);
    expect(fakeLoop.stop).toHaveBeenCalledTimes(1);
  });

  it("執行中按「取消」：resolve false 且不停止 loop", async () => {
    fakeLoop.isRunning = true;
    const guard = mountAndCaptureGuard();

    let settled: unknown = "pending";
    await act(async () => { void guard().then((v) => { settled = v; }); });
    await userEvent.click(screen.getByRole("button", { name: "取消（繼續編輯）" }));

    expect(settled).toBe(false);
    expect(fakeLoop.stop).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: 跑測試，應為綠**

執行：`npx vitest run src/components/LoopStudio/closeGuard.test.tsx`

預期：PASS，4 個測試全過（行為早已存在）。

- [ ] **Step 3: 證明測試真的有在測（刻意破壞 → 必須變紅）**

在 `src/components/LoopStudio/index.tsx:181` 把觸發條件暫時改成永遠不成立：

```tsx
      if (false && loop.isRunning) {
```

執行：`npx vitest run src/components/LoopStudio/closeGuard.test.tsx`

預期：FAIL，「Loop 執行中」那三個測試必須全部變紅。

**若有任何一個仍是綠的，代表那個測試沒有真的驗到 guard，必須先修測試再往下。**

改完後**務必把 `if (false && ` 還原成 `if (`**，再跑一次確認回到 4 綠。

- [ ] **Step 4: Commit**

```bash
git add src/components/LoopStudio/closeGuard.test.tsx
git commit -m "test(loop-studio): pin close guard behavior before refactor"
```

---

## Task 3: LoopStudio 改用共用元件

**Files:**
- Modify: `src/components/LoopStudio/index.tsx:386-422`
- Modify: `src/components/LoopStudio/styles.css:1052-1095`

- [ ] **Step 1: 匯入共用元件**

在 `src/components/LoopStudio/index.tsx` 的 import 區（第 14 行 `import { ModelPickerButton }` 之後）加入：

```tsx
import { CloseConfirmDialog } from "../CloseConfirmDialog";
```

- [ ] **Step 2: 換掉 Modal 的 JSX**

把 `src/components/LoopStudio/index.tsx` 中這一整段：

```tsx
      {showCloseConfirm && (
        <div className="ls-close-overlay">
          <div className="ls-close-dialog">
            <h3 className="ls-close-dialog-title">
              {loop.isRunning ? t.ls_close_title_running : t.ls_close_title_dirty}
            </h3>
            <p className="ls-close-dialog-body">
              {loop.isRunning ? (
                <>{t.ls_close_body_running}</>
              ) : (
                <>
                  {t.ls_close_body_dirty}<br />
                  {currentProjectPath
                    ? t.ls_close_body_modified
                    : t.ls_close_body_unsaved}
                </>
              )}
            </p>
            <div className="ls-close-dialog-actions">
              <button
                type="button"
                className="aiterm-btn aiterm-btn--secondary"
                onClick={() => handleCloseConfirm(false)}
              >
                {t.ls_cancel_continue}
              </button>
              <button
                type="button"
                className="aiterm-btn aiterm-btn--danger-solid"
                onClick={() => handleCloseConfirm(true)}
              >
                {t.ls_close_discard}
              </button>
            </div>
          </div>
        </div>
      )}
```

替換為：

```tsx
      {showCloseConfirm && (
        <CloseConfirmDialog
          title={loop.isRunning ? t.ls_close_title_running : t.ls_close_title_dirty}
          body={loop.isRunning ? (
            <>{t.ls_close_body_running}</>
          ) : (
            <>
              {t.ls_close_body_dirty}<br />
              {currentProjectPath
                ? t.ls_close_body_modified
                : t.ls_close_body_unsaved}
            </>
          )}
          confirmLabel={t.ls_close_discard}
          cancelLabel={t.ls_cancel_continue}
          onConfirm={() => handleCloseConfirm(true)}
          onCancel={() => handleCloseConfirm(false)}
        />
      )}
```

**文案與判斷條件一字不動**，只換渲染方式。

- [ ] **Step 3: 刪除搬走的樣式**

從 `src/components/LoopStudio/styles.css` 刪除 `.ls-close-overlay`、`.ls-close-dialog`、`.ls-close-dialog-title`、`.ls-close-dialog-body`、`.ls-close-dialog-actions` 五條規則，以及底下那兩行說明按鈕已被取代的註解（原第 1052-1095 行整段）。

- [ ] **Step 4: 跑 Task 2 的測試，必須仍是綠的**

執行：`npx vitest run src/components/LoopStudio/closeGuard.test.tsx`

預期：PASS，4 個測試全過。**若有任何一個變紅，代表重構改動了行為，必須修到全綠再往下。**

- [ ] **Step 5: 型別檢查**

執行：`npx tsc -b`

預期：無輸出、exit code 0。

- [ ] **Step 6: Commit**

```bash
git add src/components/LoopStudio/index.tsx src/components/LoopStudio/styles.css
git commit -m "refactor(loop-studio): use shared CloseConfirmDialog"
```

---

## Task 4: 新增 Agent 確認框的 i18n 字串

**Files:**
- Modify: `src/lib/i18n.ts:234`（zh-TW）、`src/lib/i18n.ts:1505`（en）

- [ ] **Step 1: 加入 zh-TW 字串**

在 `src/lib/i18n.ts` 第 234 行 `ca_checkpoint_notice: ...` 那一行**之後**插入：

```ts
    ca_close_title_streaming: "AI 正在回應中",
    ca_close_title_dirty: "對話尚未保存",
    ca_close_body_streaming: "關閉分頁會中斷這次回應，且整段對話不會保留。",
    ca_close_body_dirty: "這個分頁的對話沒有存檔，關閉後將永久遺失。",
    ca_close_cancel: "取消（返回對話）",
    ca_close_discard: "關閉並捨棄",
```

- [ ] **Step 2: 加入 en 字串**

在 `src/lib/i18n.ts` 第 1505 行（英文區塊的 `ca_checkpoint_notice: ...`）那一行**之後**插入：

```ts
    ca_close_title_streaming: "AI is still responding",
    ca_close_title_dirty: "Conversation not saved",
    ca_close_body_streaming: "Closing this tab interrupts the response, and the whole conversation will be lost.",
    ca_close_body_dirty: "This tab's conversation is not saved anywhere. Closing it discards the conversation permanently.",
    ca_close_cancel: "Cancel (back to chat)",
    ca_close_discard: "Close and discard",
```

- [ ] **Step 3: 型別檢查**

執行：`npx tsc -b`

預期：無輸出、exit code 0。

> **更正（2026-08-25，執行時實測推翻原假設）：** 本步驟原本寫成「兩個語系的 key 若不對稱，這裡就會報錯」，**這是錯的**。`src/lib/i18n.ts:2458-2464` 的英文物件是 `{ ...zhTW, ...enRaw }`，且 `TranslationKey` 只從 `zh-TW` 推導——英文缺 key 會靜默 fallback 成中文字串，`tsc -b` 一律通過。實測方式：故意刪掉一個英文 key 後 `tsc -b` 仍 exit 0。
>
> 因此 `tsc -b` 只能確認物件結構合法，**不能**當作語系對稱的防線。兩個語系都要加 key，必須靠人工核對（例如 `grep -c` 該 key 應為 2）。

- [ ] **Step 4: Commit**

```bash
git add src/lib/i18n.ts
git commit -m "i18n: add strings for the Agent tab close confirmation"
```

---

## Task 5: Agent 分頁接上 close guard

**Files:**
- Modify: `src/components/CodeAssistantView/index.tsx`
- Test: `src/components/CodeAssistantView/closeGuard.test.tsx`

- [ ] **Step 1: 寫會紅的測試**

建立 `src/components/CodeAssistantView/closeGuard.test.tsx`：

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// 掛載 CodeAssistantView 只需要這三個 mock。刻意不設定 projectRoot，
// 讓它渲染「選擇專案目錄」的空狀態分支——註冊 guard 的 effect 一樣會跑，
// 但不會拉進 ModelPickerButton 的配額 IPC。
vi.mock("../../ipc/provider", () => ({ listProviders: () => Promise.resolve([]) }));
vi.mock("../../ipc/config", () => ({ getConfig: () => Promise.resolve({ submit_shortcut: "enter" }) }));

const fakeAssistant = {
  messages: [] as { role: string; content: string }[],
  isStreaming: false,
  error: null as string | null,
  isFallbackMode: false,
  tokenCount: 0,
  tokenLimit: 100,
  send: vi.fn(),
  clear: vi.fn(),
};
vi.mock("../../hooks/useCodeAssistant", () => ({
  useCodeAssistant: () => fakeAssistant,
}));

import { CodeAssistantView } from "./index";
import { LocaleProvider } from "../../contexts/LocaleContext";

// register/unregister 在真實的 TerminalApp 裡是 useCallback([]) 的穩定引用。
// 測試必須照樣給穩定引用，否則每次 rerender 都會讓 effect 重新註冊一輪，
// 測到的就不是真實情境。
function mountAndCaptureGuard() {
  let guard: (() => Promise<boolean>) | undefined;
  const register = (_id: string, g: () => Promise<boolean>) => { guard = g; };
  const unregister = vi.fn();
  const ui = (
    <LocaleProvider>
      <CodeAssistantView
        isActive
        tabId="tab-1"
        registerCloseGuard={register}
        unregisterCloseGuard={unregister}
      />
    </LocaleProvider>
  );
  const view = render(ui);
  if (!guard) throw new Error("CodeAssistantView 沒有註冊 close guard");
  return { guard, view, unregister, ui };
}

beforeEach(() => {
  fakeAssistant.messages = [];
  fakeAssistant.isStreaming = false;
});

describe("Agent 分頁 close guard", () => {
  it("全新空白分頁：直接放行且不跳確認框", async () => {
    const { guard } = mountAndCaptureGuard();
    await expect(guard()).resolves.toBe(true);
    expect(screen.queryByRole("heading", { level: 3 })).not.toBeInTheDocument();
  });

  it("已有對話紀錄：跳確認框，Promise 保持未定", async () => {
    fakeAssistant.messages = [{ role: "user", content: "hi" }];
    const { guard } = mountAndCaptureGuard();

    let settled: unknown = "pending";
    await act(async () => { void guard().then((v) => { settled = v; }); });

    expect(screen.getByRole("heading", { level: 3 })).toHaveTextContent("對話尚未保存");
    expect(settled).toBe("pending");
  });

  it("正在串流：確認框顯示串流版本的文案", async () => {
    fakeAssistant.isStreaming = true;
    const { guard } = mountAndCaptureGuard();

    await act(async () => { void guard(); });

    expect(screen.getByRole("heading", { level: 3 })).toHaveTextContent("AI 正在回應中");
  });

  it("按「關閉並捨棄」：resolve true", async () => {
    fakeAssistant.messages = [{ role: "user", content: "hi" }];
    const { guard } = mountAndCaptureGuard();

    let settled: unknown = "pending";
    await act(async () => { void guard().then((v) => { settled = v; }); });
    await userEvent.click(screen.getByRole("button", { name: "關閉並捨棄" }));

    expect(settled).toBe(true);
  });

  it("按「取消（返回對話）」：resolve false 且確認框消失", async () => {
    fakeAssistant.messages = [{ role: "user", content: "hi" }];
    const { guard } = mountAndCaptureGuard();

    let settled: unknown = "pending";
    await act(async () => { void guard().then((v) => { settled = v; }); });
    await userEvent.click(screen.getByRole("button", { name: "取消（返回對話）" }));

    expect(settled).toBe(false);
    expect(screen.queryByRole("heading", { level: 3 })).not.toBeInTheDocument();
  });

  // 這一題釘住整個功能最容易靜默失效的地方：guard 是在 messages 還是空的
  // 時候註冊的，如果它閉包捕捉了當時的 messages，之後談再多輪也會直接放行。
  it("註冊之後才產生的對話，guard 仍看得到（不可讀到過期狀態）", async () => {
    const { guard, view, ui } = mountAndCaptureGuard();

    // 註冊當下 messages 是空的。模擬「註冊完 guard 之後，使用者才跟 AI 談話」，
    // 用同一組穩定 props 重繪，讓元件讀到新的 messages。
    fakeAssistant.messages = [{ role: "user", content: "後來才講的話" }];
    view.rerender(ui);

    let settled: unknown = "pending";
    await act(async () => { void guard().then((v) => { settled = v; }); });

    expect(screen.getByRole("heading", { level: 3 })).toHaveTextContent("對話尚未保存");
    expect(settled).toBe("pending");
  });

  it("unmount 時解除註冊", () => {
    const { view, unregister } = mountAndCaptureGuard();
    view.unmount();
    expect(unregister).toHaveBeenCalledWith("tab-1");
  });
});
```

- [ ] **Step 2: 跑測試確認會紅**

執行：`npx vitest run src/components/CodeAssistantView/closeGuard.test.tsx`

預期：FAIL，全部 7 個測試都紅，訊息為 `CodeAssistantView 沒有註冊 close guard`（元件還沒接 guard）。

- [ ] **Step 3: 擴充 Props 型別**

修改 `src/components/CodeAssistantView/index.tsx` 的 `interface Props`（原第 23-25 行）：

```tsx
interface Props {
  isActive: boolean;
  tabId?: string;
  registerCloseGuard?: (tabId: string, guard: () => Promise<boolean>) => void;
  unregisterCloseGuard?: (tabId: string) => void;
}

export function CodeAssistantView({
  isActive,
  tabId,
  registerCloseGuard,
  unregisterCloseGuard,
}: Props) {
```

- [ ] **Step 4: 加入 guard 狀態與 ref**

在 `src/components/CodeAssistantView/index.tsx` 中，`const { messages, isStreaming, ... } = useCodeAssistant();`（原第 43 行）**之後**加入：

```tsx
  // 關閉確認：ref 是必要的，不是風格選擇。guard 只在 tabId 變動時重新註冊，
  // 若直接閉包捕捉 messages/isStreaming，之後談的每一輪它都看不到，
  // 會在沒有任何錯誤訊號的情況下直接放行——功能等於靜默失效。
  const hasMessagesRef = useRef(false);
  hasMessagesRef.current = messages.length > 0;
  const isStreamingRef = useRef(false);
  isStreamingRef.current = isStreaming;

  const [showCloseConfirm, setShowCloseConfirm] = useState(false);
  const closeResolveRef = useRef<((canClose: boolean) => void) | null>(null);

  const handleCloseConfirm = useCallback((canClose: boolean) => {
    setShowCloseConfirm(false);
    closeResolveRef.current?.(canClose);
    closeResolveRef.current = null;
  }, []);
```

（直接在 render 期間指派 ref，與此檔既有的 `submitShortcutRef.current = submitShortcut;` 同一種寫法。）

- [ ] **Step 5: 註冊 close guard**

緊接在 Step 4 的程式碼**之後**加入。**必須放在第 186 行 `if (!projectRoot) { return (...) }` 這個提前 return 之前**——React 的 hooks 不能在條件式 return 之後呼叫：

```tsx
  useEffect(() => {
    if (!tabId || !registerCloseGuard) return;
    registerCloseGuard(tabId, () => {
      // 全新、沒談過話的分頁沒有東西可失去，不要打擾使用者。
      if (!isStreamingRef.current && !hasMessagesRef.current) {
        return Promise.resolve(true);
      }
      return new Promise<boolean>((resolve) => {
        closeResolveRef.current = resolve;
        setShowCloseConfirm(true);
      });
    });
    return () => { unregisterCloseGuard?.(tabId); };
  }, [tabId, registerCloseGuard, unregisterCloseGuard]);
```

- [ ] **Step 6: 渲染確認框**

在 `src/components/CodeAssistantView/index.tsx` 的 import 區加入：

```tsx
import { CloseConfirmDialog } from "../CloseConfirmDialog";
```

接著在**兩個** return 分支的 `<div className="ca-view">` 內部最前面各插入同一段（空狀態分支在原第 187 行附近，主分支在原第 201 行附近）——因為使用者可能在還沒選專案目錄時就已經有對話，或反過來：

```tsx
      {showCloseConfirm && (
        <CloseConfirmDialog
          title={isStreaming ? t.ca_close_title_streaming : t.ca_close_title_dirty}
          body={isStreaming ? t.ca_close_body_streaming : t.ca_close_body_dirty}
          confirmLabel={t.ca_close_discard}
          cancelLabel={t.ca_close_cancel}
          onConfirm={() => handleCloseConfirm(true)}
          onCancel={() => handleCloseConfirm(false)}
        />
      )}
```

- [ ] **Step 7: 跑測試確認轉綠**

執行：`npx vitest run src/components/CodeAssistantView/closeGuard.test.tsx`

預期：PASS，7 個測試全過。

- [ ] **Step 8: 型別檢查**

執行：`npx tsc -b`

預期：無輸出、exit code 0。

- [ ] **Step 9: Commit**

```bash
git add src/components/CodeAssistantView/
git commit -m "feat(code-assistant): confirm before closing a tab with a conversation"
```

---

## Task 6: 關閉前先切換到該分頁

沒有這一步，Task 5 只在「Agent 分頁剛好是當前分頁」時有效；點旁邊那個非當前分頁的 `✕`，確認框會畫在 `visibility: hidden` 的容器裡，使用者看不見也點不到，`handleCloseTab` 會永遠 await 下去。

判斷邏輯抽成純函式才測得到——完整渲染 `TerminalApp` 需要 mock xterm 與全部 11 種分頁，成本過高。此 repo 已有這種慣例（`LoopStudio/validateRoster.ts`、`HomeView/routeIntent.ts`）。

**Files:**
- Create: `src/lib/closeTabGuard.ts`
- Test: `src/lib/closeTabGuard.test.ts`
- Modify: `src/components/TerminalApp.tsx:301-306`、`:582`

- [ ] **Step 1: 寫會紅的測試**

建立 `src/lib/closeTabGuard.test.ts`：

```ts
import { describe, it, expect, vi } from "vitest";
import { runCloseGuard } from "./closeTabGuard";

describe("runCloseGuard", () => {
  it("沒有 guard 時直接放行，且不切換分頁", async () => {
    const setActiveId = vi.fn();
    await expect(
      runCloseGuard("tab-2", "tab-1", undefined, setActiveId)
    ).resolves.toBe(true);
    expect(setActiveId).not.toHaveBeenCalled();
  });

  it("有 guard 但目標不是當前分頁：先切過去再問", async () => {
    const setActiveId = vi.fn();
    const order: string[] = [];
    const guard = vi.fn(async () => { order.push("guard"); return true; });
    setActiveId.mockImplementation(() => { order.push("switch"); });

    await runCloseGuard("tab-2", "tab-1", guard, setActiveId);

    expect(setActiveId).toHaveBeenCalledWith("tab-2");
    expect(order).toEqual(["switch", "guard"]);
  });

  it("有 guard 且目標已是當前分頁：不重複切換", async () => {
    const setActiveId = vi.fn();
    const guard = vi.fn(async () => true);

    await runCloseGuard("tab-1", "tab-1", guard, setActiveId);

    expect(setActiveId).not.toHaveBeenCalled();
    expect(guard).toHaveBeenCalledTimes(1);
  });

  it("guard 回傳 false 時原樣傳回", async () => {
    const guard = vi.fn(async () => false);
    await expect(
      runCloseGuard("tab-1", "tab-1", guard, vi.fn())
    ).resolves.toBe(false);
  });
});
```

- [ ] **Step 2: 跑測試確認會紅**

執行：`npx vitest run src/lib/closeTabGuard.test.ts`

預期：FAIL，找不到模組 `./closeTabGuard`。

- [ ] **Step 3: 建立純函式**

建立 `src/lib/closeTabGuard.ts`：

```ts
/**
 * 決定一個分頁能不能關閉。
 *
 * 非當前分頁的內容是 `visibility: hidden` + `pointerEvents: none`
 * （見 TerminalApp 的 HIDDEN LAYOUT TRICK），確認框若畫在裡面，使用者
 * 看不見也點不到，await 就永遠不會 resolve。所以有 guard 時先把該分頁
 * 切成當前分頁再問；沒有 guard 的分頁維持原本行為，不做無謂的切換。
 */
export async function runCloseGuard(
  id: string,
  activeId: string | null,
  guard: (() => Promise<boolean>) | undefined,
  setActiveId: (id: string) => void,
): Promise<boolean> {
  if (!guard) return true;
  if (activeId !== id) setActiveId(id);
  return guard();
}
```

- [ ] **Step 4: 跑測試確認轉綠**

執行：`npx vitest run src/lib/closeTabGuard.test.ts`

預期：PASS，4 個測試全過。

- [ ] **Step 5: 接進 `handleCloseTab`**

在 `src/components/TerminalApp.tsx` 的 import 區加入：

```tsx
import { runCloseGuard } from "../lib/closeTabGuard";
```

把 `src/components/TerminalApp.tsx` 中這段：

```tsx
    const guard = closeGuardsRef.current.get(id);
    if (guard) {
      const canClose = await guard();
      if (!canClose) return;
    }
```

替換為：

```tsx
    const canClose = await runCloseGuard(
      id,
      activeIdRef.current,
      closeGuardsRef.current.get(id),
      setActiveId,
    );
    if (!canClose) return;
```

- [ ] **Step 6: 把 guard props 傳給 Agent 分頁**

把 `src/components/TerminalApp.tsx` 的這一行：

```tsx
                <CodeAssistantView isActive={isActive} />
```

替換為：

```tsx
                <CodeAssistantView
                  isActive={isActive}
                  tabId={tab.id}
                  registerCloseGuard={registerCloseGuard}
                  unregisterCloseGuard={unregisterCloseGuard}
                />
```

- [ ] **Step 7: 型別檢查與全套測試**

執行：`npx tsc -b`
預期：無輸出、exit code 0。

執行：`npm run test`
預期：全部通過，無新增失敗。

- [ ] **Step 8: Commit**

```bash
git add src/lib/closeTabGuard.ts src/lib/closeTabGuard.test.ts src/components/TerminalApp.tsx
git commit -m "fix(tabs): focus a tab before running its close guard"
```

---

## Task 7: 實機驗證

自動化測試涵蓋不到外觀與真實的關閉流程，這一步必須實際跑起來看。

- [ ] **Step 1: 啟動 App**

執行：`npm run tauri:dev`

- [ ] **Step 2: 驗證 Agent 分頁「空白直接關」**

開一個新的 Agent 分頁，什麼都不做，直接按 `✕`。

預期：**沒有**確認框，分頁直接關閉。

- [ ] **Step 3: 驗證「有對話會攔」**

再開一個 Agent 分頁，選好專案目錄後跟 AI 談一輪，等回應結束，按 `✕`。

預期：跳出「對話尚未保存」確認框。按「取消（返回對話）」→ 分頁還在、對話還在。再按一次 `✕` 並選「關閉並捨棄」→ 分頁關閉。

- [ ] **Step 4: 驗證「串流中會攔」**

在 Agent 分頁送出一個問題，**趁 AI 還在回應時**按 `✕`。

預期：跳出「AI 正在回應中」確認框。

- [ ] **Step 5: 驗證非當前分頁（這是 Task 6 的重點）**

切換到**別的**分頁（例如終端機），然後點側邊欄上那個有對話的 Agent 分頁的 `✕`。

預期：畫面**自動切到該 Agent 分頁**並顯示確認框，而不是卡住或什麼都沒發生。

- [ ] **Step 6: 驗證 Ctrl+W**

在有對話的 Agent 分頁按 `Ctrl+W`。

預期：同樣跳出確認框。

- [ ] **Step 7: 驗證 LoopStudio 沒有退步**

開 LoopStudio 分頁，設定一個目標讓它變 dirty，按 `✕`。

預期：確認框外觀與重構前一致（深色底、橘色標題、右下兩顆按鈕），文案未變，取消與關閉都正確。

- [ ] **Step 8: 全部通過後 commit（若有修正）**

若上述步驟發現問題並做了修正：

```bash
git add -A
git commit -m "fix(tabs): address issues found in manual verification"
```

若無修正則跳過此步。

---

## Self-Review 對照表

| Spec 要求 | 對應任務 |
|---|---|
| 共用 `CloseConfirmDialog`（純呈現、props 介面） | Task 1 |
| CSS 原樣搬移、保留 `position: absolute` | Task 1 Step 4 |
| LoopStudio 改用共用元件、判斷與文案不動 | Task 3 |
| 重構前先補行為測試 | Task 2（含刻意破壞的紅燈證明） |
| Agent 接 guard、條件為「執行中或有對話」 | Task 5 Step 5 |
| 新增 i18n 字串（中英各一份） | Task 4 |
| `handleCloseTab` 先切分頁再問 | Task 6 |
| 陷阱 1：guard 不可讀到過期 state | Task 5 Step 4（ref）+ Step 1 的「不可讀到過期狀態」測試 |
| 陷阱 2：cleanup 必須 unregister | Task 5 Step 5 + Step 1 的 unmount 測試 |
| 陷阱 3：重構前先補測試 | Task 2 |
| 風險：外觀跑掉 | Task 7 Step 7 |
