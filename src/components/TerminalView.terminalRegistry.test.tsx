import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";

// Same setup as TerminalView.idleSignal.test.tsx (verified there to mount
// TerminalView fully in jsdom): mocking these three low-level Tauri entry
// points covers every ipc/*.ts file in this tree, since they all funnel
// through invoke()/listen()/homeDir(). invoke() resolves for "pty_create" so
// sessionId becomes non-null, which is what drives TerminalView's
// registration effect.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((cmd: string) => {
    if (cmd === "pty_create") return Promise.resolve("test-session");
    return new Promise(() => {});
  }),
}));

// Captures every listen() registration so the test can find the handler
// TerminalView registers for this session's pty://data/{id} event, proving
// sessionId has become "test-session".
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

// Same rationale as idleSignal's test: driving useTerminalBlocks/
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

import { TerminalView } from "./TerminalView";
import { LocaleProvider } from "../contexts/LocaleContext";
import { ptyDataEvent } from "../ipc/events";
import { serializeTerminal, unregisterTerminal } from "../lib/terminalInstanceRegistry";

beforeEach(() => {
  listenHandlers.clear();
});

describe("TerminalView registers itself into terminalInstanceRegistry", () => {
  it("becomes serializable once the PTY session exists, and stops being once unmounted", async () => {
    const { unmount } = render(
      <LocaleProvider>
        <MemoryRouter>
          <TerminalView tabId="tab-1" registerCloseGuard={() => {}} unregisterCloseGuard={() => {}} />
        </MemoryRouter>
      </LocaleProvider>,
    );

    // Same wait as idleSignal's test: createPty() resolving and the
    // pty://data/{id} listener being registered both prove sessionId has
    // become "test-session".
    const dataEvent = ptyDataEvent("test-session");
    await waitFor(() => expect(listenHandlers.has(dataEvent)).toBe(true));

    await waitFor(() => expect(serializeTerminal("test-session")).not.toBeNull());
    // xterm's own serialize() on a freshly-opened, empty terminal returns a
    // string (typically just a reset sequence) — we only care that the
    // registry has SOMETHING now, not its exact content.
    expect(typeof serializeTerminal("test-session")).toBe("string");

    unmount();
    expect(serializeTerminal("test-session")).toBeNull();
    // Belt and suspenders: explicitly ensure the registry doesn't leak this
    // id past the test even if the component's own cleanup were to change.
    unregisterTerminal("test-session");
  });
});
