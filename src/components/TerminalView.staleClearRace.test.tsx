import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, act } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";

// Same mocking setup as TerminalView.remoteBurstOutput.test.tsx — real
// useTerminalBlocks, real @xterm/xterm. This file mocks "../lib/ansiBlockParser"
// so the test can assert the async parseAnsiToRenderedLines fallback is never
// invoked when a marker is available (OSC 133 C fired) — finalizeBlock takes
// the synchronous readRenderedLines path instead, which closes the
// term.clear()-vs-next-prompt-B race this file used to have to construct by
// hand with a deferred promise.
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
vi.mock("../lib/ansiBlockParser", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/ansiBlockParser")>();
  return {
    ...actual,
    parseAnsiToRenderedLines: vi.fn(
      () =>
        new Promise((resolve) => {
          resolveParse = () => resolve([{ spans: [{ text: "stubbed" }] }]);
        }),
    ),
  };
});

import { TerminalView } from "./TerminalView";
import { LocaleProvider } from "../contexts/LocaleContext";
import { ptyDataEvent } from "../ipc/events";

beforeEach(() => {
  listenHandlers.clear();
  resolveParse = null;
});

describe("finalizeBlock 的 term.clear() 與下一輪 B 標記之間不再有 race condition", () => {
  it("OSC 133 C 有回報時，區塊改用 marker 同步算 renderedLines 並同步 clear/rebase——下一個指令的 B 座標不會被延遲的 clear() 弄髒", async () => {
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

    // 第一個遠端指令："ls"。B → 指令回顯 → C（記錄 marker）→ 小輸出 → D。
    // 因為 C 有觸發，outputStartRef 會拿到一個 marker，finalizeBlock 就能
    // 用 readRenderedLines 同步算完 renderedLines，不必等非同步的
    // parseAnsiToRenderedLines——連帶 term.clear()/rebase 也是同步發生，
    // 中間不會有任何非同步空窗讓下一輪的 B 記錄到「尚未清除」的座標。
    send("user@host:~$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07");
    send("file1\r\n");
    send("\x1b]133;D;0\x07");

    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 30));
    });

    // 因為走的是同步 marker 路徑，模擬非同步解析的 mock 根本不會被呼叫。
    expect(resolveParse).toBeNull();

    // 緊接著送下一輪的 A/B/指令/C/D——沒有任何刻意延遲，因為已經不需要
    // 用延遲製造 race 視窗來驗證了：同步路徑本身就保證了正確性。
    send("\x1b]133;A\x07user@host:~$ \x1b]133;B\x07ifconfig\r\n\x1b]133;C\x07");
    send("en0: flags=8863<UP,BROADCAST> mtu 1500\r\n");
    send("\x1b]133;D;0\x07");

    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 50));
    });

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

  // 「clear() 觸發當下游標已經不在 B 記錄的那一行」這個防呆條件，現在只
  // 有「完全沒有 OSC 133 C 回報」的還原路徑（沒有 marker，走非同步
  // fallback）才可能發生——見 src/hooks/useTerminalBlocks.staleClearWrap.test.ts，
  // 那裡用直接 renderHook 取得可控制的 `cols: 80` 精確重現欄寬換行的複合
  // 情境，驗證保底機制仍然正確攔截。
});
