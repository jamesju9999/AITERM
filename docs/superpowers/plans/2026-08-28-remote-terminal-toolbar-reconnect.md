# 遠端終端機工具列：品牌樣式、換行、就地重新連線 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在遠端終端機工具列加入 AI 面板同款的漸層 sparkle 品牌樣式、允許連線資訊文字換行，並新增一顆「連線」按鈕，讓使用者可以在同一個分頁裡直接切換去連另一台主機，不需要每次都跑一次 ADD TAB 流程。

**Architecture:** `RemoteTerminalView` 的「AITerm」文字前綴改用沿用 `AiPanel` 既有的漸層文字 CSS 技巧；新增一顆「連線」按鈕透過新的 `onConnectClick` prop 把點擊事件往上交給 `TerminalApp.tsx`；`TerminalApp.tsx` 新增 `reconnectTabId` state 記住是哪個分頁要求重新連線，`ConnectDialog.onConnected` 依此決定是開新分頁還是更新既有分頁；`<RemoteTerminalView>` 改用 `key={tab.remoteConnId}`，讓連線切換時 React 自動完整重新掛載，不需要手動清空內部十幾個 state。

**Tech Stack:** TypeScript / React 19（`RemoteTerminalView/index.tsx`、`TerminalApp.tsx`）、CSS（沿用 `AiPanel` 的漸層文字技巧）、i18n（`src/lib/i18n.ts`）、Vitest。

**參考設計文件：** `docs/superpowers/specs/2026-08-28-remote-terminal-toolbar-reconnect-design.md`

---

### Task 1：品牌樣式與換行

**Files:**
- Modify: `src/components/RemoteTerminalView/index.tsx`
- Test: `src/components/RemoteTerminalView/index.test.tsx`

- [ ] **Step 1: 寫失敗的測試**

在 `src/components/RemoteTerminalView/index.test.tsx` 裡，緊接既有的 `"全螢幕程式即時窗格的高度跟著主控端實際列數變化，不是無條件撐滿容器"` 測試之後（`disconnect timing (StrictMode dev-mode trap)` 這個 describe block 之前），新增：

```tsx
  it("連線資訊文字的「AITerm」開頭套用漸層品牌樣式", async () => {
    render(<RemoteTerminalView tabId="t1" connId="c21" sas="2121" isActive hostLabel="10.10.41.1:50281" />);

    const brand = await screen.findByText("✨ AITerm");
    expect(brand).toHaveStyle({
      background: "var(--accent-gradient)",
      WebkitTextFillColor: "transparent",
    });
  });
```

- [ ] **Step 2: 執行測試，確認失敗**

Run: `npx vitest run src/components/RemoteTerminalView/index.test.tsx -t "連線資訊文字的「AITerm」開頭套用漸層品牌樣式"`
Expected: FAIL——目前畫面上是純文字 `AITerm`（沒有 `✨` 前綴，也不是獨立的 DOM 節點），`findByText("✨ AITerm")` 會找不到元素而逾時。

- [ ] **Step 3: 加上漸層品牌樣式與換行**

找到 `src/components/RemoteTerminalView/index.tsx` 裡：

```tsx
      <div className="aiterm-status" data-tauri-drag-region>
        <span className="aiterm-status-left" data-tauri-drag-region>
          AITerm · {t.remote_terminal_tab} {hostLabel} · {connectionStatusText(t, phase, elapsedMs)}
        </span>
```

改成：

```tsx
      <div className="aiterm-status" data-tauri-drag-region>
        {/* whiteSpace: "normal" 只是把「這裡本來就允許換行、不要有人以後
            幫 .aiterm-status-left 加 nowrap」這個意圖寫明白——瀏覽器對
            這個 class 的預設值本來就是 normal，這行不是修正一個既有的
            截斷問題。 */}
        <span className="aiterm-status-left" data-tauri-drag-region style={{ whiteSpace: "normal" }}>
          {/* 沿用 AiPanel/index.tsx 既有的漸層文字技巧（同一個
              var(--accent-gradient) CSS 變數、同一套 WebkitBackgroundClip
              /WebkitTextFillColor 組合），不重新設計一套新樣式。 */}
          <span style={{ background: "var(--accent-gradient)", WebkitBackgroundClip: "text", WebkitTextFillColor: "transparent", fontWeight: 700 }}>
            ✨ AITerm
          </span>{" "}
          · {t.remote_terminal_tab} {hostLabel} · {connectionStatusText(t, phase, elapsedMs)}
        </span>
```

