# 側邊欄終端機提示點 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓非 active 的終端機分頁在側邊欄顯示三色提示點（等待輸入／完成／失敗），並在 app 失焦時對「等待輸入」與「失敗」發送桌面通知。

**Architecture:** `TerminalView` 從 xterm 的 `onBell` 與既有的 OSC 133 `D` handler 各取得一個訊號，透過新的 `onAttention` prop 一律往上回報給 `TerminalApp`。`TerminalApp` 用一個純函式 `routeAttention` 把單一事件拆成兩個**互相獨立**的決定：要不要設提示點（看是不是 active 分頁）、要不要發通知（看視窗有沒有 focus）。提示點狀態存在既有的 `Tab` 物件上，由 `TabBar` 渲染。

**Tech Stack:** React 19、TypeScript、`@xterm/xterm` 5.5（`onBell`）、`@tauri-apps/plugin-notification`、`@tauri-apps/api/window`、Vitest + React Testing Library。

**設計文件：** `docs/superpowers/specs/2026-08-05-terminal-attention-badge-design.md`

---

## 背景：讀之前要知道的事

**OSC 133 是什麼。** 這是 shell integration 的跳脫序列，shell 在指令開始（`C`）與結束（`D;<exitCode>`）時寫進終端機。本專案已經在 `src/hooks/useTerminalBlocks.ts:142` 註冊了 handler 來切割「指令區塊」。本計畫**只在既有 handler 旁邊多呼叫一個 callback**，不改動它任何現有邏輯——尤其是 Windows/ConPTY 的延遲 clear 與 Ctrl+L 重新同步那段（`useTerminalBlocks.ts:158-190`），那是踩過坑修好的，註解裡有完整的 root cause。

**為什麼 callback 要經過 ref。** 本專案的 xterm 與 PTY 事件監聽器都在「掛載一次」的 effect 裡註冊，直接閉包捕捉 props 會拿到 stale 值。既有解法是 ref 橋接，見 `TerminalView.tsx:260-267` 的 `submitCommandRef`。本計畫照抄這個模式。

**桌面通知在 dev 模式下看不到。** `useMailSync.ts:104-111` 有完整說明：`tauri dev` 下外掛會以 `com.apple.Terminal` 的身分送出，且 `show()` 的結果被丟棄，失敗完全不可觀測。側邊欄的點可以在 `npm run tauri:dev` 下直接驗證，通知必須出 `tauri build` 才能確認。

**檔案結構**

| 檔案 | 責任 |
|---|---|
| `src/lib/terminalAttention.ts`（新增） | `AttentionKind` 型別與 `routeAttention` 純函式。整個功能唯一有分支邏輯的地方，也是唯一需要單元測試的決策點。 |
| `src/lib/notifyPermission.ts`（新增） | 全 app 共用的通知權限取得。從 `useMailSync` 抽出。 |
| `src/hooks/useTerminalBlocks.ts` | 多送一個 `onCommandSettled(exitCode)`。 |
| `src/components/TerminalView.tsx` | 接 `onBell`、接 `onCommandSettled`，兩者都轉成 `onAttention` 往上送。 |
| `src/components/TerminalApp.tsx` | 追蹤視窗焦點；呼叫 `routeAttention`；設定／清除提示點；發通知。 |
| `src/components/TabBar/index.tsx` + `.css` | `Tab.attention` 欄位與提示點渲染。 |
| `src/lib/i18n.ts` | 5 組字串 × 2 語言。 |

**與設計文件測試清單的一處差異（刻意的）。** 設計文件把兩條規則列在「`TerminalApp` 測試」底下：

1. *「`onAttention` 不會在當前 active 的分頁上設出提示點」* — 這條被搬進 Task 1 的 `routeAttention` 單元測試，涵蓋範圍相同且不需要測試替身。
2. *「切換到某個有 `attention` 的分頁會清掉它」* — **只做手動驗證**（Task 8 Step 2 第 5 點）。

原因：`TerminalApp` 目前**沒有任何測試檔**。要為它建一個，得同時偽造 Tauri IPC、xterm、react-router 與 localStorage，而被測的只是一個三行的 `useEffect`。這個成本與收穫不成比例。如果之後有人為了別的理由建立了 `TerminalApp` 的測試骨架，這條就該補進去。

---

## Task 1: `routeAttention` 純函式

這是整個功能的決策核心。設計文件特別點名：提示點與通知是**兩個獨立條件**，實作時最容易被錯誤地合併成一個提早 return。把兩者放進同一個純函式，就能用測試把「它們是獨立的」這件事釘死。

**Files:**
- Create: `src/lib/terminalAttention.ts`
- Test: `src/lib/terminalAttention.test.ts`

- [ ] **Step 1: 寫失敗的測試**

建立 `src/lib/terminalAttention.test.ts`：

