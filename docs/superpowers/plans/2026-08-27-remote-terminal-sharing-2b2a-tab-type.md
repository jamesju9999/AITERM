# 遠端終端機共享 2B-2a：`remote-terminal` 分頁型別與畫面 — 實作計畫

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新增 `remote-terminal` 分頁型別，讓 2B-1 做好的遠端畫面能真的顯示在一個分頁裡——xterm 依主控端尺寸建立、唯讀時按鍵不送出、連線結束時保留最後畫面。

**Architecture:** 新分頁型別**不重用 `terminal`**——既有終端機分頁的整套邏輯都假設背後有本機 PTY session（cwd 追蹤、書籤、AI 面板注入、close guard、agent mission），遠端分頁一個都沒有。`RemoteTerminalView` 只訂閱 `share-viewer://*` 事件，比照 `TerminalView` 訂閱 `pty://data/{id}` 的既有模式。

**Tech Stack:** React 19 / TypeScript / xterm.js / Vitest

**Spec:** `docs/superpowers/specs/2026-08-26-remote-terminal-sharing-2-ui-design.md`

**本計畫不含**（留給 2B-2b）：分享按鈕、分享面板、同意視窗、連線對話框。2B-2a 結束時分頁型別存在且畫面會動，但**還沒有入口能開出這種分頁**——測試是唯一的驅動方式。

---

## 這個計畫最容易出錯的地方

**九個檔案會對 `TabType` 做分支**：

```
src/components/TerminalApp.tsx
src/components/RouteHint.tsx
src/components/TabBar/index.tsx
src/components/HomeView/index.tsx
src/components/HomeView/LaunchGrid.tsx
src/components/HomeView/routeIntent.ts
src/components/NewTabPicker/index.tsx
src/components/NewTabPicker/tabCatalog.tsx
src/components/NewTabPicker/tabCatalog.test.tsx
```

spec 記了一條踩過的坑：**側邊欄的「Agent: xxx」分頁實際型別是 `terminal` 而非 `code-assistant`，型別判斷搞錯曾害整套分頁邏輯做錯。** 新型別漏處理某一處不會編譯失敗（TypeScript 的 union 在 `if` 鏈裡不強制窮舉），會變成執行期的怪現象——例如分頁標題空白、圖示不見、關閉時沒有確認。

所以 Task 2 有一個**專門掃過所有分支點的測試**，而不只是「看起來有加到」。

---

## 檔案結構

| 檔案 | 責任 | 動作 |
|---|---|---|
| `src/lib/i18n.ts` | 新字串（zh-TW ＋ en 兩份） | 修改 |
| `src/components/TabBar/index.tsx` | `TabType` 加一個成員、`Tab` 加遠端欄位、圖示 | 修改 |
| `src/components/TerminalApp.tsx` | 標題、渲染分支 | 修改 |
| `src/components/RemoteTerminalView/index.tsx` | 遠端畫面本體 | 新增 |
| `src/components/RemoteTerminalView/index.css` | 樣式 | 新增 |
| `src/components/RemoteTerminalView/index.test.tsx` | 元件測試 | 新增 |
| `src/components/tabTypeCoverage.test.ts` | 掃過所有分支點的覆蓋測試 | 新增 |

---

## Task 1: i18n 字串

**Files:**
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: 加 zh-TW 字串**

`src/lib/i18n.ts` 的 `const zhTW = {` 區塊裡，找到 `bookmarks_title:` 那一行（約 792 行），在它**後面**加入：

```ts
    // 遠端終端機共享——觀看端
    remote_terminal_tab: "遠端終端機",
    remote_terminal_connecting: "連線中…",
    remote_terminal_waiting_approval: "等待對方同意…",
    remote_terminal_read_only: "唯讀",
    remote_terminal_your_code: "請把這組數字唸給對方核對",
    remote_terminal_ended_denied: "對方沒有同意這次連線",
    remote_terminal_ended_host_stopped_sharing: "對方停止分享了",
    remote_terminal_ended_session_closed: "那個終端機已經關閉",
    remote_terminal_ended_kicked_by_host: "對方結束了你的連線",
    remote_terminal_ended_invalid_code: "編號無效或已失效",
    remote_terminal_ended_version_mismatch: "兩邊的 AITerm 版本不同，請更新",
    remote_terminal_ended_sas_commit_mismatch: "驗證失敗，可能有人在中間攔截",
    remote_terminal_ended_sas_handshake_failed: "連線交握失敗，請重試",
```