- [ ] **Step 4: 執行測試，確認通過，且沒有弄壞既有測試**

Run: `npx vitest run src/components/RemoteTerminalView/index.test.tsx`
Expected: 全數通過。

- [ ] **Step 5: 確認 tsc 沒有報錯**

Run: `npx tsc -b`
Expected: 無錯誤輸出，結束碼 0。

- [ ] **Step 6: Commit**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM-full-parity
git add src/components/RemoteTerminalView/index.tsx src/components/RemoteTerminalView/index.test.tsx
git commit -m "$(cat <<'EOF'
feat(remote-terminal): 工具列加上漸層品牌樣式、允許連線資訊換行

沿用 AiPanel 既有的漸層文字技巧（var(--accent-gradient) + WebkitBackgroundClip
/WebkitTextFillColor），把工具列開頭的純文字「AITerm」換成「✨ AITerm」，
視覺上更醒目也多佔一點高度。同時明確允許連線資訊文字換行，不再被
限制在單行內。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2：新增「連線」按鈕

**Files:**
- Modify: `src/lib/i18n.ts`
- Modify: `src/components/RemoteTerminalView/index.tsx`
- Test: `src/components/RemoteTerminalView/index.test.tsx`

- [ ] **Step 1: 新增翻譯鍵（zh-TW）**

找到 `src/lib/i18n.ts` 裡：

```ts
    remote_terminal_toolbar_ended: "連線已結束",
    // 遠端終端機共享——主控端
```

改成：

```ts
    remote_terminal_toolbar_ended: "連線已結束",
    remote_terminal_toolbar_connect_button: "連線",
    // 遠端終端機共享——主控端
```

- [ ] **Step 2: 新增翻譯鍵（en）**

找到 `src/lib/i18n.ts` 裡：

```ts
    remote_terminal_toolbar_ended: "Connection ended",
    // Remote terminal sharing — host side
```

改成：

```ts
    remote_terminal_toolbar_ended: "Connection ended",
    remote_terminal_toolbar_connect_button: "Connect",
    // Remote terminal sharing — host side
```

- [ ] **Step 3: 確認既有的語系同步測試通過**

Run: `npx vitest run src/lib/i18n.remoteTerminal.test.ts`
Expected: 全數通過（`"keeps the two locales in sync for sharing strings"` 這個既有測試會自動檢查兩邊鍵是否對稱，不需要為這件事另外寫新測試）。

- [ ] **Step 4: `Props` 新增必填的 `onConnectClick`**

找到 `src/components/RemoteTerminalView/index.tsx` 裡：

```tsx
  /**
   * 連線當下輸入的「host:port」字串，跟視窗標題「遠端終端機：10.10.41.1:
   * 50281」同一份資料（`ConnectDialog.onConnected` 回傳的 `hostLabel`，
   * 已經存在 `tab.remoteHostLabel`）。工具列的位址文字用它。
   *
   * 選填、預設空字串，不是必填：這個 prop 是這次新增的，選填可以讓既有
   * 測試呼叫端不用全部跟著改。
   */
  hostLabel?: string;
}
```

改成：

```tsx
  /**
   * 連線當下輸入的「host:port」字串，跟視窗標題「遠端終端機：10.10.41.1:
   * 50281」同一份資料（`ConnectDialog.onConnected` 回傳的 `hostLabel`，
   * 已經存在 `tab.remoteHostLabel`）。工具列的位址文字用它。
   *
   * 選填、預設空字串，不是必填：這個 prop 是這次新增的，選填可以讓既有
   * 測試呼叫端不用全部跟著改。
   */
  hostLabel?: string;
  /**
   * 使用者點了工具列的「連線」按鈕。由 `TerminalApp.tsx` 提供：開啟
   * `ConnectDialog`，並記住是「這個分頁」要求重新連線——連線成功後
   * `TerminalApp.tsx` 會更新這個分頁的 remoteConnId/remoteSas/
   * remoteHostLabel，不會開新分頁（見 Task 3）。
   *
   * 必填、沒有預設值：這顆按鈕點了沒反應會很奇怪，沒有有意義的
   * no-op 預設可以退回。
   */
  onConnectClick: () => void;
}
```

找到：

```tsx
export function RemoteTerminalView({ tabId, connId, sas, isActive, hostLabel = "" }: Props) {
```

改成：

