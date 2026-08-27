# 遠端終端機共享 2B-2b：主控端 UI 與連線入口 — 實作計畫

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把前面四份計畫做好的後端接成使用者真的按得到的東西——分享按鈕、分享面板、同意視窗、連線對話框。**做完就能跟同事實際共用終端機了。**

**Architecture:** `ShareButton`/`SharePanel` 掛在 `TerminalView` 的工具列（它只在終端機分頁渲染，所以「非終端機分頁隱藏」自動成立）。`ConsentDialog` 必須掛在 `TerminalApp` 層——非作用中的分頁是 `visibility: hidden` + `pointerEvents: none`，掛在 `TerminalView` 裡的話，對方在非作用分頁發起連線時同意視窗看不見也點不到。

**Tech Stack:** React 19 / TypeScript / Vitest + Testing Library

**Spec:** `docs/superpowers/specs/2026-08-26-remote-terminal-sharing-2-ui-design.md`

**本計畫不含**：mDNS 自動發現（2C）。2B-2b 結束時**手動輸入 `host:port` 就能完整使用**——這正好落實 spec 裡「手動位址永遠是主路徑，mDNS 只是加速器」那條。

---

## 這個計畫最重要的一件事

同意視窗的設計是「**主控端輸入對方唸出來的 4 位驗證碼**」，而不是把碼顯示出來讓人按同意。這樣「不核對」在物理上做不到——你手上沒有那四個數字，只能開口問對方。

**後端（2A）已經把這件事做成結構性保證**：`sharePending()` 回傳的 `PendingRequest` 型別上就**沒有驗證碼欄位**，比對在 Rust 的 `decide()` 裡做。

所以這份計畫的 UI **不可能**顯示主控端的碼——它根本拿不到。這不是要你克制，是型別擋著。

**觀看端相反**（2B-2a 已完成）：它**必須**顯示自己算出的碼，因為那是要唸出來的。兩邊不對稱是這個設計能成立的原因。

---

## 一個對 spec 的刻意偏離，要先講清楚

spec 的「觀看方」流程寫：

> 2. 畫面顯示自己這端算出的 4 位驗證碼…同時顯示「等待對方同意」
> 3. **對方按同意後**，開一個 `remote-terminal` 分頁

**本計畫改成：輸入短碼／位址後就立刻開分頁**，等待與驗證碼顯示都在分頁裡（2B-2a 的 `RemoteTerminalView` 已經實作了 `waiting` 階段與 SAS 橫幅）。

理由：對方是人，可能離開座位、可能講電話，等待時間不可預期。用 modal 擋著整個 app 等一個人類回應是糟糕的設計——使用者應該能切去做別的事、回頭再看。spec 沒有明確要求「等待時必須是 modal」，只要求「顯示驗證碼與等待狀態」，分頁裡同樣滿足。

---

## 檔案結構

| 檔案 | 責任 | 動作 |
|---|---|---|
| `src/lib/i18n.ts` | 主控端字串（zh-TW ＋ en） | 修改 |
| `src/hooks/useShareHost.ts` | 主控端狀態：分享中/短碼/位址/觀看者 ＋ 事件訂閱 | 新增 |
| `src/hooks/useShareHost.test.ts` | hook 測試 | 新增 |
| `src/components/SharePanel/index.tsx` | 分享按鈕 ＋ 展開的面板 | 新增 |
| `src/components/SharePanel/index.css` | 樣式 | 新增 |
| `src/components/SharePanel/index.test.tsx` | 元件測試 | 新增 |
| `src/components/ConsentDialog/index.tsx` | 同意視窗（輸入對方唸的碼） | 新增 |
| `src/components/ConsentDialog/index.css` | 樣式 | 新增 |
| `src/components/ConsentDialog/index.test.tsx` | 元件測試 | 新增 |
| `src/components/ConnectDialog/index.tsx` | 觀看端：輸入短碼／位址 | 新增 |
| `src/components/ConnectDialog/index.css` | 樣式 | 新增 |
| `src/components/ConnectDialog/index.test.tsx` | 元件測試 | 新增 |
| `src/components/TerminalView.tsx` | 工具列插入 `SharePanel` | 修改 |
| `src/components/TerminalApp.tsx` | 掛 `ConsentDialog` ＋ `ConnectDialog` ＋ picker 分支 | 修改 |
| `src/components/NewTabPicker/tabCatalog.tsx` | 把 `remote-terminal` 的 `hidden` 拿掉 | 修改 |

---

## Task 1: i18n 字串

**Files:**
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: 加 zh-TW 字串**

在 `const zhTW = {` 區塊裡，找到 2B-2a 加的那組 `remote_terminal_*`（搜 `remote_terminal_tab:`），在**那一組後面**加入：

```ts
    // 遠端終端機共享——主控端
    share_button: "分享",
    share_button_tooltip: "把這個終端機分頁分享給同事",
    share_panel_title: "分享中",
    share_panel_code: "短碼",
    share_panel_address: "位址",
    share_panel_copy: "複製",
    share_panel_copied: "已複製",
    share_panel_firewall_hint: "系統可能會問你要不要允許連入連線，請按「允許」",
    share_panel_viewers: "觀看者",
    share_panel_no_viewers: "還沒有人連進來",
    share_panel_stop: "停止分享",
    share_panel_mode_read_only: "唯讀",
    share_panel_mode_control: "控制中",
    share_panel_revoke_control: "收回控制權",
    share_panel_kick: "中斷連線",
    // 同意視窗
    consent_title: "{name} 想連進「{tab}」",
    consent_prompt: "請對方唸出他畫面上的 4 位數：",
    consent_warning: "對不上就是有人在中間，請拒絕",
    consent_name_unverified: "這個名字是對方自報的，未經驗證",
    consent_deny: "拒絕",
    consent_read_only: "只能看",
    consent_control: "可以控制",
    consent_code_mismatch: "驗證碼不符，已拒絕這次連線",
    consent_control_taken: "控制權已經給了別人。要讓這位加入，請改選「只能看」，或先收回控制權。",
    consent_request_gone: "這個請求已經失效了",
    // 連線對話框
    connect_title: "連線到同事的終端機",
    connect_code_label: "輸入同事給你的 6 位數",
    connect_name_label: "你的名字（對方會看到）",
    connect_manual_toggle: "或直接輸入位址",
    connect_manual_label: "位址",
    connect_manual_placeholder: "192.168.1.33:47823",
    connect_submit: "連線",
    connect_cancel: "取消",
    connect_not_found: "在這個網路上找不到這組編號。可能已失效，或你們不在同一個網路。",
    connect_manual_prompt: "請對方唸出他畫面上的位址：",
    connect_bad_address: "位址格式不對，應該像 192.168.1.33:47823",
    connect_failed: "連不上：{error}",
```

- [ ] **Step 2: 加 en 字串**

在 `const enRaw = {` 區塊裡對應的位置（搜第二個 `remote_terminal_tab:`）加入：