```ts
import { describe, expect, it } from "vitest";
import { routeAttention } from "./terminalAttention";

describe("routeAttention — 提示點", () => {
  it("非 active 分頁會設出對應的提示點", () => {
    expect(routeAttention({ isActiveTab: false, windowFocused: true, kind: "done" }).badge).toBe("done");
    expect(routeAttention({ isActiveTab: false, windowFocused: true, kind: "failed" }).badge).toBe("failed");
    expect(routeAttention({ isActiveTab: false, windowFocused: true, kind: "waiting" }).badge).toBe("waiting");
  });

  it("active 分頁不設提示點——使用者切回來就直接看到終端機內容了", () => {
    expect(routeAttention({ isActiveTab: true, windowFocused: true, kind: "waiting" }).badge).toBeNull();
    expect(routeAttention({ isActiveTab: true, windowFocused: false, kind: "failed" }).badge).toBeNull();
  });
});

describe("routeAttention — 桌面通知", () => {
  it("視窗有 focus 時一律不發通知——側邊欄的點就夠了", () => {
    expect(routeAttention({ isActiveTab: false, windowFocused: true, kind: "waiting" }).notify).toBe(false);
    expect(routeAttention({ isActiveTab: false, windowFocused: true, kind: "failed" }).notify).toBe(false);
  });

  it("視窗失焦時，waiting 與 failed 會發通知", () => {
    expect(routeAttention({ isActiveTab: false, windowFocused: false, kind: "waiting" }).notify).toBe(true);
    expect(routeAttention({ isActiveTab: false, windowFocused: false, kind: "failed" }).notify).toBe(true);
  });

  it("done 永遠不發通知——指令單純跑完不緊急", () => {
    expect(routeAttention({ isActiveTab: false, windowFocused: false, kind: "done" }).notify).toBe(false);
  });

  // 設計文件明確標記為必測：這是實作時最可能被錯誤合併掉的一條規則。
  // 使用者人不在 app 前面時，「它是 active 分頁」不代表有人在看。
  it("失焦 + active 分頁 + waiting → 不設提示點，但要發通知", () => {
    const r = routeAttention({ isActiveTab: true, windowFocused: false, kind: "waiting" });
    expect(r.badge).toBeNull();
    expect(r.notify).toBe(true);
  });
});
```

- [ ] **Step 2: 執行測試，確認它失敗**

Run: `npx vitest run src/lib/terminalAttention.test.ts`
Expected: FAIL，訊息類似 `Failed to resolve import "./terminalAttention"`。

- [ ] **Step 3: 寫最小實作**

建立 `src/lib/terminalAttention.ts`：

```ts
/** 終端機分頁發生的、值得使用者注意的事件。 */
export type AttentionKind = "waiting" | "done" | "failed";

export interface AttentionInput {
  /** 這個事件來自使用者當前正在看的那個分頁嗎？ */
  isActiveTab: boolean;
  /** app 視窗此刻有沒有 focus？ */
  windowFocused: boolean;
  kind: AttentionKind;
}

export interface AttentionRouting {
  /** 要設在分頁上的提示點；null 表示不設。 */
  badge: AttentionKind | null;
  /** 要不要發桌面通知。 */
  notify: boolean;
}

/**
 * 把一個 attention 事件拆成兩個「互相獨立」的決定。
 *
 * 這兩個條件不能合併：app 失焦時，即使事件來自 active 分頁也要發通知——
 * 使用者人不在 app 前面，「它是 active 分頁」不代表有人在看。反過來，
 * 提示點對 active 分頁沒有意義，因為使用者一切回來就會看到終端機內容。
 */
export function routeAttention({ isActiveTab, windowFocused, kind }: AttentionInput): AttentionRouting {
  return {
    badge: isActiveTab ? null : kind,
    notify: !windowFocused && (kind === "waiting" || kind === "failed"),
  };
}
```

- [ ] **Step 4: 執行測試，確認它通過**

Run: `npx vitest run src/lib/terminalAttention.test.ts`
Expected: PASS，6 個 test 全綠。

- [ ] **Step 5: Commit**

```bash
git add src/lib/terminalAttention.ts src/lib/terminalAttention.test.ts
git commit -m "feat(terminal): 加入 attention 事件的分派規則"
```

---

## Task 2: i18n 字串

提示點的 aria-label 與通知內文都需要 en / zh-TW 兩份。先加字串，Task 3 才有東西可用。

**Files:**
- Modify: `src/lib/i18n.ts:661`（zh-TW）、`src/lib/i18n.ts:1753`（en）

- [ ] **Step 1: 加入 zh-TW 字串**

在 `src/lib/i18n.ts` 第 661 行 `tabbar_sidebar_expand: "開啟側邊欄 (Ctrl+B)",` **後面**插入：

```ts
    // 側邊欄終端機提示點：aria-label（顏色只對看得見的人有意義，
    // 狀態語意必須另外用文字表達）
    terminal_attention_waiting_label: "終端機正在等待你的回應",
    terminal_attention_done_label: "終端機指令已完成",
    terminal_attention_failed_label: "終端機指令失敗",
    // 桌面通知內文（標題用分頁名稱）
    terminal_notify_waiting: "正在等待你的回應",
    terminal_notify_failed: "指令執行失敗",
```

- [ ] **Step 2: 加入 en 字串**

在 `src/lib/i18n.ts` 第 1753 行 `tabbar_sidebar_expand: "Open Sidebar (Ctrl+B)",` **後面**插入：

```ts
    terminal_attention_waiting_label: "Terminal is waiting for your response",
    terminal_attention_done_label: "Terminal command finished",
    terminal_attention_failed_label: "Terminal command failed",
    terminal_notify_waiting: "Waiting for your response",
    terminal_notify_failed: "A command failed",
```

- [ ] **Step 3: 型別檢查**

Run: `npx tsc -b`
Expected: 沒有輸出、exit 0。

> 注意：**不要**用 `tsc --noEmit`。根目錄 `tsconfig.json` 是 solution file（`"files": []`），那樣跑什麼都不會檢查而且一定回傳 0。這是 `CLAUDE.md` 明文記載的陷阱。