```tsx
export function RemoteTerminalView({ tabId, connId, sas, isActive, hostLabel = "", onConnectClick }: Props) {
```

- [ ] **Step 5: 確認 tsc 報錯——這是預期中的，用來找出所有需要補 prop 的既有測試呼叫點**

Run: `npx tsc -b`
Expected: FAIL，報 `src/components/RemoteTerminalView/index.test.tsx` 裡總共 19 處 `<RemoteTerminalView ... />` 呼叫缺少必填的 `onConnectClick` prop。這是這一步刻意造成的——先讓型別檢查明確列出所有要修的地方，比自己用眼睛找不容易漏掉。

- [ ] **Step 6: 幫測試檔案裡所有既有呼叫點補上 `onConnectClick`**

`src/components/RemoteTerminalView/index.test.tsx` 裡所有 `render(<RemoteTerminalView ... />)` 呼叫，結尾都是下面兩種寫法之一（已經逐一確認過，檔案裡沒有第三種寫法）：

寫法一（15 處）：結尾是 `isActive />`，改成 `isActive onConnectClick={vi.fn()} />`——**用支援 replace_all 的編輯方式，把檔案裡所有** `isActive />` **取代成** `isActive onConnectClick={vi.fn()} />`。

寫法二（4 處）：結尾是 `hostLabel="10.10.41.1:50281" />`，改成 `hostLabel="10.10.41.1:50281" onConnectClick={vi.fn()} />`——**同樣用 replace_all，把檔案裡所有** `hostLabel="10.10.41.1:50281" />` **取代成** `hostLabel="10.10.41.1:50281" onConnectClick={vi.fn()} />`。

這兩個字串在檔案裡除了 `RemoteTerminalView` 的呼叫點以外沒有其他地方會用到（已確認過），全域取代是安全的。取代完之後，確認總共新增了 19 個 `onConnectClick={vi.fn()}`：

Run: `grep -c "onConnectClick={vi.fn()}" src/components/RemoteTerminalView/index.test.tsx`
Expected: `19`

- [ ] **Step 7: 確認 tsc 通過**

Run: `npx tsc -b`
Expected: 無錯誤輸出，結束碼 0。這一步只是把 Step 5 的必填 prop 缺口補齊，還沒有新增任何行為，先在這裡確認乾淨的基準點，接下來才開始 TDD 新按鈕本身的行為。

- [ ] **Step 8: 執行既有測試，確認都還是綠燈**

Run: `npx vitest run src/components/RemoteTerminalView/index.test.tsx`
Expected: 全數通過（這一步純粹是型別層面的必填 prop 補齊，不改變任何既有行為，所有既有測試都不該受影響）。

- [ ] **Step 9: 寫失敗的測試——新按鈕本身**

在 `src/components/RemoteTerminalView/index.test.tsx` 裡，緊接 Task 1 新增的「連線資訊文字的「AITerm」開頭套用漸層品牌樣式」測試之後，新增：

```tsx
  it("點「連線」按鈕呼叫 onConnectClick", async () => {
    const onConnectClick = vi.fn();
    render(
      <RemoteTerminalView
        tabId="t1"
        connId="c22"
        sas="2222"
        isActive
        hostLabel="10.10.41.1:50281"
        onConnectClick={onConnectClick}
      />,
    );

    const connectBtn = await screen.findByTitle(/^連線$|^Connect$/);
    await userEvent.click(connectBtn);

    expect(onConnectClick).toHaveBeenCalledTimes(1);
  });
```

**注意**：這個測試自己傳入真正的 `onConnectClick` mock（不是 Step 6 補的 `vi.fn()` 佔位），因為這個測試就是要驗證這顆按鈕真的有呼叫它。

- [ ] **Step 10: 執行測試，確認失敗**

Run: `npx vitest run src/components/RemoteTerminalView/index.test.tsx -t "點「連線」按鈕呼叫 onConnectClick"`
Expected: FAIL——目前工具列右側完全沒有「連線」按鈕，`findByTitle` 會逾時找不到元素。

- [ ] **Step 11: import `LinkIcon`**

找到 `src/components/RemoteTerminalView/index.tsx` 檔案最上面：

```tsx
import { SparklesIcon } from "../Icons";
```

改成：

```tsx
import { LinkIcon, SparklesIcon } from "../Icons";
```

- [ ] **Step 12: 加入「連線」按鈕，位置在「指令書籤」之前**

找到：