```ts
    // Remote terminal sharing — host side
    share_button: "Share",
    share_button_tooltip: "Share this terminal tab with a colleague",
    share_panel_title: "Sharing",
    share_panel_code: "Code",
    share_panel_address: "Address",
    share_panel_copy: "Copy",
    share_panel_copied: "Copied",
    share_panel_firewall_hint: "Your system may ask whether to allow incoming connections — choose Allow",
    share_panel_viewers: "Viewers",
    share_panel_no_viewers: "Nobody has connected yet",
    share_panel_stop: "Stop sharing",
    share_panel_mode_read_only: "Read-only",
    share_panel_mode_control: "In control",
    share_panel_revoke_control: "Take back control",
    share_panel_kick: "Disconnect",
    // Consent dialog
    consent_title: "{name} wants to connect to \"{tab}\"",
    consent_prompt: "Ask them to read out the 4 digits on their screen:",
    consent_warning: "If they do not match, someone is in the middle — deny it",
    consent_name_unverified: "This name is self-reported and unverified",
    consent_deny: "Deny",
    consent_read_only: "View only",
    consent_control: "Allow control",
    consent_code_mismatch: "The code did not match — this connection was denied",
    consent_control_taken: "Someone else already has control. To let this person in, choose View only, or take back control first.",
    consent_request_gone: "That request is no longer valid",
    // Connect dialog
    connect_title: "Connect to a colleague's terminal",
    connect_code_label: "Enter the 6-digit code they gave you",
    connect_name_label: "Your name (they will see this)",
    connect_manual_toggle: "Or enter an address directly",
    connect_manual_label: "Address",
    connect_manual_placeholder: "192.168.1.33:47823",
    connect_submit: "Connect",
    connect_cancel: "Cancel",
    connect_not_found: "No such code on this network. It may have expired, or you may not be on the same network.",
    connect_manual_prompt: "Ask them to read out the address on their screen:",
    connect_bad_address: "That address does not look right — it should look like 192.168.1.33:47823",
    connect_failed: "Could not connect: {error}",
```

- [ ] **Step 3: 擴充既有的語系同步測試**

`src/lib/i18n.remoteTerminal.test.ts`（2B-2a 建立的）裡，把那個同步測試的前綴改成涵蓋這批新字串：

```ts
  it("keeps the two locales in sync for sharing strings", () => {
    // 語系漂移是這個 repo 記過的坑：只加一邊，另一邊會靜默 fallback 或空白。
    const prefixes = ["remote_terminal_", "share_", "consent_", "connect_"];
    const pick = (loc: "zh-TW" | "en") =>
      Object.keys(translations[loc])
        .filter((k) => prefixes.some((p) => k.startsWith(p)))
        .sort();
    expect(pick("zh-TW")).toEqual(pick("en"));
  });
```

**注意**：這個測試會連帶掃到既有的 `share_*` 開頭字串（如果有的話）。跑起來若失敗且失敗的是**既有**字串，那是既有的語系漂移——**回報給我，不要順手修**，那不在這個計畫的範圍。

- [ ] **Step 4: 跑測試**

Run: `npx vitest run src/lib/i18n.remoteTerminal.test.ts`
Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add src/lib/i18n.ts src/lib/i18n.remoteTerminal.test.ts
git commit -m "feat(share): i18n strings for the host UI and connect dialog"
```

---

## Task 2: `useShareHost` hook

把主控端的狀態集中在一個 hook 裡，元件只負責畫。這樣狀態邏輯能單獨測試，不用 render 整棵樹。

**Files:**
- Create: `src/hooks/useShareHost.ts`
- Test: `src/hooks/useShareHost.test.ts`

- [ ] **Step 1: 寫會紅的測試**

建立 `src/hooks/useShareHost.test.ts`：

```ts
import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";

const shareStartMock = vi.fn();
const shareStopMock = vi.fn();
const shareStatusMock = vi.fn();
const shareViewersMock = vi.fn();
const shareKickMock = vi.fn();
const shareRevokeMock = vi.fn();
let viewersChangedCb: (() => void) | null = null;

vi.mock("../ipc/share", () => ({
  shareStart: (...a: unknown[]) => shareStartMock(...a),
  shareStop: (...a: unknown[]) => shareStopMock(...a),
  shareStatus: (...a: unknown[]) => shareStatusMock(...a),
  shareViewers: (...a: unknown[]) => shareViewersMock(...a),
  shareKick: (...a: unknown[]) => shareKickMock(...a),
  shareRevokeControl: (...a: unknown[]) => shareRevokeMock(...a),
  onShareViewersChanged: (cb: () => void) => {
    viewersChangedCb = cb;
    return Promise.resolve(() => {});
  },
}));

import { useShareHost } from "./useShareHost";

beforeEach(() => {
  shareStartMock.mockReset().mockResolvedValue({ sharing: true, code: "559207", port: 47823 });
  shareStopMock.mockReset().mockResolvedValue({ sharing: false, code: null, port: null });
  shareStatusMock.mockReset().mockResolvedValue({ sharing: false, code: null, port: null });
  shareViewersMock.mockReset().mockResolvedValue([]);
  shareKickMock.mockReset().mockResolvedValue(undefined);
  shareRevokeMock.mockReset().mockResolvedValue(undefined);
  viewersChangedCb = null;
});

describe("useShareHost", () => {
  it("starts out not sharing", async () => {
    const { result } = renderHook(() => useShareHost("tab-1"));
    await waitFor(() => expect(shareStatusMock).toHaveBeenCalledWith("tab-1"));
    expect(result.current.sharing).toBe(false);
    expect(result.current.code).toBeNull();
  });

  it("exposes the code and address after starting", async () => {
    const { result } = renderHook(() => useShareHost("tab-1"));
    await act(async () => {
      await result.current.start();
    });
    expect(shareStartMock).toHaveBeenCalledWith("tab-1");
    expect(result.current.code).toBe("559207");
    expect(result.current.port).toBe(47823);
  });

  it("re-reads the viewer list when the backend says it changed", async () => {
    // 事件刻意不帶內容——收到就重讀，避免推播的資料跟查詢的資料對不上。
    const { result } = renderHook(() => useShareHost("tab-1"));
    await act(async () => {
      await result.current.start();
    });

    shareViewersMock.mockResolvedValue([
      { viewerId: "v1", displayName: "Alice", mode: "control" },
    ]);
    await act(async () => {
      viewersChangedCb?.();
    });

    await waitFor(() => expect(result.current.viewers).toHaveLength(1));
    expect(result.current.viewers[0].displayName).toBe("Alice");
  });

  it("clears its state when sharing stops", async () => {
    const { result } = renderHook(() => useShareHost("tab-1"));
    await act(async () => {
      await result.current.start();
    });
    await act(async () => {
      await result.current.stop();
    });
    expect(result.current.sharing).toBe(false);
    expect(result.current.code).toBeNull();
    expect(result.current.viewers).toEqual([]);
  });
});
```

- [ ] **Step 2: 跑測試確認會紅**

Run: `npx vitest run src/hooks/useShareHost.test.ts`
Expected: FAIL——`Failed to resolve import "./useShareHost"`。

- [ ] **Step 3: 實作**

建立 `src/hooks/useShareHost.ts`：

```ts
import { useCallback, useEffect, useRef, useState } from "react";
import {
  onShareViewersChanged,
  shareKick,
  shareRevokeControl,
  shareStart,
  shareStatus,
  shareStop,
  shareViewers,
  type Viewer,
} from "../ipc/share";

