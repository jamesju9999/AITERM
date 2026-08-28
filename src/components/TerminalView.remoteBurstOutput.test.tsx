import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, act } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";

// Same mocking setup as TerminalView.idleSignal.test.tsx (verified there to
// mount TerminalView fully in jsdom) — but this file deliberately does NOT
// mock "../hooks/useTerminalBlocks": this bug is specifically about the real
// interaction between onPtyData's term.write()/appendOutput() sequencing and
// @xterm/xterm's own internal async write scheduling, which a mocked hook
// (or a mocked xterm) can't exercise at all.
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

import { TerminalView } from "./TerminalView";
import { LocaleProvider } from "../contexts/LocaleContext";
import { ptyDataEvent } from "../ipc/events";

beforeEach(() => {
  listenHandlers.clear();
});

describe("TerminalView 遠端指令大量輸出的 race condition", () => {
  it("一次湧入多個 PTY chunk（模擬 ifconfig 這類大輸出）時，遠端還原出的區塊要正確收到全部輸出，不能被吃掉", async () => {
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
    expect(handlers.length).toBeGreaterThan(0);

    // 模擬遠端觀看者送出 "ifconfig" 後，shell 短時間內連續送出好幾個
    // PTY chunk（提示字元+B+指令回顯+換行+C 在第一個 chunk，接著是大量
    // 網卡資訊分成好幾個 chunk，最後一個 chunk 帶 D）。刻意在同一個
    // act() 裡連續呼叫，中間完全不 await、不讓事件迴圈喘息，重現「大量
    // 輸出一次湧入」的真實時序——這正是 bug 現場的樣子。
    const chunks = [
      "user@host:~$ \x1b]133;B\x07ifconfig\r\n\x1b]133;C\x07",
      "en0: flags=8863<UP,BROADCAST> mtu 1500\r\n",
      "en1: flags=8863<UP,BROADCAST> mtu 1500\r\n",
      "lo0: flags=8049<UP,LOOPBACK> mtu 16384\r\n",
      "\x1b]133;D;0\x07",
    ];

    act(() => {
      chunks.forEach((chunk) => {
        const bytes = new TextEncoder().encode(chunk);
        handlers.forEach((h) => h({ payload: { base64: btoa(String.fromCharCode(...bytes)) } }));
      });
    });

    // 讓 xterm.js 內部排定的非同步解析（WriteBuffer 對非使用者輸入觸發的
    // 寫入一律用 setTimeout 排到下一輪事件迴圈才真正解析）真正跑完。
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 100));
    });

    await waitFor(() => {
      const card = container.querySelector(".aiterm-block-card");
      expect(card).not.toBeNull();
    });

    const cardText = container.querySelector(".aiterm-block-card")?.textContent ?? "";
    expect(cardText).toContain("ifconfig");
    // 這是抓到的 bug：舊寫法會讓 en0/en1/lo0 這些真正的指令輸出被吃掉，
    // 卡片只剩下指令文字本身，執行時間也會異常趨近於 0ms。
    expect(cardText).toContain("en0");
    expect(cardText).toContain("en1");
    expect(cardText).toContain("lo0");
  });
});