- [ ] **Step 4: Commit**

```bash
git add src/lib/i18n.ts
git commit -m "feat(i18n): 終端機提示點與通知字串"
```

---

## Task 3: TabBar 提示點渲染

**Files:**
- Modify: `src/components/TabBar/index.tsx`（`Tab` 介面 + 圖示內渲染）
- Modify: `src/components/TabBar/index.css`
- Test: `src/components/TabBar/index.test.tsx`

- [ ] **Step 1: 寫失敗的測試**

在 `src/components/TabBar/index.test.tsx` 檔案**最末端**（第 100 行 `});` 之後）追加：

```ts
// LocaleProvider 預設 zh-TW，所以這裡斷言 zh-TW 的無障礙名稱。
describe("TabBar terminal attention indicator", () => {
  // 提示點只出現在非 active 分頁上，所以這裡刻意讓 activeId 指向另一個分頁。
  const twoTabs: Tab[] = [
    { id: "t1", title: "Tab 1", type: "terminal" },
    { id: "t2", title: "Tab 2", type: "terminal" },
  ];

  it("等待輸入時標示該分頁", () => {
    renderTabBar({
      tabs: [twoTabs[0], { ...twoTabs[1], attention: "waiting" }],
      activeId: "t1",
    });
    expect(screen.getByRole("img", { name: "終端機正在等待你的回應" })).toBeTruthy();
  });

  it("指令完成時標示該分頁", () => {
    renderTabBar({
      tabs: [twoTabs[0], { ...twoTabs[1], attention: "done" }],
      activeId: "t1",
    });
    expect(screen.getByRole("img", { name: "終端機指令已完成" })).toBeTruthy();
  });

  it("指令失敗時標示該分頁", () => {
    renderTabBar({
      tabs: [twoTabs[0], { ...twoTabs[1], attention: "failed" }],
      activeId: "t1",
    });
    expect(screen.getByRole("img", { name: "終端機指令失敗" })).toBeTruthy();
  });

  it("三種狀態用不同的 class，才能是三種不同顏色", () => {
    const kinds = ["waiting", "done", "failed"] as const;
    const classNames = kinds.map((attention) => {
      const { unmount } = renderTabBar({
        tabs: [twoTabs[0], { ...twoTabs[1], attention }],
        activeId: "t1",
      });
      const cls = screen.getByRole("img", { name: /終端機/ }).className;
      unmount();
      return cls;
    });
    expect(new Set(classNames).size).toBe(3);
  });

  it("沒有 attention 時什麼都不顯示", () => {
    renderTabBar({ tabs: twoTabs, activeId: "t1" });
    expect(screen.queryByRole("img", { name: /終端機/ })).toBeNull();
  });

  it("非終端機分頁不顯示提示點", () => {
    renderTabBar({
      tabs: [twoTabs[0], { id: "m1", title: "Mail", type: "mail", attention: "waiting" }],
      activeId: "t1",
    });
    expect(screen.queryByRole("img", { name: /終端機/ })).toBeNull();
  });

  // 與既有 mail badge 的關係：兩者語意完全不同，class 必須可區分。
  // 比照本檔既有的「stays distinguishable from the unread badge」測試。
  it("與 mail 的 badge class 不相同", () => {
    const { unmount } = renderTabBar({
      tabs: [twoTabs[0], { ...twoTabs[1], attention: "failed" }],
      activeId: "t1",
    });
    const attention = screen.getByRole("img", { name: "終端機指令失敗" }).className;
    unmount();

    renderTabBar({
      tabs: [{ id: "m1", title: "Mail", type: "mail" }],
      activeId: "t1",
      mailFailedAccountCount: 1,
    });
    const mailFailure = screen.getByRole("img", { name: "1 個信箱帳號連線失敗" }).className;

    expect(attention).not.toBe(mailFailure);
  });
});
```

- [ ] **Step 2: 執行測試，確認它失敗**

Run: `npx vitest run src/components/TabBar/index.test.tsx`
Expected: FAIL。新加的 7 個 test 全紅（`Unable to find an accessible element…`），既有的 8 個仍然通過。TypeScript 也會抱怨 `Tab` 沒有 `attention` 屬性。

- [ ] **Step 3: 加入 `attention` 欄位**

在 `src/components/TabBar/index.tsx` 檔案上方的 import 區塊追加（放在 `import "./index.css";` 之前）：

```ts
import type { AttentionKind } from "../../lib/terminalAttention";
```

在 `Tab` 介面（第 24 行起）的 `aiSummary?: string;` **後面**加入：

```ts
  /** 非 active 的終端機分頁發生了值得注意的事：在側邊欄圖示上顯示一個彩色點。
   *  只存在記憶體，不進 localStorage——重開 app 後這些事件已經沒有意義。 */
  attention?: AttentionKind;
```

- [ ] **Step 4: 渲染提示點**

在 `src/components/TabBar/index.tsx` 的 `getTabIcon` 函式**後面**加入對照表：

```ts
// 顏色只對看得見的人有意義，所以每個狀態都要有自己的文字說明。
function attentionLabel(kind: AttentionKind, t: ReturnType<typeof useLocale>["t"]): string {
  switch (kind) {
    case "waiting": return t.terminal_attention_waiting_label;
    case "done": return t.terminal_attention_done_label;
    case "failed": return t.terminal_attention_failed_label;
  }
}
```