/**
 * 一個終端機分頁的分享狀態。
 *
 * **注意這裡沒有任何驗證碼相關的狀態。** 主控端的 4 位碼永遠不離開 Rust
 * ——同意視窗要使用者輸入對方唸的碼，比對在 `share_approve` 裡做。前端
 * 拿不到那個值，所以不可能顯示它。見 `src/ipc/share.ts` 的 `PendingRequest`。
 */
export function useShareHost(tabId: string) {
  const [sharing, setSharing] = useState(false);
  const [code, setCode] = useState<string | null>(null);
  const [port, setPort] = useState<number | null>(null);
  const [viewers, setViewers] = useState<Viewer[]>([]);

  // 事件 callback 只註冊一次，但要讀到最新的 tabId——用 ref 避免 stale
  // closure（這個 repo 在 Tauri 事件監聽上踩過這個坑）。
  const tabIdRef = useRef(tabId);
  tabIdRef.current = tabId;

  const refreshViewers = useCallback(async () => {
    const list = await shareViewers(tabIdRef.current);
    setViewers(list);
  }, []);

  // 掛載時問一次目前狀態——分享是跨分頁切換存活的，重新渲染不該讓面板忘記。
  useEffect(() => {
    let alive = true;
    void shareStatus(tabId).then((s) => {
      if (!alive) return;
      setSharing(s.sharing);
      setCode(s.code);
      setPort(s.port);
      if (s.sharing) void refreshViewers();
    });
    return () => {
      alive = false;
    };
  }, [tabId, refreshViewers]);

  // 觀看者變動的推播。事件不帶內容，收到就重讀。
  useEffect(() => {
    let un: (() => void) | null = null;
    let disposed = false;
    void onShareViewersChanged(() => {
      void refreshViewers();
    }).then((f) => {
      if (disposed) f();
      else un = f;
    });
    return () => {
      disposed = true;
      un?.();
    };
  }, [refreshViewers]);

  const start = useCallback(async () => {
    const s = await shareStart(tabIdRef.current);
    setSharing(s.sharing);
    setCode(s.code);
    setPort(s.port);
  }, []);

  const stop = useCallback(async () => {
    await shareStop(tabIdRef.current);
    setSharing(false);
    setCode(null);
    setPort(null);
    setViewers([]);
  }, []);

  const kick = useCallback(
    async (viewerId: string) => {
      await shareKick(tabIdRef.current, viewerId);
      await refreshViewers();
    },
    [refreshViewers],
  );

  const revokeControl = useCallback(async () => {
    await shareRevokeControl(tabIdRef.current);
    await refreshViewers();
  }, [refreshViewers]);

  return { sharing, code, port, viewers, start, stop, kick, revokeControl };
}
```

- [ ] **Step 4: 跑測試確認轉綠**

Run: `npx vitest run src/hooks/useShareHost.test.ts`
Expected: PASS，4 個測試全過。

- [ ] **Step 5: Commit**

```bash
git add src/hooks/useShareHost.ts src/hooks/useShareHost.test.ts
git commit -m "feat(share): useShareHost hook

狀態集中在 hook 裡，元件只負責畫。刻意沒有任何驗證碼相關的狀態——
主控端的碼永遠不離開 Rust。"
```

---

## Task 3: `SharePanel`（按鈕 ＋ 面板）

**Files:**
- Create: `src/components/SharePanel/index.tsx`
- Create: `src/components/SharePanel/index.css`
- Test: `src/components/SharePanel/index.test.tsx`
- Modify: `src/components/TerminalView.tsx`

- [ ] **Step 1: 寫會紅的測試**

建立 `src/components/SharePanel/index.test.tsx`：

```tsx
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const startMock = vi.fn();
const stopMock = vi.fn();
const kickMock = vi.fn();
const revokeMock = vi.fn();
let hookState = {
  sharing: false,
  code: null as string | null,
  port: null as number | null,
  viewers: [] as Array<{ viewerId: string; displayName: string; mode: string }>,
};

vi.mock("../../hooks/useShareHost", () => ({
  useShareHost: () => ({
    ...hookState,
    start: startMock,
    stop: stopMock,
    kick: kickMock,
    revokeControl: revokeMock,
  }),
}));

vi.mock("../../contexts/LocaleContext", async () => {
  const { translations } = await import("../../lib/i18n");
  return { useLocale: () => ({ t: translations["zh-TW"], locale: "zh-TW", setLocale: () => {} }) };
});

import { SharePanel } from "./index";

beforeEach(() => {
  startMock.mockReset().mockResolvedValue(undefined);
  stopMock.mockReset().mockResolvedValue(undefined);
  kickMock.mockReset().mockResolvedValue(undefined);
  revokeMock.mockReset().mockResolvedValue(undefined);
  hookState = { sharing: false, code: null, port: null, viewers: [] };
});

describe("SharePanel", () => {
  it("starts sharing when the button is pressed", async () => {
    render(<SharePanel tabId="t1" />);
    await userEvent.click(screen.getByRole("button", { name: /分享/ }));
    expect(startMock).toHaveBeenCalled();
  });

  it("shows the code and the address together", async () => {
    // 兩個都要顯示：對方自動發現失敗時，唸位址就好，不用回頭找。
    hookState = { sharing: true, code: "559207", port: 47823, viewers: [] };
    render(<SharePanel tabId="t1" />);
    await userEvent.click(screen.getByRole("button", { name: /分享/ }));

    expect(await screen.findByText("559207")).toBeInTheDocument();
    expect(screen.getByText(/47823/)).toBeInTheDocument();
  });

  it("warns about the firewall prompt before it appears", async () => {
    // 使用者被系統彈窗嚇到而反射性按拒絕，是這個功能最可惜的失敗方式。
    hookState = { sharing: true, code: "559207", port: 47823, viewers: [] };
    render(<SharePanel tabId="t1" />);
    await userEvent.click(screen.getByRole("button", { name: /分享/ }));

    expect(await screen.findByText(/允許連入連線/)).toBeInTheDocument();
  });

  it("lists viewers with their mode and lets the host disconnect them", async () => {
    hookState = {
      sharing: true,
      code: "559207",
      port: 47823,
      viewers: [{ viewerId: "v1", displayName: "Alice", mode: "control" }],
    };
    render(<SharePanel tabId="t1" />);
    await userEvent.click(screen.getByRole("button", { name: /分享/ }));

    expect(await screen.findByText("Alice")).toBeInTheDocument();
    expect(screen.getByText(/控制中/)).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: /中斷連線/ }));
    expect(kickMock).toHaveBeenCalledWith("v1");
  });

  it("never displays a verification code", async () => {
    // 主控端的 4 位碼根本不會到前端——同意視窗要使用者輸入對方唸的碼。
    // 這個測試守著「不要哪天為了『方便』把碼加進面板」。
    hookState = {
      sharing: true,
      code: "559207",
      port: 47823,
      viewers: [{ viewerId: "v1", displayName: "Alice", mode: "read_only" }],
    };
    const { container } = render(<SharePanel tabId="t1" />);
    await userEvent.click(screen.getByRole("button", { name: /分享/ }));

    // 6 位短碼是要給對方輸入的，可以顯示；4 位驗證碼不行。
    await waitFor(() => expect(screen.getByText("559207")).toBeInTheDocument());
    expect(container.textContent).not.toMatch(/\b\d{4}\b(?!\d)/);
  });
});
```

- [ ] **Step 2: 跑測試確認會紅**

Run: `npx vitest run src/components/SharePanel`
Expected: FAIL——`Failed to resolve import "./index"`。

- [ ] **Step 3: 實作**

建立 `src/components/SharePanel/index.tsx`：

```tsx
import { useState } from "react";
import { useShareHost } from "../../hooks/useShareHost";
import { useLocale } from "../../contexts/LocaleContext";
import { LinkIcon } from "../Icons";
import "./index.css";

