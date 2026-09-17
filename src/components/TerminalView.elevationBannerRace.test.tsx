import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";

// 跟 TerminalView.closeGuard.test.tsx 同一個 baseline：mock 這三個 Tauri
// entry point 就能讓 TerminalView 在 jsdom 完整掛載，不必個別 mock
// src/ipc/*.ts。invoke() 依指令名分派——大部分維持「永不 resolve」（真的
// backend 在測試裡本來就不會回應），只有這個測試真正需要控制時機的兩個
// 提權相關指令另外處理。
let resolveElevate: ((started: boolean) => void) | null = null;
const invokeMock = vi.fn((cmd: string) => {
  if (cmd === "pty_create") return Promise.resolve("test-session");
  // 兩個指令各自失敗都被判定為權限不足——這個測試要的是「兩個獨立的
  // question banner 各自被觸發」，不是權限判定本身的邏輯。
  if (cmd === "pty_check_permission_denied") return Promise.resolve(true);
  if (cmd === "pty_elevate") {
    return new Promise<boolean>((resolve) => {
      resolveElevate = resolve;
    });
  }
  return new Promise(() => {});
});
// 用箭頭函式包一層，不要直接把 invokeMock 傳給 vi.mock 的 factory——factory
// 在模組解析階段就會同步執行（早於這個檔案自己的 `const invokeMock = ...`
// 完成初始化），直接參照會踩到 TDZ。箭頭函式本身可以在那個時間點被建立，
// 只要它「讀」invokeMock 這個閉包變數的動作延後到真正被呼叫（也就是
// TerminalView 掛載、實際發出 IPC 呼叫）的那一刻，那時 invokeMock 早已
// 初始化完畢——跟下面 ansiBlockParser mock 用 resolveParse 閉包變數是同一個
// 理由（見 TerminalView.staleClearRace.test.tsx）。
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

// 直接 mock useTerminalBlocks 並攔截 TerminalView 傳進去的
// onCommandSettled/onCommandStarted——這兩個 callback 就是
// handleCommandSettled/handleCommandStarted 本人（見 TerminalView.tsx 呼叫
// useTerminalBlocks 那行），拿到它們就能直接、確定性地觸發「指令結束」
// 「新指令開始」，不必透過 xterm 解析一長串 OSC 133 位元組序列去猜時機。
// 跟 TerminalView.closeGuard.test.tsx 同一個理由/同一個 mock 對象。
const capturedCallbacks: {
  onCommandSettled?: (exitCode: number) => void;
  onCommandStarted?: (cmd: string) => void;
} = {};
vi.mock("../hooks/useTerminalBlocks", () => ({
  useTerminalBlocks: (
    _sessionId: string,
    _termState: unknown,
    _lastCwdRef: unknown,
    _forceLiveRepaint: unknown,
    onCommandSettled: (exitCode: number) => void,
    onCommandStarted: (cmd: string) => void,
  ) => {
    capturedCallbacks.onCommandSettled = onCommandSettled;
    capturedCallbacks.onCommandStarted = onCommandStarted;
    return {
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
    };
  },
}));

import { TerminalView } from "./TerminalView";
import { LocaleProvider } from "../contexts/LocaleContext";
import { translations } from "../lib/i18n";

const t = translations["zh-TW"];

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  resolveElevate = null;
  invokeMock.mockClear();
  capturedCallbacks.onCommandSettled = undefined;
  capturedCallbacks.onCommandStarted = undefined;
});

afterEach(() => {
  vi.useRealTimers();
});

describe("elevation banner：舊指令的非同步回覆不能蓋掉新指令的 banner", () => {
  it("A 指令的 pty_elevate 晚到的取消回覆，不會蓋掉已經在顯示的 B 指令 question banner", async () => {
    render(
      <LocaleProvider>
        <MemoryRouter>
          <TerminalView tabId="tab-1" />
        </MemoryRouter>
      </LocaleProvider>,
    );

    // A 指令開始、失敗結束——checkPermissionDenied 判定為權限不足，跳出
    // question banner。
    await act(async () => {
      capturedCallbacks.onCommandStarted?.("takeown /f C:\\Windows\\System32");
      capturedCallbacks.onCommandSettled?.(1);
      await vi.runOnlyPendingTimersAsync();
    });

    expect(await screen.findByText(t.elevation_banner_question)).toBeInTheDocument();

    // 使用者按「是」：這裡送出 pty_elevate，但先不 resolve 它——模擬 UAC
    // 對話框還開著，使用者可能好幾分鐘都不會回應（見 pty_elevate 的文件
    // 註解）。
    await userEvent.click(screen.getByText(t.elevation_banner_confirm));
    expect(resolveElevate).not.toBeNull();
    const resolveA = resolveElevate!;

    // A 的 pty_elevate 還吊著的時候，使用者已經接著跑了 B 指令，一樣失敗，
    // 一樣被判定為權限不足——這會 bump commandEpochRef 並開出屬於 B 的
    // 全新 question banner。
    await act(async () => {
      capturedCallbacks.onCommandStarted?.("reg add HKLM\\...");
      capturedCallbacks.onCommandSettled?.(1);
      await vi.runOnlyPendingTimersAsync();
    });

    expect(await screen.findByText(t.elevation_banner_question)).toBeInTheDocument();
    // 確認按鈕還在，代表現在顯示的是 B 的「question」狀態，不是別的。
    expect(screen.getByText(t.elevation_banner_confirm)).toBeInTheDocument();

    // 現在才讓 A 那個吊著的 pty_elevate 回覆「使用者取消了 UAC」。沒有
    // epoch guard 的話，這裡會把畫面上 B 的 question banner 蓋成 A 的
    // cancelled 訊息——即使 B 的 banner 才是使用者當下真正在看、還沒做
    // 決定的那個。
    await act(async () => {
      resolveA(false);
      await vi.runOnlyPendingTimersAsync();
    });

    // 斷言：畫面仍然是 B 的 question banner（有可操作的是／否按鈕），
    // 完全沒有出現 A 的 cancelled 訊息。
    expect(screen.getByText(t.elevation_banner_question)).toBeInTheDocument();
    expect(screen.getByText(t.elevation_banner_confirm)).toBeInTheDocument();
    expect(screen.getByText(t.elevation_banner_cancel)).toBeInTheDocument();
    expect(screen.queryByText(t.elevation_cancelled)).not.toBeInTheDocument();
  });
});
