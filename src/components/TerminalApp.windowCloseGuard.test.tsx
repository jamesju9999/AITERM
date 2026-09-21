import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, act, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";

// 掛載骨架同 TerminalApp.routeHintCloseGuard.test.tsx：invoke/listen/homeDir 三個
// Tauri 入口涵蓋所有 ipc/*.ts，另外 window 與 notification。
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(() => new Promise(() => {})) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
vi.mock("@tauri-apps/api/path", () => ({ homeDir: vi.fn(() => Promise.resolve("/home/test")) }));

let closeCb: ((e: { preventDefault: () => void }) => Promise<void> | void) | undefined;
const destroy = vi.fn(() => Promise.resolve());
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
    onCloseRequested: (cb: typeof closeCb) => { closeCb = cb; return Promise.resolve(() => {}); },
    destroy: () => destroy(),
  }),
}));
vi.mock("@tauri-apps/plugin-notification", () => ({
  sendNotification: vi.fn(),
}));
vi.mock("../ipc/quit", () => ({
  QUIT_REQUESTED_EVENT: "app://quit-requested",
  onQuitRequested: () => Promise.resolve(() => {}),
}));

// LoopStudio 是最容易單靠兩個 mock 就掛得起來的「會回報忙碌」的分頁類型。
vi.mock("../ipc/provider", () => ({ listProviders: () => Promise.resolve([]) }));
const fakeLoop = {
  trace: [] as unknown[],
  isRunning: false,
  iteration: 0,
  start: vi.fn(),
  stop: vi.fn(),
  resume: vi.fn(),
  pendingConfirmation: null as unknown,
};
vi.mock("../hooks/useOrchestratorLoop", () => ({
  useOrchestratorLoop: () => fakeLoop,
}));

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

import { TerminalApp } from "./TerminalApp";
import { LocaleProvider } from "../contexts/LocaleContext";
import { SESSION_TABS_KEY } from "../lib/sessionTabs";

beforeEach(() => {
  fakeLoop.isRunning = false;
  closeCb = undefined;
  destroy.mockClear();
  localStorage.clear();
  localStorage.setItem(SESSION_TABS_KEY, JSON.stringify([{ title: "Loop Studio", type: "loop-studio" }]));
});

function renderApp() {
  return render(
    <LocaleProvider>
      <MemoryRouter>
        <TerminalApp />
      </MemoryRouter>
    </LocaleProvider>
  );
}

async function fireWindowClose() {
  await waitFor(() => expect(closeCb).toBeDefined());
  const preventDefault = vi.fn();
  await act(async () => { await closeCb!({ preventDefault }); });
  return preventDefault;
}

describe("TerminalApp: closing the window while work is running", () => {
  it("全部閒置：不出現確認框，直接 destroy", async () => {
    renderApp();
    await fireWindowClose();
    expect(screen.queryByRole("heading", { name: "還有工作正在進行" })).not.toBeInTheDocument();
    expect(destroy).toHaveBeenCalledTimes(1);
  });

  it("Loop 執行中：出現一個確認框，列出分頁名稱與原因，且不 destroy", async () => {
    fakeLoop.isRunning = true;
    renderApp();
    await fireWindowClose();

    const heading = screen.getByRole("heading", { name: "還有工作正在進行" });
    // 分頁列與標題列也會出現「Loop Studio」，斷言限縮在確認框內。
    const dialog = within(heading.closest(".aiterm-close-dialog") as HTMLElement);
    expect(dialog.getByText("Loop Studio — Loop 正在執行")).toBeInTheDocument();
    expect(destroy).not.toHaveBeenCalled();
  });

  it("按「取消（繼續執行）」：確認框消失、不 destroy", async () => {
    fakeLoop.isRunning = true;
    renderApp();
    await fireWindowClose();
    await userEvent.click(screen.getByRole("button", { name: "取消（繼續執行）" }));
    expect(screen.queryByRole("heading", { name: "還有工作正在進行" })).not.toBeInTheDocument();
    expect(destroy).not.toHaveBeenCalled();
  });

  it("按「關閉並中止」：destroy 視窗", async () => {
    fakeLoop.isRunning = true;
    renderApp();
    await fireWindowClose();
    await userEvent.click(screen.getByRole("button", { name: "關閉並中止" }));
    await waitFor(() => expect(destroy).toHaveBeenCalledTimes(1));
  });

  it("確認框掛在最外層容器（覆蓋整個視窗），不是嵌在某個分頁裡", async () => {
    fakeLoop.isRunning = true;
    const { container } = renderApp();
    await fireWindowClose();
    const overlay = document.querySelector(".aiterm-close-overlay");
    expect(overlay).not.toBeNull();
    expect(overlay!.parentElement).toBe(container.firstElementChild);
  });
});
