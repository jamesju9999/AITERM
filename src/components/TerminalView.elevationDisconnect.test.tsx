import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, act } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";

// 這個測試檔驗的是「holistic review」第二項發現的修法：提權狀態從 true 變回
// false（`useElevationState` 回傳值的轉換，對應後端 `exit` 或連線意外中斷）
// 時，TerminalView 要短暫顯示一句系統訊息（`t.elevation_disconnected`），
// 而不是像修法之前那樣讓徽章直接無聲消失。
//
// 跟 TerminalView.elevationBannerRace.test.tsx 同一套 baseline mock（三個
// Tauri entry point 讓 TerminalView 能在 jsdom 完整掛載），但額外直接 mock
// `useElevationState`——真正的 hook 靠 `listen()` 訂閱 Tauri 事件，這裡要的
// 是「在同一個已掛載的元件上，讓這個 hook 的回傳值從 true 變成 false」，
// 直接控制 mock 回傳值比在 jsdom 裡偽造 Tauri 事件簡單也更貼近要測的邏輯
// 本身（TerminalView 怎麼反應這個布林值的變化），不是 hook 內部怎麼訂閱。
const invokeMock = vi.fn((cmd: string) => {
  if (cmd === "pty_create") return Promise.resolve("test-session");
  if (cmd === "pty_check_permission_denied") return Promise.resolve(false);
  return new Promise(() => {});
});
vi.mock("@tauri-apps/api/core", () => ({ invoke: (cmd: string) => invokeMock(cmd) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
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

vi.mock("../hooks/useTerminalBlocks", () => ({
  useTerminalBlocks: () => ({
    blocks: [],
    isAlternateBuffer: false,
    isRawKeyboardModeActive: false,
    submitCommand: vi.fn(),
    beginTrackedBlock: vi.fn(),
    appendOutput: vi.fn(),
    setBlockGitInfo: vi.fn(),
    finalizeBlock: vi.fn(),
    clearAllBlocks: vi.fn(),
    termInstance: null,
  }),
}));

// 唯一直接控制的 mock：`elevatedMock.value` 決定每次 render 讀到的
// `useElevationState(sessionId)` 回傳值。
const elevatedMock = { value: false };
vi.mock("../hooks/useElevationState", () => ({
  useElevationState: () => elevatedMock.value,
}));

import { TerminalView } from "./TerminalView";
import { LocaleProvider } from "../contexts/LocaleContext";
import { translations } from "../lib/i18n";

const t = translations["zh-TW"];

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  elevatedMock.value = false;
  invokeMock.mockClear();
});

afterEach(() => {
  vi.useRealTimers();
});

function renderTerminalView() {
  return render(
    <LocaleProvider>
      <MemoryRouter>
        <TerminalView tabId="tab-1" />
      </MemoryRouter>
    </LocaleProvider>,
  );
}

describe("elevation banner：提權連線從已連線變回未連線時顯示系統訊息", () => {
  it("elevated 從 true 轉為 false 時短暫顯示 elevation_disconnected，並在 3 秒後自動收起", async () => {
    const { rerender } = renderTerminalView();

    // 初次掛載：elevated 恆為 false（見 useElevationState 文件註解），不應
    // 顯示任何 disconnect 訊息。
    expect(screen.queryByText(t.elevation_disconnected)).not.toBeInTheDocument();

    // 模擬後端回報已提權。
    elevatedMock.value = true;
    await act(async () => {
      rerender(
        <LocaleProvider>
          <MemoryRouter>
            <TerminalView tabId="tab-1" />
          </MemoryRouter>
        </LocaleProvider>,
      );
    });
    expect(screen.queryByText(t.elevation_disconnected)).not.toBeInTheDocument();

    // 模擬提權連線結束（exit 或斷線，前端無法區分）。
    elevatedMock.value = false;
    await act(async () => {
      rerender(
        <LocaleProvider>
          <MemoryRouter>
            <TerminalView tabId="tab-1" />
          </MemoryRouter>
        </LocaleProvider>,
      );
    });

    expect(await screen.findByText(t.elevation_disconnected)).toBeInTheDocument();

    // 3 秒後自動收起，不需要使用者手動關閉。
    await act(async () => {
      vi.advanceTimersByTime(3000);
    });
    expect(screen.queryByText(t.elevation_disconnected)).not.toBeInTheDocument();
  });

  it("elevated 從未曾為 true 就一直是 false 時，不顯示 disconnect 訊息", async () => {
    renderTerminalView();
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.queryByText(t.elevation_disconnected)).not.toBeInTheDocument();
  });
});