```tsx
        <span style={{ display: "flex", alignItems: "center", gap: "8px" }}>
          <button
            className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
            title={t.term_bookmark_tooltip}
            onClick={(e) => {
              e.stopPropagation();
              setBookmarksOpen(true);
            }}
            style={{ display: "flex", alignItems: "center", gap: "6px" }}
          >
            <span>{t.bookmarks_title}</span>
          </button>
```

改成：

```tsx
        <span style={{ display: "flex", alignItems: "center", gap: "8px" }}>
          <button
            className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
            title={t.remote_terminal_toolbar_connect_button}
            onClick={(e) => {
              e.stopPropagation();
              onConnectClick();
            }}
            style={{ display: "flex", alignItems: "center", gap: "6px" }}
          >
            <LinkIcon size={14} />
            <span>{t.remote_terminal_toolbar_connect_button}</span>
          </button>
          <button
            className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
            title={t.term_bookmark_tooltip}
            onClick={(e) => {
              e.stopPropagation();
              setBookmarksOpen(true);
            }}
            style={{ display: "flex", alignItems: "center", gap: "6px" }}
          >
            <span>{t.bookmarks_title}</span>
          </button>
```

- [ ] **Step 13: 執行測試，確認新測試通過，且沒有弄壞既有測試**

Run: `npx vitest run src/components/RemoteTerminalView/index.test.tsx`
Expected: 全數通過。

- [ ] **Step 14: 確認 tsc 沒有報錯**

Run: `npx tsc -b`
Expected: 無錯誤輸出，結束碼 0。

- [ ] **Step 15: Commit**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM-full-parity
git add src/lib/i18n.ts src/components/RemoteTerminalView/index.tsx src/components/RemoteTerminalView/index.test.tsx
git commit -m "$(cat <<'EOF'
feat(remote-terminal): 工具列新增「連線」按鈕

位置在「指令書籤」左側，圖示用既有、目前沒地方用到的 LinkIcon。點下去
只負責呼叫新的 onConnectClick prop——實際「跳出對話框、連線成功後更新
這個分頁」的邏輯在 TerminalApp.tsx（下一個 task），這個元件本身不知道
連線細節，只是把使用者的點擊往上交出去。

onConnectClick 是必填 prop（沒有有意義的預設值可以退回），這次順便
補齊測試檔案裡所有既有呼叫點缺少的這個 prop。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3：`TerminalApp.tsx` 就地重新連線邏輯

**Files:**
- Modify: `src/components/TerminalApp.tsx`
- Test: Create `src/components/TerminalApp.remoteReconnect.test.tsx`

- [ ] **Step 1: 寫失敗的測試**

建立新檔案 `src/components/TerminalApp.remoteReconnect.test.tsx`：

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";

// 跟既有的 TerminalApp.routeHintCloseGuard.test.tsx 同一套 mount probe
// 結論：這幾個底層 Tauri 入口點涵蓋 TerminalApp 掛載所需的一切。
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(() => new Promise(() => {})) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
vi.mock("@tauri-apps/api/path", () => ({ homeDir: vi.fn(() => Promise.resolve("/home/test")) }));
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
  }),
}));
vi.mock("@tauri-apps/plugin-notification", () => ({
  sendNotification: vi.fn(),
}));

// 這個測試檔案只關心 TerminalApp 怎麼管理 tabs 陣列（開新分頁 vs. 更新
// 既有分頁），不關心 RemoteTerminalView 或 ConnectDialog 各自內部的真實
// 行為（那些各自有自己的測試檔案）——兩個都 stub 成暴露必要 props 的
// 簡單元件，讓這個測試檔案可以直接觸發 TerminalApp 的狀態機邏輯，不用
// 處理一整套 share-viewer IPC/xterm mock。
type RemoteTerminalViewProps = {
  tabId: string;
  connId: string;
  hostLabel?: string;
  onConnectClick: () => void;
};
vi.mock("./RemoteTerminalView", () => ({
  RemoteTerminalView: ({ tabId, connId, hostLabel, onConnectClick }: RemoteTerminalViewProps) => (
    <div>
      <div data-testid={`remote-conn-${tabId}`}>{connId}:{hostLabel}</div>
      <button onClick={onConnectClick}>connect-button-{tabId}</button>
    </div>
  ),
}));