- [ ] **Step 2: 加 en 字串**

同一個檔案的 `const enRaw = {` 區塊（約 1299 行開始），找到對應的 `bookmarks_title:`（約 2088 行），在它**後面**加入：

```ts
    // Remote terminal sharing — viewer side
    remote_terminal_tab: "Remote Terminal",
    remote_terminal_connecting: "Connecting…",
    remote_terminal_waiting_approval: "Waiting for them to accept…",
    remote_terminal_read_only: "Read-only",
    remote_terminal_your_code: "Read these digits out to them to compare",
    remote_terminal_ended_denied: "They did not accept the connection",
    remote_terminal_ended_host_stopped_sharing: "They stopped sharing",
    remote_terminal_ended_session_closed: "That terminal has closed",
    remote_terminal_ended_kicked_by_host: "They ended your connection",
    remote_terminal_ended_invalid_code: "That code is invalid or has expired",
    remote_terminal_ended_version_mismatch: "The two AITerm versions differ — please update",
    remote_terminal_ended_sas_commit_mismatch: "Verification failed — someone may be intercepting",
    remote_terminal_ended_sas_handshake_failed: "Connection handshake failed — please retry",
```

- [ ] **Step 3: 寫測試確認八個結束原因都有對應的人話**

建立 `src/lib/i18n.remoteTerminal.test.ts`：

```ts
import { describe, expect, it } from "vitest";
import { translations } from "./i18n";

/**
 * 後端 `EndReason` 的八個值。改動這個陣列前先看
 * `src-tauri/src/share/protocol.rs`——那邊新增變體時，這裡跟 i18n 都要跟著加。
 */
const END_REASONS = [
  "denied",
  "host_stopped_sharing",
  "session_closed",
  "kicked_by_host",
  "invalid_code",
  "version_mismatch",
  "sas_commit_mismatch",
  "sas_handshake_failed",
] as const;

describe("remote terminal i18n", () => {
  for (const locale of ["zh-TW", "en"] as const) {
    it(`has a human sentence for every end reason in ${locale}`, () => {
      // spec 要求「不能有『未知錯誤』」——每個結束原因都要有一句人話。
      const t = translations[locale] as Record<string, string>;
      for (const reason of END_REASONS) {
        const key = `remote_terminal_ended_${reason}`;
        expect(t[key], `missing ${key} in ${locale}`).toBeTruthy();
      }
    });
  }

  it("keeps the two locales in sync for remote terminal strings", () => {
    // 語系漂移是這個 repo 記過的坑：只加一邊，另一邊會靜默 fallback 或空白。
    const zh = Object.keys(translations["zh-TW"]).filter((k) => k.startsWith("remote_terminal_"));
    const en = Object.keys(translations["en"]).filter((k) => k.startsWith("remote_terminal_"));
    expect(zh.sort()).toEqual(en.sort());
  });
});
```

- [ ] **Step 4: 確認 `translations` 這個名字對得上**

`i18n.ts` 匯出的物件叫什麼要實際確認：

```bash
grep -n "^export const translations\|^export const t\b\|export const TRANSLATIONS" src/lib/i18n.ts
```

若匯出的名字不是 `translations`，把測試裡的 import 改成實際的名字。若它不是一個 `{ "zh-TW": ..., "en": ... }` 形狀的物件，回報給我——那代表測試要換一種寫法。

- [ ] **Step 5: 跑測試**

Run: `npx vitest run src/lib/i18n.remoteTerminal.test.ts`
Expected: PASS，3 個測試（zh-TW、en、同步）。

- [ ] **Step 6: Commit**

```bash
git add src/lib/i18n.ts src/lib/i18n.remoteTerminal.test.ts
git commit -m "feat(share): i18n strings for the remote terminal viewer

八個 EndReason 各對應一句人話——spec 要求不能有「未知錯誤」。
測試同時守著兩個語系不漂移。"
```

---

## Task 2: `remote-terminal` 分頁型別

