import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, act } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";

// Same mocking setup as TerminalView.remoteBurstOutput.test.tsx — real
// useTerminalBlocks, real @xterm/xterm. This file additionally mocks
// "../lib/ansiBlockParser" so the test can precisely control WHEN
// finalizeBlock's async parse (and therefore its `term.clear()` call, which
// only fires after that parse resolves) actually completes, to deterministically
// construct the exact race window under investigation instead of relying on
// non-deterministic real timing.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((cmd: string) => {
    if (cmd === "pty_create") return Promise.resolve("test-session");
    return new Promise(() => {});
  }),
}));

const listenHandlers = new Map<string, ((event: { payload: unknown }) => void)[]>();
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((event: string, handler: (event: { payload: unknown }) => void) => {
    const arr = listenHandlers.get(event) ?? [];
    arr.push(handler);
    listenHandlers.set(event, arr);
    return Promise.resolve(() => {
      const idx = arr.indexOf(handler);
      if (idx >= 0) arr.splice(idx, 1);
    });
  }),
}));

vi.mock("@tauri-apps/api/path", () => ({ homeDir: vi.fn(() => Promise.resolve("/home/test")) }));

Element.prototype.scrollTo = Element.prototype.scrollTo || (() => {});
if (!window.matchMedia) {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
}
class FakeResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
(globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = FakeResizeObserver;

vi.mock("../hooks/useAgentMission", () => ({
  useAgentMission: () => ({
    agentMission: null,
    startMission: vi.fn(),
    stopMission: vi.fn(),
    addTokens: vi.fn(),
  }),
}));

// Deferred, manually-resolved parse: lets the test control precisely when
// finalizeBlock's async parseAnsiToRenderedLines(...).then(() => term.clear())
// actually fires, instead of racing against real (non-deterministic) timing.
let resolveParse: (() => void) | null = null;
vi.mock("../lib/ansiBlockParser", () => ({
  parseAnsiToRenderedLines: vi.fn(
    () =>
      new Promise((resolve) => {
        resolveParse = () => resolve([{ spans: [{ text: "stubbed" }] }]);
      }),
  ),
}));

import { TerminalView } from "./TerminalView";
import { LocaleProvider } from "../contexts/LocaleContext";
import { ptyDataEvent } from "../ipc/events";

beforeEach(() => {
  listenHandlers.clear();
  resolveParse = null;
});

describe("finalizeBlock 的延遲 term.clear() 與下一輪 B 標記之間的 race condition", () => {
  it("上一個指令的 term.clear() 延遲到下一個指令的 B 標記記錄之後才觸發，下一個指令的還原不該失敗", async () => {
    const { container } = render(
      <LocaleProvider>
        <MemoryRouter>
          <TerminalView tabId="tab-1" registerCloseGuard={() => {}} unregisterCloseGuard={() => {}} />
        </MemoryRouter>
      </LocaleProvider>,
    );

    const dataEvent = ptyDataEvent("test-session");
    await waitFor(() => expect(listenHandlers.has(dataEvent)).toBe(true));
    const handlers = listenHandlers.get(dataEvent)!;

    const send = (chunk: string) => {
      const bytes = new TextEncoder().encode(chunk);
      const base64 = btoa(String.fromCharCode(...bytes));
      act(() => {
        handlers.forEach((h) => h({ payload: { base64 } }));
      });
    };

    // 第一個遠端指令："ls"。B → 指令回顯 → C（還原建立區塊）→ 小輸出 → D
    // （觸發 finalizeBlock，啟動非同步 parseAnsiToRenderedLines——這裡用
    // 上面的 mock 卡住，不會馬上 resolve，模擬 term.clear() 被延遲）。
    send("user@host:~$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07");
    send("file1\r\n");
    send("\x1b]133;D;0\x07");

    // 讓 xterm 對前面這幾個 chunk 的非同步解析真正跑完，D 才會被處理、
    // finalizeBlock 才會被呼叫、parseAnsiToRenderedLines（也就是上面的
    // mock）才會真的被叫到、resolveParse 才會被設定。
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 30));
    });
    expect(resolveParse).not.toBeNull();

    // 這裡刻意「還沒」讓 parseAnsiToRenderedLines resolve（term.clear() 還
    // 沒被呼叫），就先讓 shell 的下一輪 prompt 週期送達：A（新提示字元）
    // 接著 B（記錄「ifconfig 從這裡開始輸入」的絕對座標）——此時的座標是
    // 相對於「ls 的輸出還留在畫面上、尚未被清除」的緩衝區狀態算出來的。
    send("\x1b]133;A\x07user@host:~$ \x1b]133;B\x07");

    // 讓 xterm 先把這個新的 B 處理完、記進 promptEndRef——要先確定新座標
    // 已經被記錄下來，才能讓接下來延遲觸發的 term.clear() 把它弄髒，
    // 而不是弄髒一個根本還沒被寫進去的舊值。
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 30));
    });

    // 現在才讓第一個指令的非同步解析完成，觸發它延遲已久的 term.clear()
    // ——這一步會把緩衝區重置，讓剛剛記錄的 B 座標變成對不上的舊資料。
    expect(resolveParse).not.toBeNull();
    await act(async () => {
      resolveParse!();
      await Promise.resolve();
      await Promise.resolve();
    });

    // 使用者接著打"ifconfig"、送出 Enter、觸發 C——這應該要能正確還原出
    // "ifconfig" 這個指令文字、變成一張完整的卡片，而不是掉進保底機制。
    send("ifconfig\r\n\x1b]133;C\x07");
    send("en0: flags=8863<UP,BROADCAST> mtu 1500\r\n");
    send("\x1b]133;D;0\x07");

    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 50));
    });

    // 讓第二個指令自己的（也被同一個 mock 卡住的）parseAnsiToRenderedLines
    // 也 resolve，讓它有機會被畫進卡片列表。
    if (resolveParse) {
      await act(async () => {
        resolveParse!();
        await Promise.resolve();
        await Promise.resolve();
      });
    }

    await waitFor(() => {
      const cards = container.querySelectorAll(".aiterm-block-card");
      expect(cards.length).toBeGreaterThanOrEqual(2);
    });

    const cardsText = Array.from(container.querySelectorAll(".aiterm-block-card")).map(
      (el) => el.textContent ?? "",
    );
    const ifconfigCard = cardsText.find((t) => t.includes("ifconfig"));
    expect(ifconfigCard).toBeDefined();
  });

  // 「clear() 觸發當下游標已經不在 B 記錄的那一行」這個防呆條件的複合
  // 情境測試（欄寬換行造成的接續行），需要精確控制欄寬與行號才能可靠
  // 重現——見 src/hooks/useTerminalBlocks.staleClearWrap.test.ts，那裡
  // 用直接 renderHook（而不是掛載整個 TerminalView）取得可控制的
  // `cols: 80`，已經證明過先前的無條件搬遷版本會產生內容錯誤的假區塊、
  // 修正後正確退回保底機制。
});