在 `.aiterm-tab-icon` 這個 span 內（第 186 行 mail connection badge 的 `)}` 之後、第 187 行 `</span>` 之前）插入：

```tsx
              {/* 錨在圖示右下角。mail 的兩個 badge 只出現在 mail 分頁、
                  這個只出現在 terminal 分頁，位置不會互撞。 */}
              {tab.type === "terminal" && tab.attention && (
                <span
                  className={`terminal-attention-badge terminal-attention-badge--${tab.attention}`}
                  role="img"
                  aria-label={attentionLabel(tab.attention, t)}
                />
              )}
```

- [ ] **Step 5: 加入樣式**

在 `src/components/TabBar/index.css` **檔案最末端**追加：

```css
/* 終端機提示點 — 錨在圖示右下角，與 mail unread（右上）分開。
   語意給看得見的人是「顏色 + 形狀」，給讀屏器是 aria-label；點本身不含文字。 */
.terminal-attention-badge {
  position: absolute;
  bottom: -5px;
  right: -7px;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  border: 1.5px solid var(--bg-secondary, #151515);
  pointer-events: none;
}

/* 只有「等待輸入」會動：它是唯一需要使用者「現在」動手的狀態。 */
.terminal-attention-badge--waiting {
  background: #f59e0b;
  box-shadow: 0 0 6px rgba(245, 158, 11, 0.6);
  animation: terminal-attention-pulse 1.6s ease-in-out infinite;
}

.terminal-attention-badge--done {
  background: #22c55e;
}

/* 方形而非圓形。紅綠是最典型的色盲失效組合，約 8% 的男性分不出
   #22c55e 與 #ef4444——而「成功 vs 失敗」正是本功能最重要的區別，
   aria-label 幫不了他們（看得見、不用讀屏器、也不會有 tooltip）。
   同一支檔案的 .mail-connection-badge 早就不信任單靠顏色，
   它在紅點裡放了一個字面的 "!"。 */
.terminal-attention-badge--failed {
  background: #ef4444;
  border-radius: 2px;
  box-shadow: 0 0 6px rgba(239, 68, 68, 0.5);
}

@keyframes terminal-attention-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.35; }
}

@media (prefers-reduced-motion: reduce) {
  .terminal-attention-badge--waiting {
    animation: none;
  }
}
```

- [ ] **Step 6: 執行測試，確認它通過**

Run: `npx vitest run src/components/TabBar/index.test.tsx`
Expected: PASS，15 個 test 全綠（既有 8 + 新增 7）。

- [ ] **Step 7: 型別檢查**

Run: `npx tsc -b`
Expected: 沒有輸出、exit 0。

- [ ] **Step 8: Commit**

```bash
git add src/components/TabBar/index.tsx src/components/TabBar/index.css src/components/TabBar/index.test.tsx
git commit -m "feat(tabbar): 終端機分頁的三色提示點"
```

---

## Task 4: 抽出共用的通知權限模組

`useMailSync.ts:62` 的 `ensureNotificationPermission` 用 hook 內的 ref 做記憶化。該處註解（`useMailSync.ts:52-61`）明講記憶化的目的是避免同時跳出多個權限請求、以及重複打擾已經拒絕過的使用者。

如果本功能另寫一份獨立的記憶化，就會製造出那條註解正在防的 bug（兩個獨立的 promise 各自請求一次）。所以抽成模組層級的單一 promise，兩邊共用。

**這是行為保持不變的搬移**，不是重構練習：只把函式搬到共用模組，並讓兩處引用它。

**但有一個測試層面的真實後果，必須一併處理。** 現在的記憶化是 hook 內的 `useRef`，所以每個 hook 實例都有全新的快取；`useMailSync.test.ts` 第 174-229 行那批「權限被拒絕」的測試正是靠這點才成立——它們各自 mount 一次、各自重新走一遍權限流程。改成模組層級的單一 promise 之後，快取會跨測試存活：第 54 行那個測試會先把 `true` 快取起來，後面那些測試就再也不會呼叫 `requestPermission`。

因此模組必須提供一個測試用的重設函式。**不要**改成 mock 掉 `../lib/notifyPermission` 來閃過——那會把整批專門驗證權限處理的測試架空，等於用刪測試來讓測試變綠。

**而且真正危險的方向不是「變紅」。** 上面描述的是看得見的失敗：後面的 test 走不到 `requestPermission`，紅燈會告訴你。相反方向才難察覺——先跑的 test 把 `granted = true` 快取起來之後，後面任何斷言「有發出通知」的 test **都會通過**，即使權限處理已經壞掉。它會為了錯誤的理由變綠，而且沒有任何訊號。所以「每個會觸發通知的測試檔都要在 `beforeEach` 重設」是硬性要求，不是整潔問題。

（跨檔案不用擔心：vitest 預設 `isolate: true`，每個測試檔有獨立的 module registry。危險只在單一檔案內的 test 順序。）

**Files:**
- Create: `src/lib/notifyPermission.ts`
- Modify: `src/hooks/useMailSync.ts:3`、`:27`、`:52-72`、`:131`
- Test: `src/hooks/useMailSync.test.ts`（`beforeEach` 加入重設）

- [ ] **Step 1: 建立共用模組**

建立 `src/lib/notifyPermission.ts`：