**Files:**
- Modify: `src/components/TabBar/index.tsx`
- Modify: `src/components/TerminalApp.tsx`
- Test: `src/components/tabTypeCoverage.test.ts`（新增）

- [ ] **Step 1: 寫會紅的覆蓋測試**

建立 `src/components/tabTypeCoverage.test.ts`：

```ts
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";

/**
 * 新增分頁型別時，九個檔案會對 `TabType` 做分支。漏處理某一處**不會編譯
 * 失敗**（TypeScript 的 union 在 `if` 鏈裡不強制窮舉），只會變成執行期的
 * 怪現象：標題空白、圖示不見、關閉時沒有確認。
 *
 * 這個 repo 記過一次同類的坑：側邊欄的「Agent: xxx」分頁實際型別是
 * `terminal` 而非 `code-assistant`，型別判斷搞錯害整套分頁邏輯做錯。
 *
 * 所以這個測試直接掃原始碼，確認新型別在每個該出現的地方都出現了。
 * 掃字串比逐一寫 render 測試脆，但它抓的是「有沒有漏掉某個檔案」，
 * 那正是逐一寫測試最容易漏的東西。
 */
const ROOT = join(__dirname, "..", "..");

function read(rel: string): string {
  return readFileSync(join(ROOT, rel), "utf8");
}

describe("remote-terminal tab type coverage", () => {
  it("is a member of the TabType union", () => {
    const src = read("src/components/TabBar/index.tsx");
    expect(src).toMatch(/export type TabType =[^;]*"remote-terminal"/s);
  });

  it("has an icon in the tab bar", () => {
    // 沒有圖示的分頁在側邊欄會是一個空格，看起來像壞掉。
    const src = read("src/components/TabBar/index.tsx");
    const iconFn = src.slice(src.indexOf("function getTabIcon"));
    expect(iconFn).toContain("remote-terminal");
  });

  it("gets a title when opened", () => {
    // TerminalApp 有一串 `if (type === "...") title = ...`。漏掉的話標題會是
    // 預設值或空字串。
    const src = read("src/components/TerminalApp.tsx");
    expect(src).toMatch(/type === "remote-terminal"\s*\)\s*title\s*=/);
  });

  it("has a render branch", () => {
    const src = read("src/components/TerminalApp.tsx");
    expect(src).toContain('tab.type === "remote-terminal"');
  });

  it("renders RemoteTerminalView", () => {
    const src = read("src/components/TerminalApp.tsx");
    expect(src).toContain("RemoteTerminalView");
  });
});
```

- [ ] **Step 2: 跑測試確認會紅**

Run: `npx vitest run src/components/tabTypeCoverage.test.ts`
Expected: FAIL，五個測試全部失敗（型別還不存在）。

- [ ] **Step 3: `TabType` 加成員、`Tab` 加欄位**

`src/components/TabBar/index.tsx` 第 26 行，把：

```ts
export type TabType = "terminal" | "database" | "design" | "cross-db" | "vcs" | "doc-converter" | "api-docs" | "loop-studio" | "code-assistant" | "knowledge-base" | "mail";
```

改成（在結尾加一個成員）：

```ts
export type TabType = "terminal" | "database" | "design" | "cross-db" | "vcs" | "doc-converter" | "api-docs" | "loop-studio" | "code-assistant" | "knowledge-base" | "mail" | "remote-terminal";
```

在 `export interface Tab {` 裡，`cwd?: string;` 那一組欄位之後加入：

```ts
  /** 遠端終端機分頁：2B-1 的觀看連線 id，所有 `share-viewer://*` 事件都掛在它上面。
   *  只有 `type === "remote-terminal"` 的分頁會有這個欄位。 */
  remoteConnId?: string;
  /** 遠端終端機分頁：對方的顯示名稱，用來當分頁標題。**未經驗證**，是對方自報的。 */
  remoteHostLabel?: string;
```

- [ ] **Step 4: 加圖示**

同一個檔案的 `function getTabIcon(...)` 裡，比照既有分支加入（用既有的 `EyeIcon`——遠端分頁是「在看別人的畫面」）：

```ts
  if (type === "remote-terminal") return <EyeIcon size={14} />;