interface Props {
  tabId: string;
}

/**
 * 分享按鈕與展開的面板。
 *
 * 掛在 `TerminalView` 的工具列——那個元件只在終端機分頁渲染，所以
 * spec 的「非終端機分頁隱藏」自動成立，不需要額外的型別判斷。
 *
 * **這個元件不可能顯示主控端的 4 位驗證碼**：那個值根本不會到前端
 * （見 `src/ipc/share.ts` 的 `PendingRequest`）。面板上的 6 位短碼是
 * 另一回事——那是要給對方輸入的，本來就該顯示。
 */
export function SharePanel({ tabId }: Props) {
  const { t } = useLocale();
  const { sharing, code, port, lanAddress, viewers, start, stop, kick, revokeControl } =
    useShareHost(tabId);
  const [open, setOpen] = useState(false);
  const [copied, setCopied] = useState(false);

  // 位址由後端提供（見 Task 3 Step 5）。前端查不到區網 IP——`hostname()`
  // 回的是主機名稱不是位址，而使用者要唸給同事的是 `192.168.1.33:47823`。
  const address = lanAddress && port ? `${lanAddress}:${port}` : null;

  async function onButtonClick() {
    if (!sharing) await start();
    setOpen((v) => !v);
  }

  async function onCopy() {
    if (!address) return;
    await navigator.clipboard.writeText(address);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1500);
  }

  return (
    <span className="aiterm-share">
      <button
        className={`aiterm-btn aiterm-btn--secondary aiterm-btn--sm ${sharing ? "aiterm-share__btn--on" : ""}`}
        title={t.share_button_tooltip}
        onClick={(e) => {
          e.stopPropagation();
          void onButtonClick();
        }}
        style={{ display: "flex", alignItems: "center", gap: "6px" }}
      >
        <LinkIcon size={14} />
        <span>{t.share_button}</span>
      </button>

      {open && sharing && (
        <div className="aiterm-share__panel" onClick={(e) => e.stopPropagation()}>
          <div className="aiterm-share__title">{t.share_panel_title}</div>

          <div className="aiterm-share__row">
            <span className="aiterm-share__label">{t.share_panel_code}</span>
            <strong className="aiterm-share__code">{code}</strong>
          </div>

          <div className="aiterm-share__row">
            <span className="aiterm-share__label">{t.share_panel_address}</span>
            <span className="aiterm-share__addr">{address}</span>
            <button
              className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
              onClick={() => void onCopy()}
            >
              {copied ? t.share_panel_copied : t.share_panel_copy}
            </button>
          </div>

          {/* 系統詢問就是這一刻跳出來的——先講，免得使用者被嚇到按拒絕。 */}
          <div className="aiterm-share__hint">⚠️ {t.share_panel_firewall_hint}</div>

          <div className="aiterm-share__viewers">
            <div className="aiterm-share__label">
              {t.share_panel_viewers}（{viewers.length}）
            </div>
            {viewers.length === 0 && (
              <div className="aiterm-share__empty">{t.share_panel_no_viewers}</div>
            )}
            {viewers.map((v) => (
              <div key={v.viewerId} className="aiterm-share__viewer">
                <span className="aiterm-share__viewer-name">{v.displayName}</span>
                <span className="aiterm-share__viewer-mode">
                  {v.mode === "control" ? t.share_panel_mode_control : t.share_panel_mode_read_only}
                </span>
                {v.mode === "control" && (
                  <button
                    className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                    onClick={() => void revokeControl()}
                  >
                    {t.share_panel_revoke_control}
                  </button>
                )}
                <button
                  className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                  onClick={() => void kick(v.viewerId)}
                >
                  {t.share_panel_kick}
                </button>
              </div>
            ))}
          </div>

          <button
            className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm aiterm-share__stop"
            onClick={() => {
              void stop();
              setOpen(false);
            }}
          >
            {t.share_panel_stop}
          </button>
        </div>
      )}
    </span>
  );
}

```

- [ ] **Step 4: 樣式**

建立 `src/components/SharePanel/index.css`：

```css
.aiterm-share {
  position: relative;
  display: inline-flex;
}