```ts
import { isPermissionGranted, requestPermission } from "@tauri-apps/plugin-notification";

// 模組層級的單一 promise，跨所有呼叫端共用。每個通知來源各自記憶化的話，
// 一次啟動就會疊出多個並行的 requestPermission() 提示，並且重複打擾
// 已經拒絕過的使用者——這正是這個記憶化存在的理由。
let pending: Promise<boolean> | null = null;

/**
 * 取得通知權限，整個 app 生命週期內只實際請求一次。
 *
 * 桌面端注意事項（tauri-plugin-notification 2.3.3，desktop.rs:61-66）：
 * permission_state() 與 request_permission() 在桌面端一律回傳 Granted，
 * 完全不會去問 OS。這段保留是因為它是外掛的正式 API 且在行動端是必要的，
 * 但在 macOS / Windows / Linux 上它永遠 resolve true。
 */
export function ensureNotificationPermission(): Promise<boolean> {
  pending ??= isPermissionGranted()
    .then((granted) => granted || requestPermission().then((p) => p === "granted"))
    .catch((err) => {
      console.error("[notify] notification permission check failed:", err);
      // 不要把一次暫時性的 IPC 失敗記憶成「被拒絕」。
      pending = null;
      return false;
    });
  return pending;
}

/**
 * 只給測試用：清掉快取的權限結果。
 *
 * 這個快取是模組層級的，所以在同一個測試檔內會跨 test 存活——一個先跑的
 * test 快取了「已授權」，後面驗證「被拒絕」路徑的 test 就永遠不會再走到
 * requestPermission()。每個 test 都必須從乾淨狀態開始。
 */
export function resetNotificationPermissionForTests(): void {
  pending = null;
}
```

- [ ] **Step 2: 讓 `useMailSync` 改用它**

在 `src/hooks/useMailSync.ts`：

1. 第 3 行的 import 改成（拿掉 `isPermissionGranted` 與 `requestPermission`，只留 `sendNotification`）：

```ts
import { sendNotification } from "@tauri-apps/plugin-notification";
import { ensureNotificationPermission } from "../lib/notifyPermission";
```

2. 刪掉第 27 行的 `const permissionRef = useRef<Promise<boolean> | null>(null);`
3. 刪掉第 52-72 行整段 `ensureNotificationPermission` 的註解與 `useCallback` 定義（那些註解已經搬進新模組）。
4. 第 131 行的 effect 依賴陣列 `}, [refreshUnread, ensureNotificationPermission]);` 改成 `}, [refreshUnread]);`（現在它是模組層級的匯入，不會變動）。

第 99 行的 `if (await ensureNotificationPermission()) {` 不用改——名稱相同，現在解析到匯入的版本。

- [ ] **Step 3: 執行 mail 測試，親眼看到快取洩漏造成的失敗**

Run: `npx vitest run src/hooks/useMailSync.test.ts`
Expected: **FAIL**。至少第 183 行（`expect(requestPermission).toHaveBeenCalledTimes(1)`）與第 214、228 行（`expect(requestPermission).toHaveBeenCalled()`）會逾時失敗——因為更早的 test 已經把「已授權」快取進模組層級的 `pending`。

先看到這個失敗很重要：它證明了 Step 4 要加的重設函式是必要的，而不是憑空多出來的程式碼。

`useMailSync.test.ts` 第 14-17 行 mock 的是 `@tauri-apps/plugin-notification` 這個外掛層，新模組也是從那裡匯入，所以 mock 本身仍然有效、不需要改動。要改的只有測試之間的狀態隔離。

- [ ] **Step 4: 在測試裡重設快取**

在 `src/hooks/useMailSync.test.ts` 第 26 行的 import 區塊追加：

```ts
import { resetNotificationPermissionForTests } from "../lib/notifyPermission";
```

在 `beforeEach`（第 30 行起）的 `vi.clearAllMocks();` **後面**加入：

```ts
    // 權限快取是模組層級的，會跨 test 存活。不重設的話，先跑的 test
    // 快取了「已授權」，後面驗證「被拒絕」路徑的 test 就走不到 requestPermission。
    resetNotificationPermissionForTests();
```

- [ ] **Step 5: 執行 mail 測試，確認全部通過**

Run: `npx vitest run src/hooks/useMailSync.test.ts`
Expected: PASS，全部既有 test 通過。這證明搬移之後 `useMailSync` 的行為完全沒變。

- [ ] **Step 6: 型別檢查與全套測試**

Run: `npx tsc -b && npx vitest run`
Expected: 兩者皆 exit 0。

- [ ] **Step 7: Commit**

```bash
git add src/lib/notifyPermission.ts src/hooks/useMailSync.ts src/hooks/useMailSync.test.ts
git commit -m "refactor(notify): 抽出共用的通知權限取得"
```

---

## Task 5: `useTerminalBlocks` 回報指令結果

**Files:**
- Modify: `src/hooks/useTerminalBlocks.ts:38-43`（函式簽章）、`:187-190`（OSC handler）、`:205`（依賴陣列）

- [ ] **Step 1: 加入 `onCommandSettled` 參數**

`src/hooks/useTerminalBlocks.ts` 第 38-43 行的簽章改成：

```ts
export function useTerminalBlocks(
  sessionId: string,
  term: Terminal | null,
  cwdRef?: React.RefObject<string>,
  onLiveClear?: () => void,
  /** 每次有指令跑完就呼叫，帶上它的 exit code。給側邊欄提示點用。
   *  必須是穩定的參考（useCallback 空依賴或 ref 橋接）——它進了下面
   *  OSC handler effect 的依賴陣列，每次換身分都會重新註冊 handler。 */
  onCommandSettled?: (exitCode: number) => void,
): UseTerminalBlocksResult {
```