```

確認 `EyeIcon` 已經在這個檔案的 import 清單裡；沒有的話加進去（它定義在 `src/components/Icons.tsx`）。

- [ ] **Step 5: 加標題**

`src/components/TerminalApp.tsx` 那串 `if (type === "...") title = ...`（約 261-270 行），在 `if (type === "mail") title = t.mail_tab;` 後面加入：

```ts
    if (type === "remote-terminal") title = t.remote_terminal_tab;
```

- [ ] **Step 6: 加渲染分支**

同一個檔案的渲染鏈（約 554 行開始的 `tab.type === "database" ? ... : tab.type === "design" ? ...`）。在鏈的**最後一個具名分支之後、`terminal` 的 fallback 之前**加入：

```ts
              ) : tab.type === "remote-terminal" ? (
                <RemoteTerminalView
                  tabId={tab.id}
                  connId={tab.remoteConnId ?? ""}
                  isActive={isActive}
                />
```

並在檔案頂端加 import：

```ts
import { RemoteTerminalView } from "./RemoteTerminalView";
```

**注意**：實際的鏈長什麼樣要先用 Read 看過再改——三元運算子鏈接錯地方會讓某個既有型別掉進錯的分支，而那不會編譯失敗。

- [ ] **Step 7: 跑測試確認轉綠**

Run: `npx vitest run src/components/tabTypeCoverage.test.ts`
Expected: PASS，5 個測試全過。

（`RemoteTerminalView` 這時還不存在，所以 `npx tsc -b` 會失敗——那是預期的，Task 3 才建立它。）

- [ ] **Step 8: Commit**

```bash
git add src/components/TabBar/index.tsx src/components/TerminalApp.tsx src/components/tabTypeCoverage.test.ts
git commit -m "feat(share): add the remote-terminal tab type

九個檔案會對 TabType 做分支，漏處理不會編譯失敗只會變成執行期怪現象
（標題空白、圖示不見）。覆蓋測試直接掃原始碼確認每個分支點都有處理。"
```

---

## Task 3: `RemoteTerminalView`

**Files:**
- Create: `src/components/RemoteTerminalView/index.tsx`
- Create: `src/components/RemoteTerminalView/index.css`
- Test: `src/components/RemoteTerminalView/index.test.tsx`

- [ ] **Step 1: 寫會紅的測試**

建立 `src/components/RemoteTerminalView/index.test.tsx`：

```tsx
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

// 事件訂閱的假實作：測試自己保留 callback，之後手動觸發。
const handlers: Record<string, (v: never) => void> = {};
function captureHandler(name: string) {
  return (connId: string, cb: (v: never) => void) => {
    handlers[`${name}:${connId}`] = cb;
    return Promise.resolve(() => {});
  };
}

const sendMock = vi.fn();

vi.mock("../../ipc/shareViewer", () => ({
  onShareViewerSas: captureHandler("sas"),
  onShareViewerGranted: captureHandler("granted"),
  onShareViewerData: captureHandler("data"),
  onShareViewerResync: captureHandler("resync"),
  onShareViewerControlChanged: captureHandler("control"),
  onShareViewerEnded: captureHandler("ended"),
  shareViewerSend: (...a: unknown[]) => sendMock(...a),
  shareViewerDisconnect: vi.fn().mockResolvedValue(undefined),
}));

// xterm 在 jsdom 下量不到尺寸，用假的。
const writeMock = vi.fn();
const clearMock = vi.fn();
vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    write = writeMock;
    clear = clearMock;
    open = vi.fn();
    dispose = vi.fn();
    onData = vi.fn();
    loadAddon = vi.fn();
  },
}));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit = vi.fn(); } }));

import { RemoteTerminalView } from "./index";

beforeEach(() => {
  for (const k of Object.keys(handlers)) delete handlers[k];
  writeMock.mockReset();
  clearMock.mockReset();
  sendMock.mockReset();
});