.aiterm-share__btn--on {
  border-color: var(--aiterm-accent, #22d3ee);
  color: var(--aiterm-accent, #22d3ee);
}

.aiterm-share__panel {
  position: absolute;
  top: calc(100% + 6px);
  right: 0;
  z-index: 40;
  width: 320px;
  padding: 12px;
  border-radius: 8px;
  background: var(--aiterm-surface-2, #1e293b);
  border: 1px solid var(--aiterm-border, #334155);
  box-shadow: 0 8px 24px #0008;
  font-size: 13px;
}

.aiterm-share__title {
  font-weight: 600;
  margin-bottom: 10px;
}

.aiterm-share__row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}

.aiterm-share__label {
  color: var(--aiterm-text-muted, #94a3b8);
  min-width: 44px;
}

.aiterm-share__code {
  font-family: var(--aiterm-mono, monospace);
  font-size: 20px;
  letter-spacing: 3px;
  color: var(--aiterm-accent, #22d3ee);
}

.aiterm-share__addr {
  font-family: var(--aiterm-mono, monospace);
  flex: 1;
}

.aiterm-share__hint {
  margin: 10px 0;
  padding: 8px;
  border-radius: 6px;
  background: var(--aiterm-warn-bg, #78350f33);
  color: var(--aiterm-warn-fg, #fbbf24);
  line-height: 1.5;
}

.aiterm-share__viewers {
  border-top: 1px solid var(--aiterm-border, #334155);
  padding-top: 10px;
  margin-bottom: 10px;
}

.aiterm-share__empty {
  color: var(--aiterm-text-muted, #94a3b8);
  padding: 6px 0;
}

.aiterm-share__viewer {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 0;
}

.aiterm-share__viewer-name {
  flex: 1;
}

.aiterm-share__viewer-mode {
  color: var(--aiterm-text-muted, #94a3b8);
}

.aiterm-share__stop {
  width: 100%;
}
```

- [ ] **Step 5: 接進 `TerminalView` 的工具列**

`src/components/TerminalView.tsx`，找到「指令書籤」那顆按鈕（搜 `t.bookmarks_title`，約 1695-1705 行）。在**那顆按鈕之前**插入：

```tsx
          <SharePanel tabId={tabId} />
```

並在檔案頂端加 import：

```tsx
import { SharePanel } from "./SharePanel";
```

**確認 `tabId` 這個變數在該位置的 scope 裡拿得到**——`TerminalView` 的 props 有 `tabId`（見第 195 行的解構），但要確認它在 JSX 那一段是可見的。拿不到就回報，不要自己傳別的值。

- [ ] **Step 6: 後端提供區網位址**

面板要顯示 `192.168.1.33:47823` 給對方輸入，但**前端查不到區網 IP**——瀏覽器環境沒有這個能力，`hostname()` 回的是主機名稱不是位址。Rust 那邊查得到。

`src-tauri/src/commands/share.rs`，`ShareStatus` 加一個欄位：

```rust
    /// 這台機器在區網上的位址（不含 port），給對方手動輸入用。
    ///
    /// 查不到時是 `None`——那不是錯誤，面板會退成只顯示 port，使用者自己
    /// 知道 IP。2C 的 mDNS 上線後多數情況也不需要手動輸入。
    pub lan_address: Option<String>,
```

同一個檔案加一個查詢函式：

```rust
/// 盡力問出這台機器的區網位址。查不到回 `None`。
///
/// 用系統指令而不是列舉網路介面，是因為「哪一張介面才是使用者實際連著的
/// 那張」在多網卡機器上很難判斷，而系統自己知道。查不到不影響功能——
/// 面板會退成只顯示 port。
fn lan_address() -> Option<String> {
    #[cfg(target_os = "macos")]
    let cmd = "ipconfig getifaddr en0 2>/dev/null || ipconfig getifaddr en1 2>/dev/null";
    #[cfg(target_os = "linux")]
    let cmd = "hostname -I 2>/dev/null | awk '{print $1}'";
    #[cfg(target_os = "windows")]
    let cmd = "";

    #[cfg(target_os = "windows")]
    {
        // Windows 沒有簡短的等價指令，而 PowerShell 啟動成本高。留給使用者
        // 自己輸入——面板只顯示 port。
        let _ = cmd;
        return None;
    }

    #[cfg(not(target_os = "windows"))]
    {
        let out = std::process::Command::new("sh").arg("-c").arg(cmd).output().ok()?;
        let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
        if s.is_empty() { None } else { Some(s) }
    }
}
```

`share_start` 與 `share_status` 兩處建構 `ShareStatus` 時都填上 `lan_address: lan_address()`；`share_stop` 那處填 `lan_address: None`。

前端 `src/ipc/share.ts` 的 `ShareStatus` 介面加：

```ts
  /** 這台機器的區網位址（不含 port）。查不到時是 null，面板退成只顯示 port。 */
  lanAddress: string | null;
```

`src/hooks/useShareHost.ts` 加對應的 state（`lanAddress`），跟 `code`/`port` 一起設與清除，並加進回傳物件。

**這一步會動到 Rust**，所以你不能自己編譯——寫完回報，我來跑 `cargo test`。

- [ ] **Step 7: 跑測試確認轉綠**

Run: `npx vitest run src/components/SharePanel`
Expected: PASS，5 個測試全過。

- [ ] **Step 8: Commit**

```bash
git add src/components/SharePanel/ src/components/TerminalView.tsx
git commit -m "feat(share): share button and panel

短碼與位址同時顯示——對方自動發現失敗時唸位址就好。防火牆提醒放在這裡，
因為系統詢問就是按下分享那一刻跳出來的。"
```

---

## Task 4: `ConsentDialog`

**這是整個功能最關鍵的畫面。**

**Files:**
- Create: `src/components/ConsentDialog/index.tsx`
- Create: `src/components/ConsentDialog/index.css`
- Test: `src/components/ConsentDialog/index.test.tsx`
- Modify: `src/components/TerminalApp.tsx`

- [ ] **Step 1: 寫會紅的測試**

建立 `src/components/ConsentDialog/index.test.tsx`：

```tsx
import { describe, expect, it, vi, beforeEach } from "vitest";
import { act } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const approveMock = vi.fn();
const denyMock = vi.fn();
let pendingCb: ((p: { requestId: string; tabId: string; displayName: string }) => void) | null = null;

vi.mock("../../ipc/share", () => ({
  shareApprove: (...a: unknown[]) => approveMock(...a),
  shareDeny: (...a: unknown[]) => denyMock(...a),
  onSharePendingRequest: (cb: (p: never) => void) => {
    pendingCb = cb as never;
    return Promise.resolve(() => {});
  },
}));

vi.mock("../../contexts/LocaleContext", async () => {
  const { translations } = await import("../../lib/i18n");
  return { useLocale: () => ({ t: translations["zh-TW"], locale: "zh-TW", setLocale: () => {} }) };
});

import { ConsentDialog } from "./index";

const TABS = [{ id: "t1", title: "Claude Code" }];

beforeEach(() => {
  approveMock.mockReset().mockResolvedValue({ kind: "approved", viewerId: "v1" });
  denyMock.mockReset().mockResolvedValue(undefined);
  pendingCb = null;
});

async function arrive() {
  await vi.waitFor(() => expect(pendingCb).toBeTruthy());
  await act(async () => {
    pendingCb!({ requestId: "r1", tabId: "t1", displayName: "Alice" });
  });
}

describe("ConsentDialog", () => {
  it("stays out of the way until a request arrives", () => {
    render(<ConsentDialog tabs={TABS} />);
    expect(screen.queryByText(/想連進/)).not.toBeInTheDocument();
  });

  it("names who is asking and which tab", async () => {
    render(<ConsentDialog tabs={TABS} />);
    await arrive();
    expect(screen.getByText(/Alice/)).toBeInTheDocument();
    expect(screen.getByText(/Claude Code/)).toBeInTheDocument();
  });

  it("never shows a verification code of its own", async () => {
    // **這是整個設計的關鍵。** 主控端看得到自己的碼就會照抄而不問對方，
    // 人工核對變成自欺。後端根本不送那個值過來（PendingRequest 型別上就
    // 沒有），這個測試守著「不要哪天為了『方便』把它加回來」。
    render(<ConsentDialog tabs={TABS} />);
    await arrive();
    // 畫面上不該有任何 4 位數字——那會是「答案」，使用者會照抄而不問對方。
    expect(document.body.textContent).not.toMatch(/\b\d{4}\b(?!\d)/);
  });

  it("sends the typed code to the backend rather than comparing here", async () => {
    render(<ConsentDialog tabs={TABS} />);
    await arrive();

    await userEvent.type(screen.getByRole("textbox"), "4917");
    await userEvent.click(screen.getByRole("button", { name: /可以控制/ }));

    expect(approveMock).toHaveBeenCalledWith("r1", "control", "4917");
  });

  it("tells the host when the code did not match, and closes", async () => {
    // 輸錯直接拒絕，不給重試——攻擊者只有 1/10000 的一發機會，給重試等於
    // 送他一萬次。
    approveMock.mockResolvedValue({ kind: "codeMismatch" });
    render(<ConsentDialog tabs={TABS} />);
    await arrive();

    await userEvent.type(screen.getByRole("textbox"), "1234");
    await userEvent.click(screen.getByRole("button", { name: /只能看/ }));

    expect(await screen.findByText(/驗證碼不符/)).toBeInTheDocument();
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
  });

  it("explains when control is already taken and keeps the dialog open", async () => {
    // 這種情況請求還在（後端會放回待審），主控端可以改用唯讀重新裁決。
    approveMock.mockResolvedValue({ kind: "controlTaken" });
    render(<ConsentDialog tabs={TABS} />);
    await arrive();

    await userEvent.type(screen.getByRole("textbox"), "4917");
    await userEvent.click(screen.getByRole("button", { name: /可以控制/ }));

    expect(await screen.findByText(/控制權已經給了別人/)).toBeInTheDocument();
    expect(screen.getByRole("textbox")).toBeInTheDocument();
  });

  it("denies without needing a code", async () => {
    render(<ConsentDialog tabs={TABS} />);
    await arrive();
    await userEvent.click(screen.getByRole("button", { name: /拒絕/ }));
    expect(denyMock).toHaveBeenCalledWith("r1");
  });
});
```

- [ ] **Step 2: 跑測試確認會紅**

Run: `npx vitest run src/components/ConsentDialog`
Expected: FAIL——`Failed to resolve import "./index"`。

- [ ] **Step 3: 實作**

建立 `src/components/ConsentDialog/index.tsx`：

```tsx
import { useEffect, useState } from "react";
import { onSharePendingRequest, shareApprove, shareDeny, type PendingRequest } from "../../ipc/share";
import { useLocale } from "../../contexts/LocaleContext";
import "./index.css";

interface Props {
  /** 分頁清單，用來把 `tabId` 換成使用者看得懂的標題。 */
  tabs: Array<{ id: string; title: string }>;
}

/**
 * 同意視窗：有人要連進來時跳出來。
 *
 * **必須掛在 `TerminalApp` 層，不能在 `TerminalView` 裡。** 非作用中的分頁
 * 是用 `visibility: hidden` + `pointerEvents: none` 隱藏的，而連線請求可能
 * 來自任何一個分享中的分頁——掛在 `TerminalView` 裡的話，對方在非作用分頁
 * 發起連線時，這個視窗看不見也點不到。
 *
 * **這個視窗絕不顯示主控端自己算出的 4 位驗證碼。** 使用者必須跟對方口頭
 * 核對、把聽到的數字打進來。後端根本不送那個值過來（`PendingRequest` 型別
 * 上就沒有），所以這不是「UI 選擇不顯示」，是拿不到。
 *
 * 若碼顯示在這裡，使用者會照抄畫面上的數字而不問對方，人工核對變成自欺，
 * 而那次口頭核對正是整個防中間人保證的最後一哩。
 */
export function ConsentDialog({ tabs }: Props) {
  const { t } = useLocale();
  const [req, setReq] = useState<PendingRequest | null>(null);
  const [typed, setTyped] = useState("");
  const [outcome, setOutcome] = useState<string | null>(null);

  useEffect(() => {
    let un: (() => void) | null = null;
    let disposed = false;
    void onSharePendingRequest((p) => {
      setReq(p);
      setTyped("");
      setOutcome(null);
    }).then((f) => {
      if (disposed) f();
      else un = f;
    });
    return () => {
      disposed = true;
      un?.();
    };
  }, []);

  if (!req) return null;

  const tabTitle = tabs.find((x) => x.id === req.tabId)?.title ?? req.tabId;
  const title = t.consent_title.replace("{name}", req.displayName).replace("{tab}", tabTitle);

  async function decide(mode: "read_only" | "control") {
    const d = await shareApprove(req!.requestId, mode, typed);
    switch (d.kind) {
      case "approved":
        setReq(null);
        return;
      case "codeMismatch":
        // 後端已經拒絕了這筆請求，不給重試。關掉輸入框但留著訊息。
        setOutcome(t.consent_code_mismatch);
        return;
      case "controlTaken":
        // 請求還在，主控端可以改選「只能看」。
        setOutcome(t.consent_control_taken);
        return;
      case "requestGone":
        setOutcome(t.consent_request_gone);
        return;
    }
  }

  const closed = outcome === t.consent_code_mismatch || outcome === t.consent_request_gone;

  return (
    <div className="aiterm-consent__backdrop">
      <div className="aiterm-consent" role="dialog" aria-modal="true">
        <div className="aiterm-consent__title">{title}</div>
        <div className="aiterm-consent__unverified">{t.consent_name_unverified}</div>

        {!closed && (
          <>
            <label className="aiterm-consent__prompt" htmlFor="aiterm-consent-code">
              {t.consent_prompt}
            </label>
            <input
              id="aiterm-consent-code"
              className="aiterm-consent__input"
              type="text"
              inputMode="numeric"
              maxLength={4}
              autoFocus
              value={typed}
              onChange={(e) => setTyped(e.target.value.replace(/\D/g, "").slice(0, 4))}
            />
            <div className="aiterm-consent__warning">⚠️ {t.consent_warning}</div>
          </>
        )}

        {outcome && <div className="aiterm-consent__outcome">{outcome}</div>}

        <div className="aiterm-consent__actions">
          <button
            className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
            onClick={() => {
              void shareDeny(req.requestId);
              setReq(null);
            }}
          >
            {closed ? t.connect_cancel : t.consent_deny}
          </button>
          {!closed && (
            <>
              <button
                className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                disabled={typed.length !== 4}
                onClick={() => void decide("read_only")}
              >
                {t.consent_read_only}
              </button>
              <button
                className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
                disabled={typed.length !== 4}
                onClick={() => void decide("control")}
              >
                {t.consent_control}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: 樣式**

建立 `src/components/ConsentDialog/index.css`：

```css
.aiterm-consent__backdrop {
  position: fixed;
  inset: 0;
  z-index: 200;
  display: flex;
  align-items: center;
  justify-content: center;
  background: #000a;
}

.aiterm-consent {
  width: 380px;
  padding: 20px;
  border-radius: 10px;
  background: var(--aiterm-surface-2, #1e293b);
  border: 1px solid var(--aiterm-border, #334155);
  box-shadow: 0 12px 40px #000c;
}

.aiterm-consent__title {
  font-size: 15px;
  font-weight: 600;
  margin-bottom: 4px;
}

.aiterm-consent__unverified {
  font-size: 11px;
  color: var(--aiterm-text-muted, #94a3b8);
  margin-bottom: 16px;
}

.aiterm-consent__prompt {
  display: block;
  font-size: 13px;
  margin-bottom: 8px;
}

.aiterm-consent__input {
  width: 100%;
  padding: 10px;
  font-family: var(--aiterm-mono, monospace);
  font-size: 26px;
  letter-spacing: 10px;
  text-align: center;
  border-radius: 6px;
  border: 1px solid var(--aiterm-border, #334155);
  background: var(--aiterm-surface-1, #0f172a);
  color: var(--aiterm-text, #e2e8f0);
}

.aiterm-consent__warning {
  margin-top: 10px;
  font-size: 12px;
  color: var(--aiterm-warn-fg, #fbbf24);
}

.aiterm-consent__outcome {
  margin-top: 12px;
  padding: 10px;
  border-radius: 6px;
  background: var(--aiterm-error-bg, #7f1d1d33);
  color: var(--aiterm-error-fg, #fca5a5);
  font-size: 13px;
  line-height: 1.5;
}

.aiterm-consent__actions {
  display: flex;
  gap: 8px;
  justify-content: flex-end;
  margin-top: 18px;
}
```

- [ ] **Step 5: 掛進 `TerminalApp`**

`src/components/TerminalApp.tsx`，在 `NewTabPicker` 那一段附近（約 518 行，搜 `<NewTabPicker`）的**同一層**加入：

```tsx
        <ConsentDialog tabs={tabs.map((x) => ({ id: x.id, title: x.title }))} />
```

**不要**放在分頁的渲染迴圈裡——非作用中的分頁是 `visibility: hidden` + `pointerEvents: none`，放進去的話對方在非作用分頁發起連線時視窗看不見也點不到。`NewTabPicker`/`RouteHint` 就是掛在 app 層，照那個位置放。

並在檔案頂端加 import：

```tsx
import { ConsentDialog } from "./ConsentDialog";
```

- [ ] **Step 6: 跑測試確認轉綠**

Run: `npx vitest run src/components/ConsentDialog`
Expected: PASS，7 個測試全過。

若 `act` 的 import 或用法有問題（React 19 的 `act` 從 `react` 匯出，不是 `react-dom/test-utils`），以實際能跑的寫法為準修正並回報。

- [ ] **Step 7: Commit**

```bash
git add src/components/ConsentDialog/ src/components/TerminalApp.tsx
git commit -m "feat(share): consent dialog

主控端輸入對方唸的 4 位碼，而不是把碼顯示出來讓人按同意——這樣「不核對」
在物理上做不到。後端根本不送那個值過來，所以這是型別擋著，不是 UI 自律。

掛在 TerminalApp 層：非作用分頁是 visibility:hidden + pointerEvents:none，
掛在 TerminalView 裡的話，對方在非作用分頁發起連線時視窗看不見也點不到。"
```

---

## Task 5: `ConnectDialog` 與入口

**Files:**
- Create: `src/components/ConnectDialog/index.tsx`
- Create: `src/components/ConnectDialog/index.css`
- Test: `src/components/ConnectDialog/index.test.tsx`
- Modify: `src/components/TerminalApp.tsx`
- Modify: `src/components/NewTabPicker/tabCatalog.tsx`

- [ ] **Step 1: 寫會紅的測試**

建立 `src/components/ConnectDialog/index.test.tsx`：

```tsx
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const connectMock = vi.fn();
vi.mock("../../ipc/shareViewer", () => ({
  shareViewerConnect: (...a: unknown[]) => connectMock(...a),
}));

vi.mock("../../contexts/LocaleContext", async () => {
  const { translations } = await import("../../lib/i18n");
  return { useLocale: () => ({ t: translations["zh-TW"], locale: "zh-TW", setLocale: () => {} }) };
});

import { ConnectDialog } from "./index";

const onConnected = vi.fn();
const onCancel = vi.fn();

beforeEach(() => {
  connectMock.mockReset().mockResolvedValue("conn-1");
  onConnected.mockReset();
  onCancel.mockReset();
});

describe("ConnectDialog", () => {
  it("keeps the manual address field out of the way at first", () => {
    // 平常乾淨；出事時才把退路攤開（見 spec 的「觀看端」）。
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    expect(screen.queryByPlaceholderText(/192\.168/)).not.toBeInTheDocument();
  });

  it("reveals the manual address field on demand", async () => {
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.click(screen.getByText(/直接輸入位址/));
    expect(screen.getByPlaceholderText(/192\.168/)).toBeInTheDocument();
  });

  it("connects with a manually entered host and port", async () => {
    // 2C 的 mDNS 還沒上線，手動位址是這個階段唯一的路——也是永遠可用的
    // 主路徑（見 spec 的決策紀錄）。
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.click(screen.getByText(/直接輸入位址/));
    await userEvent.type(screen.getByPlaceholderText(/192\.168/), "192.168.1.33:47823");
    await userEvent.type(screen.getByLabelText(/你的名字/), "Bob");
    await userEvent.type(screen.getByLabelText(/6 位數/), "559207");
    await userEvent.click(screen.getByRole("button", { name: /^連線$/ }));

    expect(connectMock).toHaveBeenCalledWith("192.168.1.33", 47823, "559207", "Bob");
    expect(onConnected).toHaveBeenCalledWith("conn-1", "192.168.1.33:47823");
  });

  it("rejects an address that is not host:port", async () => {
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.click(screen.getByText(/直接輸入位址/));
    await userEvent.type(screen.getByPlaceholderText(/192\.168/), "just-a-hostname");
    await userEvent.type(screen.getByLabelText(/6 位數/), "559207");
    await userEvent.click(screen.getByRole("button", { name: /^連線$/ }));

    expect(await screen.findByText(/位址格式不對/)).toBeInTheDocument();
    expect(connectMock).not.toHaveBeenCalled();
  });

  it("shows why connecting failed instead of closing silently", async () => {
    connectMock.mockRejectedValue("連不上 192.168.1.33:47823");
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.click(screen.getByText(/直接輸入位址/));
    await userEvent.type(screen.getByPlaceholderText(/192\.168/), "192.168.1.33:47823");
    await userEvent.type(screen.getByLabelText(/6 位數/), "559207");
    await userEvent.click(screen.getByRole("button", { name: /^連線$/ }));

    expect(await screen.findByText(/連不上/)).toBeInTheDocument();
    expect(onConnected).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: 跑測試確認會紅**

Run: `npx vitest run src/components/ConnectDialog`
Expected: FAIL——`Failed to resolve import "./index"`。

- [ ] **Step 3: 實作**

建立 `src/components/ConnectDialog/index.tsx`：

```tsx
import { useState } from "react";
import { shareViewerConnect } from "../../ipc/shareViewer";
import { useLocale } from "../../contexts/LocaleContext";
import "./index.css";

interface Props {
  /** 連上之後回報連線 id 與對方位址，讓上層開一個 `remote-terminal` 分頁。 */
  onConnected: (connId: string, hostLabel: string) => void;
  onCancel: () => void;
}

/**
 * 觀看端的連線入口。
 *
 * **手動位址永遠是主路徑**（見 spec 的決策紀錄）：2C 會加上 mDNS 自動發現，
 * 但它在公司網路／跨 VLAN／訪客 Wi-Fi 常常失效，所以手動那條路必須一直
 * 走得通。這個階段 mDNS 還沒上線，所以手動是唯一的路。
 *
 * 平常把手動欄位收起來、需要時展開——spec 選的是「找不到時自動展開並說明
 * 原因」，2C 接上 mDNS 後會補上自動展開那段。
 */
export function ConnectDialog({ onConnected, onCancel }: Props) {
  const { t } = useLocale();
  const [code, setCode] = useState("");
  const [name, setName] = useState("");
  const [manualOpen, setManualOpen] = useState(false);
  const [address, setAddress] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit() {
    setError(null);
    const parsed = parseAddress(address);
    if (!parsed) {
      setError(t.connect_bad_address);
      return;
    }
    setBusy(true);
    try {
      const connId = await shareViewerConnect(parsed.host, parsed.port, code, name || "AITerm");
      onConnected(connId, address);
    } catch (e) {
      // 連不上要說原因，不要靜默關閉——使用者才知道下一步該做什麼。
      setError(t.connect_failed.replace("{error}", String(e)));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="aiterm-connect__backdrop">
      <div className="aiterm-connect" role="dialog" aria-modal="true">
        <div className="aiterm-connect__title">{t.connect_title}</div>

        <label className="aiterm-connect__label" htmlFor="aiterm-connect-code">
          {t.connect_code_label}
        </label>
        <input
          id="aiterm-connect-code"
          className="aiterm-connect__code"
          type="text"
          inputMode="numeric"
          maxLength={6}
          autoFocus
          value={code}
          onChange={(e) => setCode(e.target.value.replace(/\D/g, "").slice(0, 6))}
        />

        <label className="aiterm-connect__label" htmlFor="aiterm-connect-name">
          {t.connect_name_label}
        </label>
        <input
          id="aiterm-connect-name"
          className="aiterm-connect__text"
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />

        {!manualOpen && (
          <button className="aiterm-connect__toggle" onClick={() => setManualOpen(true)}>
            ▸ {t.connect_manual_toggle}
          </button>
        )}

        {manualOpen && (
          <>
            <label className="aiterm-connect__label" htmlFor="aiterm-connect-addr">
              {t.connect_manual_label}
            </label>
            <input
              id="aiterm-connect-addr"
              className="aiterm-connect__text"
              type="text"
              placeholder={t.connect_manual_placeholder}
              value={address}
              onChange={(e) => setAddress(e.target.value)}
            />
          </>
        )}

        {error && <div className="aiterm-connect__error">{error}</div>}

        <div className="aiterm-connect__actions">
          <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={onCancel}>
            {t.connect_cancel}
          </button>
          <button
            className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
            disabled={busy || code.length !== 6}
            onClick={() => void submit()}
          >
            {t.connect_submit}
          </button>
        </div>
      </div>
    </div>
  );
}

/** `host:port` → `{ host, port }`。格式不對回 `null`。 */
function parseAddress(raw: string): { host: string; port: number } | null {
  const m = raw.trim().match(/^(.+):(\d{1,5})$/);
  if (!m) return null;
  const port = Number(m[2]);
  if (!Number.isInteger(port) || port < 1 || port > 65535) return null;
  return { host: m[1], port };
}
```

- [ ] **Step 4: 樣式**

建立 `src/components/ConnectDialog/index.css`：

```css
.aiterm-connect__backdrop {
  position: fixed;
  inset: 0;
  z-index: 200;
  display: flex;
  align-items: center;
  justify-content: center;
  background: #000a;
}

.aiterm-connect {
  width: 380px;
  padding: 20px;
  border-radius: 10px;
  background: var(--aiterm-surface-2, #1e293b);
  border: 1px solid var(--aiterm-border, #334155);
  box-shadow: 0 12px 40px #000c;
}

.aiterm-connect__title {
  font-size: 15px;
  font-weight: 600;
  margin-bottom: 16px;
}

.aiterm-connect__label {
  display: block;
  font-size: 12px;
  color: var(--aiterm-text-muted, #94a3b8);
  margin: 12px 0 6px;
}

.aiterm-connect__code {
  width: 100%;
  padding: 10px;
  font-family: var(--aiterm-mono, monospace);
  font-size: 24px;
  letter-spacing: 8px;
  text-align: center;
  border-radius: 6px;
  border: 1px solid var(--aiterm-border, #334155);
  background: var(--aiterm-surface-1, #0f172a);
  color: var(--aiterm-text, #e2e8f0);
}

.aiterm-connect__text {
  width: 100%;
  padding: 8px;
  border-radius: 6px;
  border: 1px solid var(--aiterm-border, #334155);
  background: var(--aiterm-surface-1, #0f172a);
  color: var(--aiterm-text, #e2e8f0);
  font-size: 13px;
}

.aiterm-connect__toggle {
  margin-top: 14px;
  background: none;
  border: none;
  padding: 0;
  color: var(--aiterm-text-muted, #94a3b8);
  font-size: 12px;
  cursor: pointer;
}

.aiterm-connect__error {
  margin-top: 12px;
  padding: 10px;
  border-radius: 6px;
  background: var(--aiterm-error-bg, #7f1d1d33);
  color: var(--aiterm-error-fg, #fca5a5);
  font-size: 12px;
  line-height: 1.5;
}

.aiterm-connect__actions {
  display: flex;
  gap: 8px;
  justify-content: flex-end;
  margin-top: 18px;
}
```

- [ ] **Step 5: 接上入口**

`src/components/NewTabPicker/tabCatalog.tsx`，把 `remote-terminal` 那一筆的 `hidden: true` **拿掉**——現在有入口了。

`src/components/TerminalApp.tsx`：

1. 加狀態與 import：

```tsx
import { ConnectDialog } from "./ConnectDialog";
```

```tsx
  const [connectOpen, setConnectOpen] = useState(false);
```

2. `handlePickerSelect` 裡，在建立分頁**之前**攔截 `remote-terminal`：

```tsx
    if (type === "remote-terminal") {
      // 遠端分頁要先問短碼／位址才知道要連誰。連上之後才建分頁——
      // 那時才有 connId 可以掛事件。
      setConnectOpen(true);
      return "";
    }
```

**注意**：`handlePickerSelect` 目前回傳 `newId`（`handleRouteHintPick` 會用），所以這裡回傳空字串而不是 `undefined`，避免改動它的簽名。實際回傳型別要先用 Read 確認。

3. 在 `NewTabPicker` 那一層加入：

```tsx
        {connectOpen && (
          <ConnectDialog
            onCancel={() => setConnectOpen(false)}
            onConnected={(connId, hostLabel) => {
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
                },
              ]);
              setActiveId(newId);
            }}
          />
        )}
```

**注意**：`setTabs` / `setActiveId` 的實際名稱與 `Tab` 的必填欄位要先用 Read 確認——建分頁的既有寫法在 `handlePickerSelect` 裡，照它的形狀來。

- [ ] **Step 6: 跑測試確認轉綠**

Run: `npx vitest run src/components/ConnectDialog`
Expected: PASS，5 個測試全過。

- [ ] **Step 7: 全套迴歸 ＋ 型別檢查**

Run: `npx vitest run && npx tsc -b`
Expected: 全綠，`tsc -b` 無輸出。

**用 `npx tsc -b`，不要用 `tsc --noEmit`**——根 `tsconfig.json` 是 solution file（`"files": []`），`--noEmit` 什麼都不檢查而且永遠 exit 0。

- [ ] **Step 8: Commit**

```bash
git add src/components/ConnectDialog/ src/components/TerminalApp.tsx src/components/NewTabPicker/tabCatalog.tsx
git commit -m "feat(share): connect dialog and the viewer entry point

手動位址永遠是主路徑——2C 的 mDNS 只是加速器，在公司網路／跨 VLAN／
訪客 Wi-Fi 常常失效。連不上要說原因，不要靜默關閉。"
```

---

## 完成標準

- [ ] `npx vitest run` 全綠
- [ ] `npx tsc -b` 通過
- [ ] `cd src-tauri && cargo test` 仍全綠（這份計畫不動 Rust，這是防迴歸）
- [ ] **同意視窗的測試證明畫面上沒有任何 4 位數字**——那是整個防中間人設計的最後一哩
- [ ] `remote-terminal` 的 catalog 條目不再是 `hidden`

  注意：`npm run lint` 在這個 repo **本來就不綠**（199 個既有問題），**不是這個計畫造成的、不要順手修**。

**做完這份計畫，這個功能就能真的用了**：兩台機器、手動貼位址、口頭核對 4 位碼、看畫面、接手打字。**尚未具備**：mDNS 自動發現（2C）——在那之前短碼欄位仍要搭配手動位址一起用。