- [ ] **Step 2: 在 OSC handler 裡呼叫它**

第 187-190 行目前是：

```ts
          finalizeBlock(latest.id, isNaN(exitCode) ? 0 : exitCode);
        } else {
          finalizeBlock(latest.id, isNaN(exitCode) ? 0 : exitCode, { clearOnParsed: true });
        }
```

改成：

```ts
          finalizeBlock(latest.id, isNaN(exitCode) ? 0 : exitCode);
        } else {
          finalizeBlock(latest.id, isNaN(exitCode) ? 0 : exitCode, { clearOnParsed: true });
        }

        // 兩條分支（Windows/ConPTY 與其他平台）的差別只在畫面清除時機，
        // 對「指令結束了、結果是什麼」沒有影響，所以放在合流之後呼叫一次。
        onCommandSettled?.(isNaN(exitCode) ? 0 : exitCode);
```

**不要改動** `isWindows` 分支裡的任何一行——那段的延遲 clear 與 `\x0c` 重新同步是踩過坑修好的（見該處註解），時序很敏感。

- [ ] **Step 3: 更新依賴陣列**

第 205 行 `}, [term, finalizeBlock, onLiveClear, sessionId]);` 改成：

```ts
  }, [term, finalizeBlock, onLiveClear, sessionId, onCommandSettled]);
```

- [ ] **Step 4: 確認既有測試沒有壞**

Run: `npx vitest run src/hooks/useTerminalBlocks.test.ts`
Expected: PASS。新參數是選用的，既有呼叫端與測試都不用改。

- [ ] **Step 5: 型別檢查**

Run: `npx tsc -b`
Expected: 沒有輸出、exit 0。

- [ ] **Step 6: Commit**

```bash
git add src/hooks/useTerminalBlocks.ts
git commit -m "feat(terminal): useTerminalBlocks 回報指令 exit code"
```

---

## Task 6: `TerminalView` 送出 attention 事件

**Files:**
- Modify: `src/components/TerminalView.tsx:102-118`（props 介面）、`:142`（解構）、`:253-258`（`useTerminalBlocks` 呼叫）

- [ ] **Step 1: 加入 prop**

在 `src/components/TerminalView.tsx` 上方的 import 區塊追加：

```ts
import type { AttentionKind } from "../lib/terminalAttention";
```

在 `TerminalViewProps` 介面（第 102 行起）的 `onSummaryUpdate` **後面**加入：

```ts
  /** 這個分頁發生了需要使用者注意的事。TerminalView 一律回報，
   *  「這個分頁是不是 active」與「視窗有沒有 focus」都由 TerminalApp 判斷——
   *  避免那些條件在 xterm / PTY 事件的 closure 裡變 stale。 */
  onAttention?: (kind: AttentionKind) => void;
```

第 142 行的解構加上 `onAttention`：

```tsx
export function TerminalView({ isActive = true, onToggleSidebar, isSidebarOpen = true, onSessionCreated, initialCwd, initialMission, enterpriseTask, onAgentProgress, onSummaryUpdate, onAttention }: TerminalViewProps) {
```

- [ ] **Step 2: 建立穩定的 ref 橋接**

在第 251 行（`forceLiveRepaint` 的 `}, []);` 之後）、第 253 行的 `useTerminalBlocks` 呼叫**之前**插入：

```tsx
  // TerminalApp 傳進來的是 inline arrow function，每次 render 都是新身分。
  // 直接把它放進 useTerminalBlocks 的依賴會讓 OSC handler 每次 render
  // 重新註冊。橋接成 ref，對外露出一個永久穩定的 emitAttention——
  // 與本檔 submitCommandRef / beginTrackedBlockRef 同樣的理由與作法。
  const onAttentionRef = useRef(onAttention);
  useEffect(() => { onAttentionRef.current = onAttention; }, [onAttention]);

  const emitAttention = useCallback((kind: AttentionKind) => {
    onAttentionRef.current?.(kind);
  }, []);

  const handleCommandSettled = useCallback((exitCode: number) => {
    emitAttention(exitCode === 0 ? "done" : "failed");
  }, [emitAttention]);
```

- [ ] **Step 3: 接上 `useTerminalBlocks`**

第 253-258 行的呼叫加上第五個參數：

```tsx
  const { blocks, isAlternateBuffer, submitCommand, beginTrackedBlock, appendOutput, setBlockGitInfo } = useTerminalBlocks(
    sessionId,
    termState,
    lastCwdRef,
    forceLiveRepaint,
    handleCommandSettled,
  );
```

- [ ] **Step 4: 接上 bell**

在 Step 2 插入的那段**後面**再加一個 effect：

```tsx
  // 終端機的 bell（\x07）。CLI 工具停下來等使用者回答時多半會敲一次——
  // 這是全螢幕 TUI（Claude Code、vim、lazygit）執行期間唯一可用的訊號，
  // 因為 shell 在那段期間把整個 TUI 視為「一個還在跑的指令」，
  // OSC 133 D 要等它退出才會發出。
  useEffect(() => {
    if (!termState) return;
    const disposable = termState.onBell(() => emitAttention("waiting"));
    return () => disposable.dispose();
  }, [termState, emitAttention]);
```

- [ ] **Step 5: 型別檢查**

Run: `npx tsc -b`
Expected: 沒有輸出、exit 0。

- [ ] **Step 6: 確認終端機既有測試沒有壞**