describe("RemoteTerminalView", () => {
  it("shows its own verification code for the user to read out", async () => {
    // 觀看端**必須**顯示自己算出的碼——那是要唸給對方聽的。主控端相反，
    // 那邊絕不顯示自己的碼（否則會照抄而不問對方）。兩邊不對稱是刻意的。
    render(<RemoteTerminalView tabId="t1" connId="c1" isActive />);
    await waitFor(() => expect(handlers["sas:c1"]).toBeDefined());

    handlers["sas:c1"]("4917" as never);

    expect(await screen.findByText("4917")).toBeInTheDocument();
  });

  it("does not send keystrokes while read-only", async () => {
    render(<RemoteTerminalView tabId="t1" connId="c2" isActive />);
    await waitFor(() => expect(handlers["granted:c2"]).toBeDefined());

    handlers["granted:c2"]({ mode: "read_only", cols: 80, rows: 24 } as never);

    // 伺服器端還有一道 may_send_input 檢查，但前端這層是給使用者的回饋：
    // 唯讀時按鍵**根本不送出**，而不是送了被拒絕。
    await waitFor(() => expect(screen.getByText(/唯讀|Read-only/)).toBeInTheDocument());
    expect(sendMock).not.toHaveBeenCalled();
  });

  it("clears the screen before a resync replay", async () => {
    // 漏掉的位元組可能截斷 ANSI 逃脫序列——帶著壞掉的畫面繼續是不會自己好的。
    render(<RemoteTerminalView tabId="t1" connId="c3" isActive />);
    await waitFor(() => expect(handlers["resync:c3"]).toBeDefined());

    handlers["resync:c3"](undefined as never);

    await waitFor(() => expect(clearMock).toHaveBeenCalled());
  });

  it("keeps the last screen and explains why the connection ended", async () => {
    render(<RemoteTerminalView tabId="t1" connId="c4" isActive />);
    await waitFor(() => expect(handlers["ended:c4"]).toBeDefined());

    handlers["ended:c4"]("host_stopped_sharing" as never);

    // 不清空畫面——最後看到的內容仍要能閱讀。
    expect(clearMock).not.toHaveBeenCalled();
    expect(
      await screen.findByText(/對方停止分享了|They stopped sharing/),
    ).toBeInTheDocument();
  });

  it("shows a human sentence for an unrecognised end reason", async () => {
    // spec 要求「不能有『未知錯誤』」。真的收到沒見過的 reason 時（例如
    // 對方是更新版），也要給一句人話而不是原始字串。
    render(<RemoteTerminalView tabId="t1" connId="c5" isActive />);
    await waitFor(() => expect(handlers["ended:c5"]).toBeDefined());

    handlers["ended:c5"]("something_from_the_future" as never);

    expect(screen.queryByText("something_from_the_future")).not.toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 跑測試確認會紅**

Run: `npx vitest run src/components/RemoteTerminalView`
Expected: FAIL——`Failed to resolve import "./index"`。

- [ ] **Step 3: 確認 xterm 的 import 路徑**

這個專案用的 xterm 套件名稱要實際確認（有 `xterm` 與 `@xterm/xterm` 兩種可能）：

```bash
grep -n "from \"@xterm/xterm\"\|from \"xterm\"\|addon-fit" src/components/TerminalView.tsx | head -4
```

**用查到的實際路徑**，並把測試裡 `vi.mock` 的路徑改成一致。

- [ ] **Step 4: 實作元件**

建立 `src/components/RemoteTerminalView/index.tsx`：

```tsx
import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import {
  onShareViewerControlChanged,
  onShareViewerData,
  onShareViewerEnded,
  onShareViewerGranted,
  onShareViewerResync,
  onShareViewerSas,
  shareViewerDisconnect,
  shareViewerSend,
} from "../../ipc/shareViewer";
import { useLocale } from "../../contexts/LocaleContext";
import type { Translations } from "../../lib/i18n";
import "./index.css";

interface Props {
  tabId: string;
  /** 2B-1 的觀看連線 id。所有 `share-viewer://*` 事件都掛在它上面。 */
  connId: string;
  isActive: boolean;
}

type Phase =
  | { kind: "waiting"; sas: string | null }
  | { kind: "live"; mode: string }
  | { kind: "ended"; reason: string };

export function RemoteTerminalView({ tabId, connId, isActive }: Props) {
  const { t } = useLocale();
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const [phase, setPhase] = useState<Phase>({ kind: "waiting", sas: null });

  // `phase` 要在事件 callback 裡讀到最新值，但那些 callback 只註冊一次——
  // 用 ref 避免стale closure（這個 repo 在 Tauri 事件監聽上踩過這個坑）。
  const phaseRef = useRef(phase);
  phaseRef.current = phase;

  useEffect(() => {
    if (!connId) return;
    const unlisteners: Array<() => void> = [];
    let disposed = false;

    const track = (p: Promise<() => void>) => {
      p.then((un) => {
        if (disposed) un();
        else unlisteners.push(un);
      });
    };

    track(onShareViewerSas(connId, (sas) => setPhase({ kind: "waiting", sas })));

    track(
      onShareViewerGranted(connId, ({ mode, cols, rows }) => {
        // 尺寸由主控端說了算——照它給的建立，不用自己的視窗大小。
        // `mode` 為空字串代表這是後續的 resize 通知，不是初次核准。
        if (mode) setPhase({ kind: "live", mode });
        const term = termRef.current;
        if (term && cols > 0 && rows > 0) {
          term.resize?.(cols, rows);
        }
      }),
    );

    track(
      onShareViewerData(connId, (b64) => {
        const bytes = atob(b64);
        const arr = new Uint8Array(bytes.length);
        for (let i = 0; i < bytes.length; i++) arr[i] = bytes.charCodeAt(i);
        termRef.current?.write(arr);
      }),
    );

    track(
      onShareViewerResync(connId, () => {
        // 清空再接全量重播。漏掉的位元組可能截斷 ANSI 逃脫序列，帶著壞掉
        // 的畫面繼續是不會自己好的。
        termRef.current?.clear();
      }),
    );

    track(
      onShareViewerControlChanged(connId, (mode) => {
        setPhase({ kind: "live", mode });
      }),
    );

    track(onShareViewerEnded(connId, (reason) => setPhase({ kind: "ended", reason })));

    return () => {
      disposed = true;
      for (const un of unlisteners) un();
    };
  }, [connId]);

  // xterm 的建立與銷毀。刻意跟事件訂閱分開——訂閱只依賴 connId，終端機
  // 只依賴掛載，兩者的生命週期不同。
  useEffect(() => {
    if (!hostRef.current) return;
    const term = new Terminal({ convertEol: false, cursorBlink: false });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(hostRef.current);
    termRef.current = term;

    const onData = term.onData((data: string) => {
      // **唯讀時按鍵根本不送出**，不是送了被伺服器拒絕。伺服器端還有一道
      // `may_send_input`，但那是安全邊界；這一層是給使用者的即時回饋。
      const p = phaseRef.current;
      if (p.kind === "live" && p.mode === "control") {
        void shareViewerSend(connId, data);
      }
    });

    return () => {
      onData?.dispose?.();
      term.dispose();
      termRef.current = null;
    };
  }, [connId]);

  // 分頁關閉時斷線。
  useEffect(() => {
    return () => {
      if (connId) void shareViewerDisconnect(connId);
    };
  }, [connId]);

  return (
    <div className="aiterm-remote-terminal" data-tab-id={tabId} data-active={isActive}>
      {phase.kind === "waiting" && (
        <div className="aiterm-remote-terminal__banner">
          <span>{t.remote_terminal_waiting_approval}</span>
          {phase.sas && (
            <>
              {/* 觀看端**必須**顯示自己算出的碼——那是要唸給對方聽的。
                  主控端相反：那邊絕不顯示自己的碼，否則使用者會照抄而不
                  問對方，人工核對變成自欺。兩邊不對稱是刻意的。 */}
              <strong className="aiterm-remote-terminal__sas">{phase.sas}</strong>
              <span className="aiterm-remote-terminal__hint">{t.remote_terminal_your_code}</span>
            </>
          )}
        </div>
      )}

      {phase.kind === "live" && phase.mode === "read_only" && (
        <div className="aiterm-remote-terminal__banner aiterm-remote-terminal__banner--readonly">
          {t.remote_terminal_read_only}
        </div>
      )}

      {phase.kind === "ended" && (
        <div className="aiterm-remote-terminal__banner aiterm-remote-terminal__banner--ended">
          {endReasonText(t, phase.reason)}
        </div>
      )}

      <div className="aiterm-remote-terminal__screen" ref={hostRef} />
    </div>
  );
}

/**
 * 把後端的 `EndReason` 字串轉成一句人話。
 *
 * spec 要求**不能有「未知錯誤」**。認不得的 reason（例如對方是更新版）
 * 也要給一句話，而不是把原始字串丟到畫面上。
 *
 * `t` 是具名的 `Translations` 型別，不是 `Record<string, string>`——用
 * 變數當 key 去索引具名型別，TypeScript 會擋，所以這裡先轉成
 * `Record<string, string>` 再查。這是刻意的 escape hatch，範圍限縮在這
 * 一個函式裡。
 */
function endReasonText(t: Translations, reason: string): string {
  const key = `remote_terminal_ended_${reason}`;
  const table = t as unknown as Record<string, string>;
  return table[key] ?? t.remote_terminal_ended_session_closed;
}
```

- [ ] **Step 5: 樣式**

建立 `src/components/RemoteTerminalView/index.css`：

```css
.aiterm-remote-terminal {
  display: flex;
  flex-direction: column;
  height: 100%;
  /* 跟 .aiterm-live-frame 同一個理由：overflow:hidden 會讓內容被瀏覽器
     捲出視野而變空白，clip 不會。這個 repo 為此debug 過一輪。 */
  overflow: clip;
}

.aiterm-remote-terminal__banner {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 12px;
  font-size: 13px;
  background: var(--aiterm-surface-2, #1e293b);
  border-bottom: 1px solid var(--aiterm-border, #334155);
}

.aiterm-remote-terminal__sas {
  font-family: var(--aiterm-mono, monospace);
  font-size: 22px;
  letter-spacing: 4px;
  color: var(--aiterm-accent, #22d3ee);
}

.aiterm-remote-terminal__hint {
  color: var(--aiterm-text-muted, #94a3b8);
}

.aiterm-remote-terminal__banner--readonly {
  background: var(--aiterm-warn-bg, #78350f33);
  color: var(--aiterm-warn-fg, #fbbf24);
}

.aiterm-remote-terminal__banner--ended {
  background: var(--aiterm-error-bg, #7f1d1d33);
  color: var(--aiterm-error-fg, #fca5a5);
}

.aiterm-remote-terminal__screen {
  flex: 1;
  min-height: 0;
  overflow: clip;
}
```

- [ ] **Step 6: 確認 `useLocale` 的實際介面**

```bash
grep -n "export function useLocale" -A 6 src/contexts/LocaleContext.tsx
```

確認它回傳的是 `{ t }` 還是別的形狀，並把元件裡的用法改成一致。若 `t` 的型別不是可索引的字串對映，`endReasonText` 的簽名要跟著調整（例如改成吃 `typeof t`）。

- [ ] **Step 7: 跑測試確認轉綠**

Run: `npx vitest run src/components/RemoteTerminalView`
Expected: PASS，5 個測試全過。

- [ ] **Step 8: 型別檢查**

Run: `npx tsc -b`
Expected: 沒有輸出。

**用 `npx tsc -b`，不要用 `tsc --noEmit`**——根 `tsconfig.json` 是 solution file（`"files": []`），`--noEmit` 什麼都不檢查而且永遠 exit 0。

- [ ] **Step 9: Commit**

```bash
git add src/components/RemoteTerminalView/ src/components/TerminalApp.tsx
git commit -m "feat(share): RemoteTerminalView

觀看端顯示自己算出的驗證碼（要唸給對方聽）——主控端相反，那邊絕不顯示。
唯讀時按鍵根本不送出，不是送了被拒絕。Resync 先清空再接全量重播。
連線結束保留最後畫面，八個 EndReason 各有一句人話。"
```

---

## 完成標準

- [ ] `npx vitest run src/components/RemoteTerminalView src/components/tabTypeCoverage.test.ts src/lib/i18n.remoteTerminal.test.ts` 全綠
- [ ] `npx tsc -b` 通過
- [ ] `cd src-tauri && cargo test` 仍全綠（這份計畫不動 Rust，這是防迴歸）
- [ ] 覆蓋測試的五項都過——那是「九個檔案沒漏掉」的證據

  注意：`npm run lint` 在這個 repo **本來就不綠**（199 個既有問題），**不是這個計畫造成的、不要順手修**。

**尚未具備**：分享按鈕、分享面板、同意視窗、連線對話框（2B-2b）、mDNS（2C）。2B-2a 結束時分頁型別存在且畫面會動，但**沒有入口能開出這種分頁**。