type ConnectDialogProps = {
  onConnected: (connId: string, sas: string, hostLabel: string) => void;
  onCancel: () => void;
};
vi.mock("./ConnectDialog", () => ({
  ConnectDialog: ({ onConnected, onCancel }: ConnectDialogProps) => (
    <div>
      <button onClick={() => onConnected("new-conn-id", "1234", "10.0.0.9:9999")}>
        fire-connected
      </button>
      <button onClick={onCancel}>fire-cancel</button>
    </div>
  ),
}));

// 同一個理由：這個測試檔案只關心「選了 remote-terminal 類型之後
// TerminalApp 怎麼分流」，不重新測 NewTabPicker 自己的清單/搜尋邏輯
// （那是它自己測試檔案的職責）。
type NewTabPickerProps = { onSelect: (type: string) => void };
vi.mock("./NewTabPicker", () => ({
  NewTabPicker: ({ onSelect }: NewTabPickerProps) => (
    <button onClick={() => onSelect("remote-terminal")}>pick-remote-terminal</button>
  ),
}));

import { TerminalApp } from "./TerminalApp";
import { LocaleProvider } from "../contexts/LocaleContext";
import { SESSION_TABS_KEY } from "../lib/sessionTabs";

beforeEach(() => {
  localStorage.clear();
  // 起始狀態：一個既有的遠端終端機分頁。還原機制（sessionTabs.ts 的
  // SavedTab）本來就不存 remoteConnId/remoteSas/remoteHostLabel（那些是
  // 執行期才有意義的欄位），所以還原出來的分頁這三個欄位都是空的——這對
  // 這裡的測試沒有影響：要驗證的是「點連線鈕、完成連線流程後，這個分頁
  // 的欄位被更新成新值」，起始值是不是空字串不影響這個行為本身。
  localStorage.setItem(
    SESSION_TABS_KEY,
    JSON.stringify([{ title: "遠端終端機：舊主機", type: "remote-terminal" }]),
  );
});

function renderApp() {
  return render(
    <LocaleProvider>
      <MemoryRouter>
        <TerminalApp />
      </MemoryRouter>
    </LocaleProvider>,
  );
}

describe("TerminalApp: 遠端終端機工具列的「連線」按鈕就地重新連線", () => {
  it("點既有分頁的連線按鈕、完成連線流程後，更新同一個分頁而不是開新分頁", async () => {
    renderApp();

    const connectButtons = await screen.findAllByText(/^connect-button-/);
    expect(connectButtons).toHaveLength(1);
    await userEvent.click(connectButtons[0]);

    const fireConnected = await screen.findByText("fire-connected");
    await userEvent.click(fireConnected);

    // 還是只有一個遠端終端機分頁的畫面——沒有多開一個。
    await screen.findByTestId(/remote-conn-/);
    const remoteViews = screen.getAllByTestId(/remote-conn-/);
    expect(remoteViews).toHaveLength(1);
    expect(remoteViews[0].textContent).toBe("new-conn-id:10.0.0.9:9999");
  });

  it("點連線按鈕後按取消，不影響後續從 ADD TAB 開新分頁的既有流程", async () => {
    renderApp();

    const connectButtons = await screen.findAllByText(/^connect-button-/);
    await userEvent.click(connectButtons[0]);

    const fireCancel = await screen.findByText("fire-cancel");
    await userEvent.click(fireCancel);

    // 對話框關閉、沒有任何分頁被更新——原本的分頁欄位維持原樣（起始值
    // 是空字串，因為 sessionTabs 還原本來就不存這幾個欄位）。
    const remoteViewsAfterCancel = screen.getAllByTestId(/remote-conn-/);
    expect(remoteViewsAfterCancel).toHaveLength(1);
    expect(remoteViewsAfterCancel[0].textContent).toBe(":");
  });

  it("沒有先點過任何分頁的連線鈕、直接從 ADD TAB 開新分頁，仍然正常開新分頁", async () => {
    // 確保這次改動沒有破壞既有的「開新分頁」路徑——這裡完全不碰任何
    // 既有分頁的連線鈕，直接走 ADD TAB → 選 remote-terminal 類型 →
    // ConnectDialog 這條路，reconnectTabId 應該從頭到尾都是 null。
    renderApp();

    const beforeCount = screen.getAllByTestId(/remote-conn-/).length;
    expect(beforeCount).toBe(1);

    await userEvent.click(await screen.findByTitle("New Tab (Ctrl+T)"));
    await userEvent.click(await screen.findByText("pick-remote-terminal"));

    const fireConnected = await screen.findByText("fire-connected");
    await userEvent.click(fireConnected);

    // 開了一個新分頁——現在應該有兩個遠端終端機分頁的畫面，原本那個
    // 分頁的內容維持原樣（起始的空字串），新分頁帶著剛才連線的內容。
    const remoteViewsAfterAdd = await screen.findAllByTestId(/remote-conn-/);
    expect(remoteViewsAfterAdd).toHaveLength(2);
    const texts = remoteViewsAfterAdd.map((el) => el.textContent).sort();
    expect(texts).toEqual([":", "new-conn-id:10.0.0.9:9999"]);
  });
});
```

- [ ] **Step 2: 執行測試，確認失敗**

Run: `npx vitest run src/components/TerminalApp.remoteReconnect.test.tsx`
Expected: FAIL——三個測試都會失敗。前兩個測試（就地更新、取消不污染）
會在等 `fire-connected`/`fire-cancel` 按鈕出現時逾時，因為
`TerminalApp.tsx` 目前呼叫 `<RemoteTerminalView>` 時完全沒有傳
`onConnectClick`，mock 元件裡的 `onClick={onConnectClick}` 會拿到
`undefined`，點擊「connect-button-」不會有任何反應，`ConnectDialog`
根本沒被打開。第三個測試（ADD TAB 開新分頁）在還沒實作 Step 3-5 之前
其實應該本來就會通過（那條路徑完全沒被這次改動碰到）——先確認它是不是
真的維持通過，若這一步它也失敗了，代表對 mock 設定的理解有誤，要先
排查清楚再繼續，不要跳過。

- [ ] **Step 3: 新增 `reconnectTabId` state**

找到 `src/components/TerminalApp.tsx` 裡：

```tsx
  const [connectOpen, setConnectOpen] = useState(false);