Run: `npx vitest run src/components/TerminalView.searchCascade.test.tsx src/components/TerminalBlockCard.test.tsx`
Expected: PASS。

- [ ] **Step 7: Commit**

```bash
git add src/components/TerminalView.tsx
git commit -m "feat(terminal): 從 bell 與 exit code 送出 attention 事件"
```

---

## Task 7: `TerminalApp` 分派、清除與通知

**Files:**
- Modify: `src/components/TerminalApp.tsx`（import、焦點追蹤、清除 effect、`onAttention` 接線）

- [ ] **Step 1: 加入 import**

在 `src/components/TerminalApp.tsx` 上方 import 區塊追加：

```ts
import { getCurrentWindow } from "@tauri-apps/api/window";
import { sendNotification } from "@tauri-apps/plugin-notification";
import { ensureNotificationPermission } from "../lib/notifyPermission";
import { routeAttention, type AttentionKind } from "../lib/terminalAttention";
```

- [ ] **Step 2: 追蹤視窗焦點**

在第 92 行 `const [lastTerminalPtyId, setLastTerminalPtyId] = useState<string>("");` **之前**插入：

```tsx
  // 視窗焦點放在 ref 而非 state：它只被事件 callback 讀取，不影響任何渲染，
  // 用 state 會讓每次切換視窗都重繪整個 app。初始值樂觀設為 true，
  // 這樣在 isFocused() 回來之前不會誤發通知。
  const windowFocusedRef = useRef(true);
  useEffect(() => {
    const win = getCurrentWindow();
    let unlisten: (() => void) | undefined;
    win.isFocused().then((f) => { windowFocusedRef.current = f; }).catch(() => {});
    win.onFocusChanged(({ payload }) => { windowFocusedRef.current = payload; })
      .then((u) => { unlisten = u; })
      .catch(() => {});
    return () => unlisten?.();
  }, []);
```

- [ ] **Step 3: 切到某分頁時清掉它的提示點**

在 Step 2 插入的內容**後面**再加一個 effect：

```tsx
  // 使用者選定的規則：切過去就算讀取過了。所以當前 active 的分頁
  // 永遠不會有提示點。用函式式 setTabs 並在無事可做時回傳同一個陣列，
  // 避免每次切分頁都製造新的 tabs 參考。
  useEffect(() => {
    setTabs((prev) =>
      prev.some((t) => t.id === activeId && t.attention)
        ? prev.map((t) => (t.id === activeId ? { ...t, attention: undefined } : t))
        : prev
    );
  }, [activeId]);
```

- [ ] **Step 4: 寫分派函式**

在 Step 3 的 effect **後面**加入：

```tsx
  // 一個 attention 事件 → 兩個互相獨立的決定。規則本體在 routeAttention
  // （src/lib/terminalAttention.ts），那裡有單元測試釘住「提示點看分頁、
  // 通知看視窗焦點」這兩者不能被合併。
  const handleAttention = useCallback((tabId: string, tabTitle: string, kind: AttentionKind) => {
    const { badge, notify } = routeAttention({
      isActiveTab: activeIdRef.current === tabId,
      windowFocused: windowFocusedRef.current,
      kind,
    });

    if (badge) {
      setTabs((prev) => prev.map((t) => (t.id === tabId ? { ...t, attention: badge } : t)));
    }

    if (notify) {
      const body = kind === "waiting" ? t.terminal_notify_waiting : t.terminal_notify_failed;
      ensureNotificationPermission().then((granted) => {
        if (granted) sendNotification({ title: tabTitle, body });
      }).catch(() => { /* 通知失敗不是使用者能處理的事 */ });
    }
  }, [t]);
```

`t` 已經存在——`TerminalApp.tsx:56` 就有 `const { t } = useLocale();`。直接用，不要再取一次。

- [ ] **Step 5: 接到 `TerminalView`**

在 `TerminalView` 的渲染處，於第 383 行開始的 `onSummaryUpdate={(summary) => { … }}` 這整個 prop **後面**（也就是 `/>` 之前）加入：

```tsx
                  onAttention={(kind) => handleAttention(tab.id, tab.title, kind)}
```

- [ ] **Step 6: 型別檢查與全套測試**

Run: `npx tsc -b && npx vitest run`
Expected: 兩者皆 exit 0。

- [ ] **Step 7: Lint**

Run: `npx eslint src/lib/terminalAttention.ts src/lib/notifyPermission.ts src/components/TabBar/index.tsx src/components/TerminalView.tsx src/components/TerminalApp.tsx src/hooks/useTerminalBlocks.ts src/hooks/useMailSync.ts`
Expected: 沒有輸出、exit 0。

> **不要用 `npm run lint` 當作通過標準。** 這個 repo 的全庫 lint 現況本來就是紅的（91 個問題／71 個 error），全部落在與本功能無關的檔案（`CodeAssistantView`、`CommandBookmarks`、`DocConverterView`、`FileExplorer` 等）。那是既有債務，不屬於本功能的範圍，也不該由本功能順手清理。只檢查自己動過的檔案。

- [ ] **Step 8: Commit**

```bash
git add src/components/TerminalApp.tsx
git commit -m "feat(tabbar): 分派終端機 attention 到提示點與桌面通知"
```

---

## Task 8: 手動驗證

自動測試涵蓋 `routeAttention` 的規則與 `TabBar` 的渲染。**訊號實際會不會來**（xterm bell、OSC 133）只能在真的終端機裡驗證。

