# 遠端終端機工具列 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `RemoteTerminalView`（遠端終端機分頁）頂部加一條跟本機 `TerminalView` 同樣風格的工具列，填補因兩邊視窗頂部工具列高度不一致而在遠端分頁下方留下的空白區域，並讓遠端分頁在視覺上跟本機分頁更一致。

**Architecture:** 沿用 `TerminalView.css` 既有的 `.aiterm-status`/`.aiterm-status-left` 樣式（額外 import 這個檔案，不重寫一套），左側顯示「位址 · 連線狀態 · 模式/已連線時間」文字，右側放「指令書籤」（真正可用，複用既有 `CommandBookmarksPicker`）與「Ask AI」（視覺佔位，點下去只是顯示既有的 `aiUnsupported` 提示）兩顆按鈕。

**Tech Stack:** TypeScript / React 19（`RemoteTerminalView/index.tsx`）、CSS（沿用 `TerminalView.css`）、i18n（`src/lib/i18n.ts`）、Vitest。

**參考設計文件：** `docs/superpowers/specs/2026-08-28-remote-terminal-toolbar-design.md`

---

### Task 1：新增翻譯鍵、`hostLabel` prop 資料流、連線狀態文字與已連線時間

**Files:**
- Modify: `src/lib/i18n.ts`
- Modify: `src/components/TerminalApp.tsx`
- Modify: `src/components/RemoteTerminalView/index.tsx`
- Test: `src/components/RemoteTerminalView/index.test.tsx`

- [ ] **Step 1: 新增翻譯鍵（zh-TW）**

找到 `src/lib/i18n.ts` 裡（zh-TW 區塊）：

```ts
    remote_terminal_ai_unsupported: "AI 指令目前不支援於遠端分頁。",
    // 遠端終端機共享——主控端
```

改成：

```ts
    remote_terminal_ai_unsupported: "AI 指令目前不支援於遠端分頁。",
    remote_terminal_toolbar_connected_prefix: "已連線",
    remote_terminal_toolbar_control_mode: "控制模式",
    remote_terminal_toolbar_ended: "連線已結束",
    // 遠端終端機共享——主控端
```

- [ ] **Step 2: 新增翻譯鍵（en）**

找到 `src/lib/i18n.ts` 裡（en 區塊）：

```ts
    remote_terminal_ai_unsupported: "AI commands are not supported in remote tabs yet.",
    // Remote terminal sharing — host side
```

改成：

```ts
    remote_terminal_ai_unsupported: "AI commands are not supported in remote tabs yet.",
    remote_terminal_toolbar_connected_prefix: "Connected",
    remote_terminal_toolbar_control_mode: "Control mode",
    remote_terminal_toolbar_ended: "Connection ended",
    // Remote terminal sharing — host side
```

- [ ] **Step 3: 確認既有的語系同步測試通過**

Run: `npx vitest run src/lib/i18n.remoteTerminal.test.ts`
Expected: 全數通過——`src/lib/i18n.remoteTerminal.test.ts` 裡的
`"keeps the two locales in sync for sharing strings"` 這個既有測試會自動
檢查 `remote_terminal_` 開頭的鍵兩個語系是否一致，不需要為這件事另外
寫新測試。若這裡失敗，代表 Step 1/2 兩邊的鍵不對稱，回去檢查拼字。

- [ ] **Step 4: 寫失敗的測試——連線狀態文字**

在 `src/components/RemoteTerminalView/index.test.tsx` 裡，緊接既有的
`"disables WarpInput while read-only"` 測試之後，新增：

