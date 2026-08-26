import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, act } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";

// Same setup as TerminalView.closeGuard.test.tsx (verified there to mount
// TerminalView fully in jsdom): mocking these three low-level Tauri entry
// points covers every ipc/*.ts file in this tree, since they all funnel
// through invoke()/listen()/homeDir(). Unlike closeGuard's test, invoke()
// here DOES resolve for "pty_create" — this test needs sessionId to become
// non-null so <AiPanel> actually renders, and needs the real onPtyData
// listener registered so we can drive a real PTY chunk through it.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((cmd: string) => {
    if (cmd === "pty_create") return Promise.resolve("test-session");
    // Everything else (config, shell-type detection, recent-output
    // backfill, …) never resolves in a test — same as closeGuard's test —
    // which is fine since nothing here awaits them.
    return new Promise(() => {});
  }),
}));

// Captures every listen() registration so the test can find the handler
// TerminalView registers for this session's pty://data/{id} event and
// invoke it directly, the same way the real Tauri event bridge would.
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

// jsdom doesn't implement these; xterm.js and TerminalView's own resize
// handling call them unconditionally on mount.
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

// Same rationale as closeGuard's test: driving useTerminalBlocks/
// useAgentMission for real would mean choreographing a full OSC-133 byte
// stream through xterm's parser, which is unrelated to what this test
// checks. Inert mocks keep the onPtyData callback's other side effects
// (appendOutput, blocksRef reads, agentMissionRef reads) no-ops.
vi.mock("../hooks/useTerminalBlocks", () => ({
  useTerminalBlocks: () => ({
    blocks: [],
    isAlternateBuffer: false,
    submitCommand: vi.fn(),
    beginTrackedBlock: vi.fn(),
    appendOutput: vi.fn(),
    setBlockGitInfo: vi.fn(),
    termInstance: null,
  }),
}));

vi.mock("../hooks/useAgentMission", () => ({
  useAgentMission: () => ({
    agentMission: null,
    startMission: vi.fn(),
    stopMission: vi.fn(),
    addTokens: vi.fn(),
  }),
}));

// Records every props object AiPanel is rendered with, so the test can
// inspect the getIdleMs function TerminalView passes it.
const aiPanelProps: Record<string, unknown>[] = [];
vi.mock("./AiPanel", () => ({
  AiPanel: (props: Record<string, unknown>) => {
    aiPanelProps.push(props);
    return null;
  },
}));

import { TerminalView } from "./TerminalView";
import { LocaleProvider } from "../contexts/LocaleContext";
import { ptyDataEvent } from "../ipc/events";

beforeEach(() => {
  listenHandlers.clear();
  aiPanelProps.length = 0;
});

describe("TerminalView idle signal", () => {
  it("傳給 AiPanel 的 getIdleMs 在有 PTY 輸出後回傳值變小", async () => {
    const nowRef = { value: 1_000_000 };
    const nowSpy = vi.spyOn(Date, "now").mockImplementation(() => nowRef.value);
    try {
      render(
        <LocaleProvider>
          <MemoryRouter>
            <TerminalView tabId="tab-1" registerCloseGuard={() => {}} unregisterCloseGuard={() => {}} />
          </MemoryRouter>
        </LocaleProvider>,
      );

      // Wait for createPty() to resolve and the pty://data/{id} listener to
      // actually be registered — both are async chains kicked off from an
      // effect.
      const dataEvent = ptyDataEvent("test-session");
      await waitFor(() => expect(listenHandlers.has(dataEvent)).toBe(true));
      await waitFor(() => expect(aiPanelProps.length).toBeGreaterThan(0));

      const getIdleMs = aiPanelProps[aiPanelProps.length - 1].getIdleMs as (() => number) | undefined;
      expect(typeof getIdleMs).toBe("function");

      // Advance the clock with no PTY output in between: idle time should
      // reflect that elapsed time.
      nowRef.value += 4_000;
      expect(getIdleMs!()).toBeGreaterThanOrEqual(4_000);

      // Now drive a real PTY chunk through the same event the backend would
      // emit, exactly like onPtyData's real subscriber does.
      const handlers = listenHandlers.get(dataEvent)!;
      expect(handlers.length).toBeGreaterThan(0);
      act(() => {
        handlers.forEach((h) => h({ payload: { base64: btoa("hello\r\n") } }));
      });

      // A few ms later, idle time should be back down near zero — proof
      // that the PTY chunk actually updated the last-output timestamp
      // getIdleMs reads.
      nowRef.value += 5;
      const idleAfter = getIdleMs!();
      expect(idleAfter).toBeLessThan(100);
      expect(idleAfter).toBeLessThan(4_000);
    } finally {
      nowSpy.mockRestore();
    }
  });
});