**Files:** 無（純驗證）

- [ ] **Step 1: 啟動 app**

Run: `npm run tauri:dev`

> 若 `src-tauri/binaries/` 是空的，先跑對應平台的 `scripts/setup-uv-{mac,linux,win}` 腳本。`tauri-build` 的 `build.rs` 會在**編譯期**檢查每個 `externalBin` 都存在，缺了連 `cargo check` 都會失敗。

- [ ] **Step 2: 驗證「完成」（綠點）**

1. 開兩個終端機分頁。
2. 在分頁 2 執行 `sleep 5`（Windows PowerShell：`Start-Sleep 5`）。
3. 立刻切到分頁 1。
4. 5 秒後，側邊欄的分頁 2 圖示右下角應出現**綠點**。
5. 切到分頁 2 → 綠點消失。

- [ ] **Step 3: 驗證「失敗」（紅點）**

1. 在分頁 2 執行 `sleep 3; exit 1`（Windows：`Start-Sleep 3; exit 1`）。
2. 立刻切到分頁 1。
3. 應出現**紅色方點**（不是綠色圓點）。形狀差異是這裡的重點：`done` 與 `failed` 不能只靠紅綠區分，那組顏色對約 8% 的男性無效。截圖或瞇眼確認方形真的看得出來——`border-radius: 2px` 在 8px 見方上很容易被誤設成看起來仍像圓的。

- [ ] **Step 4: 驗證「等待輸入」（橘色脈動點）**

1. 在分頁 2 執行 `sleep 3; printf '\a'`（Windows PowerShell：`Start-Sleep 3; [console]::beep()` 不會走 PTY，改用 `Start-Sleep 3; Write-Host "`a" -NoNewline`）。
2. 立刻切到分頁 1。
3. 應出現**會脈動的橘點**。

- [ ] **Step 5: 驗證 active 分頁不會有點**

在**當前正在看的**分頁執行 `printf '\a'` 與 `exit 1`。側邊欄不該出現任何點。

- [ ] **Step 6: 驗證通知條件（僅限正式 build）**

> **這一步在 `tauri:dev` 下驗證不了。** 依 `useMailSync.ts:104-111` 的說明，dev 模式下通知會以 `com.apple.Terminal` 的身分送出，且 `show()` 的結果被丟棄、失敗完全不可觀測。必須跑 `npm run tauri:build`（貢獻者用 `npm run tauri:build -- --no-sign`）並開啟產出的 bundle。

1. 在分頁 2 執行 `sleep 8; printf '\a'`，然後**切換到別的 app**（讓 AITerm 失焦）。
2. 應收到桌面通知，標題為分頁名稱、內文為「正在等待你的回應」。
3. 重複但改成 `sleep 8` 單獨執行（結束時 exit code 0）→ **不應**有通知（`done` 不發通知）。
4. 讓 AITerm 保持在前景並重複第 1 步 → **不應**有通知（只有側邊欄的點）。

- [ ] **Step 7: 驗證 bell 的誤報率（這是本設計唯一會誤報的訊號）**

`onBell` 對**每一個** `\x07` 都會觸發，不只是「CLI 在問你問題」。已知的無辜來源至少有：bash readline 在 tab 補全有歧義時的嗶聲、zsh 走到歷史盡頭的嗶聲。

設計文件的原則是「寧可漏報，不可誤報」，所以這一項要實測：

1. 在**預設的 bash**（不是你自訂過的 shell）開一個分頁，切到別的分頁。
2. 在該分頁打幾個字後連按兩次 Tab 製造歧義補全，讓它嗶。
3. 看側邊欄是否亮起橘點。

實務上這個誤報對提示點影響有限（你當時人就在那個分頁打字，而 active 分頁不會亮點），對通知更小（要打字就代表視窗有 focus，而通知只在失焦時發）。但如果背景腳本會嗶，橘點就會無故亮起。若實測發現吵到無法接受，回頭調整的方向是「只在 alternate buffer 中才把 bell 視為 waiting」，不是加靜默啟發式。

- [ ] **Step 8: 記錄結果**

把每一步的實際結果寫下來。若 Step 4 的 bell 在你的 shell 上沒有觸發，先確認該 shell 有沒有把 bell 靜音（zsh 的 `unsetopt beep`、`.inputrc` 的 `set bell-style none`），再判定是實作問題。

---

## 已知限制（實作時不要試圖「修好」）

- **全螢幕 TUI 期間沒有 OSC 133 訊號。** Claude Code、vim、lazygit 執行期間，shell 只把它們視為一個還在跑的指令；`D` 要等程式退出才發出。那段時間唯一的訊號是 bell。
- **因此「Claude Code 問問題時會不會亮橘燈」取決於它自己有沒有敲 bell**，那是該工具的通知設定，AITerm 控制不了。第一版接受這個漏報。
- **不要為了補上這個漏報而加輸出靜默的啟發式判斷。** 設計文件已經評估並否決：跑很久的編譯、卡住的下載、開著沒動的 vim 都會被誤判成「等你回答」，而誤判過幾次之後使用者就會永久無視這個提示。原則是**寧可漏報，不可誤報**。

## 跨平台

三種訊號（xterm bell、OSC 133、Tauri 通知）在 macOS / Windows / Linux 上行為一致，沒有平台專屬 API。Windows 的 OSC 133 路徑走既有的 ConPTY 分支，本計畫只在兩條分支合流之後多呼叫一個 callback，不改變該分支的時序。