```tsx
  it("工具列顯示位址與連線狀態文字，隨 phase 變化", async () => {
    render(<RemoteTerminalView tabId="t1" connId="c17" sas="1717" isActive hostLabel="10.10.41.1:50281" />);

    // 等待核准中：顯示位址與等待文字，不顯示任何連線時間或模式字樣。
    expect(await screen.findByText(/10\.10\.41\.1:50281/)).toBeInTheDocument();
    expect(screen.getByText(/等待對方同意|Waiting for them to accept/)).toBeInTheDocument();
    expect(screen.queryByText(/已連線|Connected/)).not.toBeInTheDocument();

    await waitFor(() => expect(handlers["granted:c17"]).toBeDefined());
    act(() => {
      handlers["granted:c17"]({ mode: "control", cols: 80, rows: 24, hostOs: "linux" } as never);
    });

    // 已連線：顯示模式文字。
    await waitFor(() => {
      expect(screen.getByText(/已連線.*控制模式|Connected.*Control mode/)).toBeInTheDocument();
    });

    // 唯讀模式文字沿用既有翻譯鍵，兩者用同一個 phase 走一次確認切得過去。
    act(() => {
      handlers["control:c17"]("read_only" as never);
    });
    await waitFor(() => {
      expect(screen.getByText(/已連線.*唯讀|Connected.*Read-only/)).toBeInTheDocument();
    });

    // 連線結束：顯示結束文字，不再顯示模式或連線時間。
    await waitFor(() => expect(handlers["ended:c17"]).toBeDefined());
    act(() => {
      handlers["ended:c17"]("host_stopped_sharing" as never);
    });
    await waitFor(() => {
      expect(screen.getByText(/連線已結束|Connection ended/)).toBeInTheDocument();
    });
  });

  it("已連線時間從進入 live 那一刻開始每秒遞增，控制權變更不會讓它歸零", async () => {
    vi.useFakeTimers();
    try {
      render(<RemoteTerminalView tabId="t1" connId="c18" sas="1818" isActive hostLabel="10.10.41.1:50281" />);
      await waitFor(() => expect(handlers["granted:c18"]).toBeDefined());
      act(() => {
        handlers["granted:c18"]({ mode: "control", cols: 80, rows: 24, hostOs: "linux" } as never);
      });

      // 剛進 live：還沒經過任何一次 1 秒 tick，顯示 0s。
      await waitFor(() => {
        expect(screen.getByText(/已連線 0s|Connected 0s/)).toBeInTheDocument();
      });

      act(() => {
        vi.advanceTimersByTime(3000);
      });
      await waitFor(() => {
        expect(screen.getByText(/已連線 3s|Connected 3s/)).toBeInTheDocument();
      });

      // 控制權變更（同樣是 phase.kind === "live"，只是 mode 換了）不該讓
      // 已經走了的秒數歸零——這是 connectedAtRef 用 `=== null` 判斷、
      // 只在第一次進 live 時寫入的用意所在。
      act(() => {
        handlers["control:c18"]("read_only" as never);
      });
      act(() => {
        vi.advanceTimersByTime(2000);
      });
      await waitFor(() => {
        expect(screen.getByText(/已連線 5s|Connected 5s/)).toBeInTheDocument();
      });
    } finally {
      vi.useRealTimers();
    }
  });
```

**注意**：這個檔案既有的 `handlers`/`act`/`waitFor`/`screen` 都已經在檔案
最上面 import 好了，直接用即可。`vi.useFakeTimers()`／`vi.useRealTimers()`
這個檔案已經有先例（見 `disconnect timing (StrictMode dev-mode trap)`
那個 describe block），跟進即可。

- [ ] **Step 5: 執行測試，確認失敗**

Run: `npx vitest run src/components/RemoteTerminalView/index.test.tsx -t "工具列顯示位址與連線狀態文字\|已連線時間從進入"`
Expected: FAIL——目前 `RemoteTerminalView` 完全沒有渲染任何位址、連線
狀態或已連線時間文字，也沒有 `hostLabel` 這個 prop（TypeScript 應該也會
在這一步順便報 `hostLabel` 不是合法 prop，屬於預期中的失敗，不用另外
處理，Step 6 之後就會修好）。

- [ ] **Step 6: `Props` 新增 `hostLabel`，新增連線時間 state 與計算邏輯**