```

改成：

```tsx
  const [connectOpen, setConnectOpen] = useState(false);
  // 記住這次開啟 ConnectDialog，是不是某個既有遠端終端機分頁的工具列
  // 「連線」按鈕要求的——是的話，連線成功後要更新那個分頁，不是開新的。
  // null 代表這次是走 ADD TAB 的正常「開新分頁」流程。
  const [reconnectTabId, setReconnectTabId] = useState<string | null>(null);
```

- [ ] **Step 4: `<RemoteTerminalView>` 呼叫處加上 `key`／`onConnectClick`**

找到（Task 2 已經在這裡加過一個暫時的 no-op `onConnectClick` 佔位，附帶
說明它是給這個 task 用的——若實際內容跟這裡不完全一樣，先讀檔案目前的
真實內容，這個佔位的存在本身不影響下面的取代邏輯，整段一起換掉即可）：

```tsx
              ) : tab.type === "remote-terminal" ? (
                <RemoteTerminalView
                  tabId={tab.id}
                  connId={tab.remoteConnId ?? ""}
                  sas={tab.remoteSas ?? ""}
                  isActive={isActive}
                  hostLabel={tab.remoteHostLabel ?? ""}
                  // 佔位——onConnectClick 是必填 prop 但真正的「開對話框、
                  // 就地重新連線」邏輯是下一個 task 的範圍，這裡先給
                  // no-op 讓型別檢查通過，不提前實作行為。
                  onConnectClick={() => {}}
                />
              ) : (
```

改成：

```tsx
              ) : tab.type === "remote-terminal" ? (
                // key={tab.remoteConnId}：連線切換時強制 React 把舊的
                // RemoteTerminalView 整個卸載、掛一個全新的實例，而不是
                // 保留舊實例只換 props。RemoteTerminalView 內部有十幾個
                // 只在掛載當下初始化一次的 state（phase、connectedAtRef/
                // elapsedMs、liveRows、hostRows、bookmarksOpen、
                // aiUnsupported、hostPlatform，還有 useTerminalBlocks
                // 自己的 blocks/isAlternateBuffer，以及 xterm 實例本身）
                // ——connId prop 換了值不會讓它們自動歸零，用 key 換掉
                // 整個實例才能保證乾淨的起始狀態，不需要在元件內部逐一
                // 手動清空、也不會有漏清某個 state 的風險。舊實例卸載
                // 時，既有的斷線 effect（disconnectTimerRef，[connId]
                // 依賴）會照常觸發，正確斷掉舊連線，不需要另外處理。
                <RemoteTerminalView
                  key={tab.remoteConnId}
                  tabId={tab.id}
                  connId={tab.remoteConnId ?? ""}
                  sas={tab.remoteSas ?? ""}
                  isActive={isActive}
                  hostLabel={tab.remoteHostLabel ?? ""}
                  onConnectClick={() => {
                    setReconnectTabId(tab.id);
                    setConnectOpen(true);
                  }}
                />
              ) : (
```

- [ ] **Step 5: `ConnectDialog` 呼叫處分流「開新分頁」與「更新既有分頁」**

找到：

```tsx
      {connectOpen && (
        <ConnectDialog
          onCancel={() => setConnectOpen(false)}
          onConnected={(connId, sas, hostLabel) => {
            setConnectOpen(false);
            const newId = crypto.randomUUID();
            setTabs((prev) => [
              ...prev,
              {
                id: newId,
                title: `${t.remote_terminal_tab}：${hostLabel}`,
                type: "remote-terminal",
                remoteConnId: connId,
                remoteHostLabel: hostLabel,
                // 這一端算出的驗證碼，要唸給對方核對。跟著連線的回傳值走而
                // 不是事件——事件會在這個分頁掛載之前就發出去。
                remoteSas: sas,
              },
            ]);
            selectTab(newId);
          }}
        />
      )}
```

改成：

```tsx
      {connectOpen && (
        <ConnectDialog
          onCancel={() => {
            setConnectOpen(false);
            // 沒清的話，使用者從工具列按了連線鈕、又按取消，下一次改從
            // ADD TAB 開新分頁走正常流程，會被誤判成「這是剛才那個分頁
            // 要求的重新連線」，錯誤地更新舊分頁而不是開新分頁。
            setReconnectTabId(null);
          }}
          onConnected={(connId, sas, hostLabel) => {
            setConnectOpen(false);
            if (reconnectTabId) {
              const targetId = reconnectTabId;
              setReconnectTabId(null);
              setTabs((prev) =>
                prev.map((tab) =>
                  tab.id === targetId
                    ? {
                        ...tab,
                        title: `${t.remote_terminal_tab}：${hostLabel}`,
                        remoteConnId: connId,
                        remoteHostLabel: hostLabel,
                        remoteSas: sas,
                      }
                    : tab,
                ),
              );
              selectTab(targetId);
              return;
            }
            const newId = crypto.randomUUID();
            setTabs((prev) => [
              ...prev,
              {
                id: newId,
                title: `${t.remote_terminal_tab}：${hostLabel}`,
                type: "remote-terminal",
                remoteConnId: connId,
                remoteHostLabel: hostLabel,
                // 這一端算出的驗證碼，要唸給對方核對。跟著連線的回傳值走而
                // 不是事件——事件會在這個分頁掛載之前就發出去。
                remoteSas: sas,
              },
            ]);
            selectTab(newId);
          }}
        />
      )}
```

**注意**：`t.remote_terminal_tab` 這裡的 `t` 是元件最上面 `useLocale()` 回傳的 `Translations` 物件（`TerminalApp.tsx:56` 附近既有的 `const { t } = useLocale();`），固定代表「遠端終端機」這個字串，跟 map 迴圈裡的 `tab`/新分支裡的 `tab` 變數（分頁物件本身）是兩個完全不同的東西——這裡故意把 map 迴圈裡原本用外層作用域同名 `tab` 的寫法保留（改用 `.map((tab) => ...)` 裡的區域變數 `tab`，不要跟外層 `.map((tab) => { const isActive = ... })` 那個 `tab` 混淆，兩者作用域不重疊，但變數名稱相同，寫的時候注意別搞混指到哪一層）。

- [ ] **Step 6: 執行測試，確認新測試通過，且沒有弄壞既有測試**

Run: `npx vitest run src/components/TerminalApp.remoteReconnect.test.tsx`
Expected: 全數通過。

Run: `npx vitest run src/components/TerminalApp.routeHintCloseGuard.test.tsx`
Expected: 全數通過（確認這次改動沒有影響既有的 `TerminalApp.tsx` 測試）。

- [ ] **Step 7: 確認 tsc 沒有報錯**

Run: `npx tsc -b`
Expected: 無錯誤輸出，結束碼 0。

- [ ] **Step 8: Commit**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM-full-parity
git add src/components/TerminalApp.tsx src/components/TerminalApp.remoteReconnect.test.tsx
git commit -m "$(cat <<'EOF'
feat(remote-terminal): 工具列連線按鈕可以就地切換連線，不開新分頁

新增 reconnectTabId state 記住是哪個既有分頁的工具列「連線」按鈕觸發了
ConnectDialog；連線成功時依此決定是更新那個分頁的 remoteConnId/
remoteSas/remoteHostLabel（就地切換），還是照舊開一個新分頁（沒有
reconnectTabId 時，即維持既有的 ADD TAB 流程）。取消時清空
reconnectTabId，避免污染下一次正常的開新分頁流程。

<RemoteTerminalView> 改用 key={tab.remoteConnId}：連線切換時讓 React
把舊實例整個卸載、掛一個全新的，RemoteTerminalView 內部十幾個只在
掛載當下初始化一次的 state 因此自動全部歸零，不需要手動逐一清空、
也不會有漏清某個 state 的風險。舊實例卸載時既有的斷線邏輯會照常觸發，
正確斷掉舊連線。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4：完整驗證與重啟 dev build

**Files:** 無新增/修改，純驗證。

- [ ] **Step 1: 前端型別檢查**

Run: `npx tsc -b`
Expected: 無錯誤輸出，結束碼 0。

- [ ] **Step 2: 前端完整測試套件**

Run: `npx vitest run`
Expected: 全數通過，沒有既有測試被改壞。

- [ ] **Step 3: Lint 範圍比對**

Run: `npx eslint src/components/RemoteTerminalView/index.tsx src/components/RemoteTerminalView/index.test.tsx src/components/TerminalApp.tsx src/components/TerminalApp.remoteReconnect.test.tsx src/lib/i18n.ts`
Expected: 沒有新增的 lint 錯誤（若有既有、跟這次改動無關的錯誤，比對是否在
改動前就已存在，方法同這個分支先前幾次驗證：另開一個 disposable 的
`git worktree add --detach` 比對改動前的版本，記得先 `ln -s` 對應的
`node_modules` 目錄再跑 `npx eslint`，不要對這個作用中的 worktree
執行 `git checkout <舊commit> -- .`）。

- [ ] **Step 4: 重新啟動 dev build，準備讓使用者做真機測試**

```bash
ps aux | grep -i "tauri dev\|target/debug/app\|node.*vite" | grep -v grep
```

把列出的舊 process（`npm exec tauri dev`、`node .../tauri`、`node .../vite`、
`target/debug/app`）逐一 `kill`，確認 `ps aux` 再查一次是乾淨的，然後：

```bash
cd /Users/jamesju/Documents/GitHub/AITERM-full-parity && nohup npm run tauri:dev > /tmp/aiterm-dev.log 2>&1 &
disown
```

等到 `target/debug/app` process 出現後再 `tail -20 /tmp/aiterm-dev.log`，
確認沒有 port 衝突（`Address already in use`）、沒有重複輪詢（`Another
instance is already polling`）之類的錯誤，且 `ps aux | grep target/debug/app`
只有一個新啟動的 process。

- [ ] **Step 5: 明確告知使用者以下事項，不要自己代為判斷完成**

1. 確認工具列開頭的「AITerm」變成帶漸層色的「✨ AITerm」樣式。
2. 確認連線資訊文字在視窗較窄時會換行，不會被截斷或跑版。
3. 確認「連線」按鈕出現在「指令書籤」左側，點下去跳出的對話框跟 ADD TAB
   開新分頁時的對話框外觀一致。
4. 在**既有**的遠端終端機分頁上點「連線」，輸入另一台主機的 host/port/
   六位碼、完成連線流程，確認：**沒有**多開一個新分頁，同一個分頁畫面
   直接切換成新的連線內容（分頁標題、位址、連線狀態都更新成新的）。
5. 確認切換連線後，工具列的「已連線時間」從 0 重新開始算，不是接續
   舊連線的秒數。
6. 點連線鈕後按取消，確認原本的分頁完全沒有變化，再跑一次正常的
   ADD TAB 流程確認還是會開新分頁（不會被誤判成剛才那次取消掉的
   重新連線）。
7. 全螢幕程式（Claude Code CLI）分頁下方的空白區域，這次應該只有
   稍微縮小（新增的品牌樣式行高＋連線按鈕不會完全填滿——已知、這次
   沒有解決的結構性差異，見 spec 的「範圍界定」一節），不要期待它
   完全消失。
8. 若發現任何問題，比照這個分支一貫的作法：不要用截圖描述去猜根因，讀
   實際程式碼、寫紅燈測試證明重現、再修。
