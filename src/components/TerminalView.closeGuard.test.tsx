import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";

// Mount probe (done ad-hoc before writing this file, then discarded) found
// TerminalView renders fully in jsdom with only these three low-level Tauri
// modules mocked — every ipc/*.ts file in this tree goes through invoke()/
// listen()/homeDir() from these three entry points, so mocking them here
// covers pty/ai/config/fs/enterprise/provider/web/vcs/telegram/usage in one
// shot instead of mocking each ipc/*.ts individually. invoke() never resolves
// (a real backend never answers in a test), which is fine: nothing in these
// tests waits on it.
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(() => new Promise(() => {})) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
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

// The two signals the close guard reads — "is a command running" and "is the
// agent mission active" — come from these two hooks. Driving them for real
// would mean choreographing PTY byte streams through xterm's parser (the
// running-block state machine lives inside useTerminalBlocks itself). Mocking
// the hooks directly lets tests set exactly the state each scenario needs,
// same idea as CodeAssistantView/closeGuard.test.tsx mocking useCodeAssistant.
type FakeBlock = { id: string; command: string; status: "running" | "completed" | "failed"; startTime: number; rawOutput: string };
const fakeBlocksState = { value: [] as FakeBlock[] };
vi.mock("../hooks/useTerminalBlocks", () => ({
  useTerminalBlocks: () => ({
    blocks: fakeBlocksState.value,
    isAlternateBuffer: false,
    submitCommand: vi.fn(),
    beginTrackedBlock: vi.fn(),
    appendOutput: vi.fn(),
    setBlockGitInfo: vi.fn(),
    termInstance: null,
  }),
}));

type FakeMission = { active: boolean; goal: string; stepCount: number; maxSteps: number; tokensUsed: number; history: unknown[] } | null;
const fakeMissionState = { value: null as FakeMission };
vi.mock("../hooks/useAgentMission", () => ({
  useAgentMission: () => ({
    agentMission: fakeMissionState.value,
    startMission: vi.fn(),
    stopMission: vi.fn(),
    addTokens: vi.fn(),
  }),
}));

import { TerminalView } from "./TerminalView";
import { LocaleProvider } from "../contexts/LocaleContext";

function mountAndCaptureGuard() {
  let guard: (() => Promise<boolean>) | undefined;
  const register = (_id: string, g: () => Promise<boolean>) => { guard = g; };
  const unregister = vi.fn();
  // register/unregister must be stable references across a rerender — in the
  // real app (TerminalApp) they are useCallback([]) refs — otherwise every
  // rerender re-runs the registration effect and the test wouldn't be
  // exercising the "registered once, state changes later" scenario for real.
  const renderUi = () => (
    <LocaleProvider>
      <MemoryRouter>
        <TerminalView tabId="tab-1" registerCloseGuard={register} unregisterCloseGuard={unregister} />
      </MemoryRouter>
    </LocaleProvider>
  );
  const view = render(renderUi());
  if (!guard) throw new Error("TerminalView 沒有註冊 close guard");
  return { guard, view, unregister, renderUi };
}

beforeEach(() => {
  fakeBlocksState.value = [];
  fakeMissionState.value = null;
});

describe("TerminalView close guard", () => {
  it("閒置（無 running block、無 active mission）：guard 直接 resolve true，不顯示對話框", async () => {
    const { guard } = mountAndCaptureGuard();
    await expect(guard()).resolves.toBe(true);
    expect(screen.queryByRole("heading", { level: 3 })).not.toBeInTheDocument();
  });

  it("有指令執行中：顯示對話框，標題為「指令執行中」，Promise 保持未定", async () => {
    fakeBlocksState.value = [{ id: "b1", command: "npm test", status: "running", startTime: 0, rawOutput: "" }];
    const { guard } = mountAndCaptureGuard();

    let settled: unknown = "pending";
    await act(async () => { void guard().then((v) => { settled = v; }); });

    expect(screen.getByRole("heading", { level: 3 })).toHaveTextContent("指令執行中");
    expect(settled).toBe("pending");
  });

  it("Agent 任務進行中：顯示對話框，標題為「Agent 任務進行中」", async () => {
    fakeMissionState.value = { active: true, goal: "do the thing", stepCount: 1, maxSteps: 5, tokensUsed: 10, history: [] };
    const { guard } = mountAndCaptureGuard();

    await act(async () => { void guard(); });

    expect(screen.getByRole("heading", { level: 3 })).toHaveTextContent("Agent 任務進行中");
  });

  it("按「關閉並中止」：resolve true", async () => {
    fakeBlocksState.value = [{ id: "b1", command: "npm test", status: "running", startTime: 0, rawOutput: "" }];
    const { guard } = mountAndCaptureGuard();

    let settled: unknown = "pending";
    await act(async () => { void guard().then((v) => { settled = v; }); });
    await userEvent.click(screen.getByRole("button", { name: "關閉並中止" }));

    expect(settled).toBe(true);
  });

  it("按「取消（繼續執行）」：resolve false，對話框消失", async () => {
    fakeBlocksState.value = [{ id: "b1", command: "npm test", status: "running", startTime: 0, rawOutput: "" }];
    const { guard } = mountAndCaptureGuard();

    let settled: unknown = "pending";
    await act(async () => { void guard().then((v) => { settled = v; }); });
    await userEvent.click(screen.getByRole("button", { name: "取消（繼續執行）" }));

    expect(settled).toBe(false);
    expect(screen.queryByRole("heading", { level: 3 })).not.toBeInTheDocument();
  });

  // 釘住 stale closure：guard 只註冊一次，若它閉包捕捉了註冊當下的 blocks/
  // agentMission，之後才開始跑的指令它就看不到，會在沒有任何錯誤訊號的情況下
  // 直接放行。用同一組穩定 register/unregister 引用重繪，模擬「註冊完 guard
  // 之後，指令才開始執行」。
  it("註冊之後才開始執行的指令，guard 仍看得到", async () => {
    const { guard, view, renderUi } = mountAndCaptureGuard();

    fakeBlocksState.value = [{ id: "b1", command: "npm test", status: "running", startTime: 0, rawOutput: "" }];
    view.rerender(renderUi());

    let settled: unknown = "pending";
    await act(async () => { void guard().then((v) => { settled = v; }); });

    expect(screen.getByRole("heading", { level: 3 })).toHaveTextContent("指令執行中");
    expect(settled).toBe("pending");
  });

  it("unmount 時呼叫 unregisterCloseGuard", () => {
    const { view, unregister } = mountAndCaptureGuard();
    view.unmount();
    expect(unregister).toHaveBeenCalledWith("tab-1");
  });
});