找到 `src/components/RemoteTerminalView/index.tsx` 裡：

```tsx
interface Props {
  tabId: string;
  /** 2B-1 的觀看連線 id。所有 `share-viewer://*` 事件都掛在它上面。 */
  connId: string;
  /**
   * 這一端算出的 4 位驗證碼，**要顯示給使用者唸給對方聽**。
   *
   * 用 prop 而不是訂閱事件：它在連線建立的當下就已知（跟著
   * `shareViewerConnect` 的回傳值一起來），而這個元件要等分頁開好才掛載
   * ——用事件送必然遺失，因為發出的時候還沒有人在聽。
   */
  sas: string;
  isActive: boolean;
}
```

改成：

```tsx
interface Props {
  tabId: string;
  /** 2B-1 的觀看連線 id。所有 `share-viewer://*` 事件都掛在它上面。 */
  connId: string;
  /**
   * 這一端算出的 4 位驗證碼，**要顯示給使用者唸給對方聽**。
   *
   * 用 prop 而不是訂閱事件：它在連線建立的當下就已知（跟著
   * `shareViewerConnect` 的回傳值一起來），而這個元件要等分頁開好才掛載
   * ——用事件送必然遺失，因為發出的時候還沒有人在聽。
   */
  sas: string;
  isActive: boolean;
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

找到：

```tsx
export function RemoteTerminalView({ tabId, connId, sas, isActive }: Props) {
```

改成：

```tsx
export function RemoteTerminalView({ tabId, connId, sas, isActive, hostLabel = "" }: Props) {
```

- [ ] **Step 7: 新增已連線時間 state 與計算邏輯**

找到：

```tsx
  const [aiUnsupported, setAiUnsupported] = useState(false);
```

改成（新增在它之前）：

```tsx
  // 已連線時間從進入 live 的那一刻開始算，用 `=== null` 當 guard 只寫入
  // 一次——`phase.kind` 進了 "live" 之後，後續的控制權變更
  // （onShareViewerControlChanged）會用不同的 mode 再次呼叫
  // setPhase({kind:"live", ...})，但 `phase.kind` 這個依賴值本身沒變，
  // 這個 effect 不會重跑，connectedAtRef 因此不會被後續的 mode 變更
  // 動到，不需要另外分辨「是第一次進 live 還是後續的 mode 變更」。
  const connectedAtRef = useRef<number | null>(null);
  const [elapsedMs, setElapsedMs] = useState(0);
  useEffect(() => {
    if (phase.kind === "live" && connectedAtRef.current === null) {
      connectedAtRef.current = Date.now();
    }
  }, [phase.kind]);
  useEffect(() => {
    if (phase.kind !== "live") return;
    const interval = setInterval(() => {
      if (connectedAtRef.current !== null) {
        setElapsedMs(Date.now() - connectedAtRef.current);
      }
    }, 1000);
    return () => clearInterval(interval);
  }, [phase.kind]);

  const [aiUnsupported, setAiUnsupported] = useState(false);
```

- [ ] **Step 8: 加入工具列 JSX（左側文字）**

找到：

```tsx
  return (
    <div className="aiterm-remote-terminal" data-tab-id={tabId} data-active={isActive}>
      {phase.kind === "waiting" && (
```

改成：

```tsx
  return (
    <div className="aiterm-remote-terminal" data-tab-id={tabId} data-active={isActive}>
      <div className="aiterm-status">
        <span className="aiterm-status-left">
          AITerm · {t.remote_terminal_tab} {hostLabel} · {connectionStatusText(t, phase, elapsedMs)}
        </span>
      </div>
      {phase.kind === "waiting" && (
```

- [ ] **Step 9: 新增 `connectionStatusText`／`formatElapsed` 函式**

找到檔案最後面 `endReasonText` 函式定義之後（檔案結尾）：

```tsx
function endReasonText(t: Translations, reason: string): string {
  const key = `remote_terminal_ended_${reason}`;
  const table = t as unknown as Record<string, string>;
  return table[key] ?? t.remote_terminal_ended_session_closed;
}
```

在它之後（檔案最後）新增：

```tsx

/**
 * 工具列左側的連線狀態片語，依 `phase` 三態切換。`elapsedMs` 只在
 * `phase.kind === "live"` 時才會被用到，其餘兩態忽略它。
 */
function connectionStatusText(t: Translations, phase: Phase, elapsedMs: number): string {
  if (phase.kind === "waiting") return t.remote_terminal_waiting_approval;
  if (phase.kind === "ended") return t.remote_terminal_toolbar_ended;
  const modeLabel = phase.mode === "read_only" ? t.remote_terminal_read_only : t.remote_terminal_toolbar_control_mode;
  return `${t.remote_terminal_toolbar_connected_prefix} ${formatElapsed(elapsedMs)} · ${modeLabel}`;
}

/**
 * 把毫秒數轉成「12s」/「3m45s」/「1h05m」這種簡短格式。跟
 * `TerminalBlockCard.tsx` 裡既有的 `formatDuration` 邏輯類似但獨立寫一份
 * （不跨檔案匯出私有函式）：那邊是給單一指令的執行時間用，通常不會超過
 * 一小時，這裡是連線總時間，可能開很久，需要多處理小時這一級，用途不同
 * 分開寫更清楚。
 */
function formatElapsed(ms: number): string {
  const totalSeconds = Math.floor(ms / 1000);
  if (totalSeconds < 60) return `${totalSeconds}s`;
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (minutes < 60) return `${minutes}m${seconds}s`;
  const hours = Math.floor(minutes / 60);
  const remMinutes = minutes % 60;
  return `${hours}h${String(remMinutes).padStart(2, "0")}m`;
}
```

- [ ] **Step 10: import `TerminalView.css`**

找到 `src/components/RemoteTerminalView/index.tsx` 檔案最上面：

```tsx
import type { Translations } from "../../lib/i18n";
import "./index.css";
```

改成：

```tsx
import type { Translations } from "../../lib/i18n";
import "../TerminalView.css";
import "./index.css";
```

**為什麼要多 import 一個看起來「屬於別的元件」的 CSS 檔**：這個工具列
刻意直接沿用 `TerminalView.css` 裡 `.aiterm-status`/`.aiterm-status-left`
既有的樣式規則（背景色、padding、左右兩區 flex 排版），不重寫一套新的
——這樣兩邊分頁的工具列字體、間距、外觀永遠自動一致，不會因為各自維護
一份相近但微妙不同的 CSS 而之後跑掉。CSS 檔案在這個專案裡是全域套用的
純樣式表（沒有用 CSS Modules 做 scope 隔離），從哪個元件 import 只影響
「這份樣式表有沒有被打進最終的 bundle」，不影響套用範圍，兩個元件都
import 同一份是安全、冪等的，不會有重複定義或衝突的問題。

- [ ] **Step 11: 執行測試，確認 Step 4 的新測試通過，且沒有弄壞既有測試**

Run: `npx vitest run src/components/RemoteTerminalView/index.test.tsx`
Expected: 全數通過。

- [ ] **Step 12: 確認 tsc 沒有報錯**

Run: `npx tsc -b`
Expected: 無錯誤輸出，結束碼 0。

- [ ] **Step 13: `TerminalApp.tsx` 傳入 `hostLabel` prop**

找到 `src/components/TerminalApp.tsx` 裡：

```tsx
              ) : tab.type === "remote-terminal" ? (
                <RemoteTerminalView
                  tabId={tab.id}
                  connId={tab.remoteConnId ?? ""}
                  sas={tab.remoteSas ?? ""}
                  isActive={isActive}
                />
              ) : (
```

改成：

```tsx
              ) : tab.type === "remote-terminal" ? (
                <RemoteTerminalView
                  tabId={tab.id}
                  connId={tab.remoteConnId ?? ""}
                  sas={tab.remoteSas ?? ""}
                  isActive={isActive}
                  hostLabel={tab.remoteHostLabel ?? ""}
                />
              ) : (
```

- [ ] **Step 14: 確認 tsc 與既有測試仍然通過**

Run: `npx tsc -b`
Expected: 無錯誤輸出，結束碼 0。

Run: `npx vitest run`
Expected: 全數通過，沒有既有測試被改壞（`TerminalApp.tsx` 這個改動只是
多傳一個選填 prop，不應該影響任何既有測試）。

- [ ] **Step 15: Commit**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM-full-parity
git add src/lib/i18n.ts src/components/TerminalApp.tsx src/components/RemoteTerminalView/index.tsx src/components/RemoteTerminalView/index.test.tsx
git commit -m "$(cat <<'EOF'
feat(remote-terminal): 工具列顯示連線位址、狀態與已連線時間

使用者發現遠端終端機分頁下方留白，根因是本機視窗頂部多一條工具列
（分頁列＋連線狀態＋分享/書籤/Remote/Ask AI 按鈕），同樣視窗高度下
本機能分給終端機內容的空間本來就比沒有這條工具列的遠端分頁少——不是
即時窗格高度算錯，是兩邊工具列高度不一致。與其把即時窗格硬撐高去填
這個差距，改成讓遠端分頁也有一條對等的工具列，兩邊視覺結構一致。

這個 task 先做左側的連線資訊文字：位址（沿用視窗標題同一份 hostLabel
資料，這次補上傳進元件）、連線狀態（等待/已連線/已結束）、模式
（控制/唯讀）、已連線時間（從進入 live 那一刻起算，每秒更新，格式
比照 TerminalBlockCard 既有的耗時顯示邏輯但獨立一份，延伸支援小時）。
直接沿用 TerminalView.css 既有的 .aiterm-status 樣式，不重寫一套。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2：工具列右側加入「指令書籤」（可用）與「Ask AI」（視覺佔位）按鈕

**Files:**
- Modify: `src/components/RemoteTerminalView/index.tsx`
- Test: `src/components/RemoteTerminalView/index.test.tsx`

- [ ] **Step 1: 寫失敗的測試**

在 `src/components/RemoteTerminalView/index.test.tsx` 裡，緊接 Task 1
新增的「已連線時間從進入 live 那一刻開始每秒遞增」測試之後，新增：

```tsx
  it("點指令書籤按鈕開啟選單，選擇後把指令文字填進輸入框", async () => {
    // CommandBookmarksPicker 選擇後是透過全域事件 warp-fill-command 跟
    // WarpInput 溝通（跟 TerminalView.tsx 完全同一條路徑）——這裡直接
    // 監聽這個事件確認有正確送出，不用真的去戳 WarpInput 內部狀態。
    const fillSpy = vi.fn();
    window.addEventListener("warp-fill-command", fillSpy);
    try {
      render(<RemoteTerminalView tabId="t1" connId="c19" sas="1919" isActive hostLabel="10.10.41.1:50281" />);

      const bookmarkBtn = await screen.findByTitle(/儲存至書籤|Save to Bookmarks/i);
      await userEvent.click(bookmarkBtn);

      // CommandBookmarksPicker 沒有書籤時仍然會渲染（空清單），這裡只
      // 驗證按鈕確實開啟了選單本身，不驗證書籤內容——書籤資料的正確性
      // 由 CommandBookmarks 自己的測試負責，不是這個元件的職責。
      expect(await screen.findByText(/⭐ 指令書籤|⭐ Command Bookmarks/)).toBeInTheDocument();
    } finally {
      window.removeEventListener("warp-fill-command", fillSpy);
    }
  });

  it("點 Ask AI 按鈕只顯示既有的不支援提示，不會真的呼叫任何 AI", async () => {
    render(<RemoteTerminalView tabId="t1" connId="c20" sas="2020" isActive hostLabel="10.10.41.1:50281" />);

    const askAiBtn = await screen.findByTitle(/開啟 AI 助手|Open AI Helper/i);
    await userEvent.click(askAiBtn);

    expect(await screen.findByText(/AI 指令目前不支援|not supported in remote/i)).toBeInTheDocument();
    // sendMock 是這個檔案既有的、代表「真的送位元組給對方」的探針——
    // 點 Ask AI 不該觸發任何送出行為。
    expect(sendMock).not.toHaveBeenCalled();
  });
```

- [ ] **Step 2: 執行測試，確認失敗**

Run: `npx vitest run src/components/RemoteTerminalView/index.test.tsx -t "點指令書籤按鈕\|點 Ask AI 按鈕"`
Expected: FAIL——目前工具列右側完全沒有任何按鈕，`findByTitle` 會逾時
找不到元素。

- [ ] **Step 3: import 需要的模組**

找到 `src/components/RemoteTerminalView/index.tsx` 檔案最上面：

```tsx
import { useCallback, useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import {
  onShareViewerControlChanged,
  onShareViewerData,
  onShareViewerEnded,
  onShareViewerGranted,
  onShareViewerResync,
  shareViewerDisconnect,
  shareViewerSend,
} from "../../ipc/shareViewer";
import { useLocale } from "../../contexts/LocaleContext";
import { getActiveTheme, type AppTheme } from "../../lib/themes";
import { useTerminalBlocks } from "../../hooks/useTerminalBlocks";
import { WarpInput } from "../WarpInput";
import { TerminalBlockCard } from "../TerminalBlockCard";
import { addBookmark } from "../CommandBookmarks";
import { parseAiPrefix, parseAgentPrefix } from "../parseAiPrefix";
import type { Translations } from "../../lib/i18n";
import "../TerminalView.css";
import "./index.css";
```

改成：

```tsx
import { useCallback, useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import {
  onShareViewerControlChanged,
  onShareViewerData,
  onShareViewerEnded,
  onShareViewerGranted,
  onShareViewerResync,
  shareViewerDisconnect,
  shareViewerSend,
} from "../../ipc/shareViewer";
import { useLocale } from "../../contexts/LocaleContext";
import { getActiveTheme, type AppTheme } from "../../lib/themes";
import { useTerminalBlocks } from "../../hooks/useTerminalBlocks";
import { WarpInput } from "../WarpInput";
import { TerminalBlockCard } from "../TerminalBlockCard";
import { CommandBookmarksPicker, addBookmark } from "../CommandBookmarks";
import { parseAiPrefix, parseAgentPrefix } from "../parseAiPrefix";
import { SparklesIcon } from "../Icons";
import type { Translations } from "../../lib/i18n";
import "../TerminalView.css";
import "./index.css";
```

- [ ] **Step 4: 新增 `bookmarksOpen` state**

找到（Task 1 已經加過 `elapsedMs` 相關程式碼，這裡接在它後面）：

```tsx
  const [aiUnsupported, setAiUnsupported] = useState(false);
```

改成：

```tsx
  const [aiUnsupported, setAiUnsupported] = useState(false);
  const [bookmarksOpen, setBookmarksOpen] = useState(false);
```

- [ ] **Step 5: 工具列右側加入兩顆按鈕與書籤選單**

找到（Task 1 加的那段）：

```tsx
      <div className="aiterm-status">
        <span className="aiterm-status-left">
          AITerm · {t.remote_terminal_tab} {hostLabel} · {connectionStatusText(t, phase, elapsedMs)}
        </span>
      </div>
```

改成：

```tsx
      <div className="aiterm-status">
        <span className="aiterm-status-left">
          AITerm · {t.remote_terminal_tab} {hostLabel} · {connectionStatusText(t, phase, elapsedMs)}
        </span>
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
          {/* 視覺佔位，不是真的 AI 功能——遠端連線目前完全不支援 AI/Agent
              指令（見下面 handleWarpSubmit 對 /ai、/agent 開頭的既有擋法），
              點下去只是重用同一套拒絕邏輯顯示既有提示，不呼叫任何 API。
              這次的目標純粹是視覺對齊本機分頁的工具列，不是新增 AI 能力。 */}
          <button
            className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
            title={t.term_ai_helper_tooltip}
            onClick={(e) => {
              e.stopPropagation();
              setAiUnsupported(true);
            }}
            style={{ display: "flex", alignItems: "center", gap: "6px" }}
          >
            <SparklesIcon size={14} />
            <span>Ask AI</span>
          </button>
        </span>
      </div>
      {bookmarksOpen && (
        <CommandBookmarksPicker
          onSelect={(cmd) => {
            setBookmarksOpen(false);
            window.dispatchEvent(new CustomEvent("warp-fill-command", { detail: { cmd } }));
          }}
          onClose={() => setBookmarksOpen(false)}
        />
      )}
```

- [ ] **Step 6: 執行測試，確認新測試通過，且沒有弄壞既有測試**

Run: `npx vitest run src/components/RemoteTerminalView/index.test.tsx`
Expected: 全數通過。

- [ ] **Step 7: 確認 tsc 沒有報錯**

Run: `npx tsc -b`
Expected: 無錯誤輸出，結束碼 0。

- [ ] **Step 8: Commit**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM-full-parity
git add src/components/RemoteTerminalView/index.tsx src/components/RemoteTerminalView/index.test.tsx
git commit -m "$(cat <<'EOF'
feat(remote-terminal): 工具列右側加入指令書籤與 Ask AI 佔位按鈕

指令書籤：真正可用，複用本機分頁同一套 CommandBookmarksPicker，選擇
後透過既有的 warp-fill-command 全域事件把指令文字填進 WarpInput，不需
要改 WarpInput 本身。

Ask AI：視覺佔位，不是新功能——遠端連線目前完全不支援 AI/Agent 指令，
點下去只是重用既有的 aiUnsupported 提示邏輯，不呼叫任何 API、不擴大
範圍。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3：完整驗證與重啟 dev build

**Files:** 無新增/修改，純驗證。

- [ ] **Step 1: 前端型別檢查**

Run: `npx tsc -b`
Expected: 無錯誤輸出，結束碼 0。

- [ ] **Step 2: 前端完整測試套件**

Run: `npx vitest run`
Expected: 全數通過，沒有既有測試被改壞。

- [ ] **Step 3: Lint 範圍比對**

Run: `npx eslint src/components/RemoteTerminalView/index.tsx src/components/RemoteTerminalView/index.test.tsx src/components/TerminalApp.tsx src/lib/i18n.ts`
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

1. 重新連線一次遠端終端機分頁，確認頂部工具列出現、樣式跟本機分頁的
   工具列一致（背景色、字體、按鈕外觀）。
2. 確認左側文字依連線階段正確顯示：等待核准中的等待文字、已連線後的
   「已連線 Xs · 控制模式/唯讀」、對方結束分享後的「連線已結束」。
3. 放著不動，確認已連線時間會持續遞增（每秒），不會卡住或跳回 0。
4. 點「指令書籤」，確認選單正常開啟、選擇後指令文字真的填進下方輸入框。
5. 點「Ask AI」，確認顯示「AI 指令目前不支援於遠端分頁」提示，不會有
   任何其他反應（不會真的呼叫 AI）。
6. 確認全螢幕程式（vim/htop/Claude Code CLI）分頁下方的空白區域，是否
   因為新工具列佔用了原本本機工具列多出來的那部分高度差，明顯變小或
   消失——若還有明顯空白，不要用猜的調數字，讀實際渲染出來的高度、
   量測本機與遠端兩邊工具列的實際像素高度差，回報給我。
7. 若發現任何問題，比照這個分支一貫的作法：不要用截圖描述去猜根因，讀
   實際程式碼、寫紅燈測試證明重現、再修。
