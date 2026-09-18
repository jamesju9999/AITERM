import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";

// 同 TerminalView.elevationBannerRace.test.tsx 的 baseline：mock 三個 Tauri
// entry point 就能讓 TerminalView 在 jsdom 完整掛載。`pty_elevate` 的回覆
// 由測試自己控制時機，因為這個檔案要驗的正是「提權成功之前／之後」的差別。
let resolveElevate: ((started: boolean) => void) | null = null;
const invokeMock = vi.fn((cmd: string) => {
  if (cmd === "pty_create") return Promise.resolve("test-session");
  if (cmd === "pty_check_permission_denied") return Promise.resolve(true);
  if (cmd === "pty_elevate") {
    return new Promise<boolean>((resolve) => {
      resolveElevate = resolve;
    });
  }
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

// 攔截 TerminalView 傳進 useTerminalBlocks 的 callback。這個檔案多攔一個
// `onPromptReady`（OSC 133 B），因為自動重跑就是掛在它上面——直接呼叫它比
// 餵一串 OSC 位元組進 xterm 再猜時機來得確定。
const submitCommandMock = vi.fn();
const capturedCallbacks: {
  onCommandSettled?: (exitCode: number) => void;
  onCommandStarted?: (cmd: string) => void;
  onPromptReady?: () => void;
} = {};
vi.mock("../hooks/useTerminalBlocks", () => ({
  useTerminalBlocks: (
    _sessionId: string,
    _termState: unknown,
    _lastCwdRef: unknown,
    _forceLiveRepaint: unknown,
    onCommandSettled: (exitCode: number) => void,
    onCommandStarted: (cmd: string) => void,
    _write: unknown,
    _hostPlatform: unknown,
    onPromptReady: () => void,
  ) => {
    capturedCallbacks.onCommandSettled = onCommandSettled;
    capturedCallbacks.onCommandStarted = onCommandStarted;
    capturedCallbacks.onPromptReady = onPromptReady;
    return {
      blocks: [],
      isAlternateBuffer: false,
      isRawKeyboardModeActive: false,
      submitCommand: submitCommandMock,
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

const FAILED_CMD = "DISM.exe /Online /Cleanup-Image /CheckHealth";

beforeEach(() => {
  resolveElevate = null;
  invokeMock.mockClear();
  submitCommandMock.mockClear();
  capturedCallbacks.onCommandSettled = undefined;
  capturedCallbacks.onCommandStarted = undefined;
  capturedCallbacks.onPromptReady = undefined;
});

afterEach(() => {
  vi.useRealTimers();
});

/** 跑一條指令並讓它以權限不足失敗，等 question banner 出現。 */
async function runFailingCommand() {
  await act(async () => {
    capturedCallbacks.onCommandStarted?.(FAILED_CMD);
    capturedCallbacks.onCommandSettled?.(740);
  });
  expect(await screen.findByText(t.elevation_banner_question)).toBeTruthy();
}

function renderView() {
  return render(
    <LocaleProvider>
      <MemoryRouter>
        <TerminalView tabId="tab-1" />
      </MemoryRouter>
    </LocaleProvider>,
  );
}

describe("提權成功後自動重跑失敗的指令", () => {
  it("提權成功、提權 shell 的提示字元就緒後，自動重送原本失敗的那條指令", async () => {
    const user = userEvent.setup();
    renderView();
    await runFailingCommand();

    await user.click(screen.getByText(t.elevation_banner_confirm));
    await act(async () => {
      resolveElevate?.(true);
    });

    // 提權成功但 shell 還沒就緒——這段期間不可以送出任何東西，ConPTY 在
    // shell 就緒前收到的輸入會被直接丟掉（實機量到約 1.8 秒的空窗）。
    expect(submitCommandMock).not.toHaveBeenCalled();

    // 提權 shell 送出 OSC 133 B＝可以收輸入了。
    await act(async () => {
      capturedCallbacks.onPromptReady?.();
    });

    expect(submitCommandMock).toHaveBeenCalledWith(FAILED_CMD);
  });

  it("使用者在 UAC 按取消時不重跑", async () => {
    const user = userEvent.setup();
    renderView();
    await runFailingCommand();

    await user.click(screen.getByText(t.elevation_banner_confirm));
    await act(async () => {
      resolveElevate?.(false);
    });
    await act(async () => {
      capturedCallbacks.onPromptReady?.();
    });

    expect(submitCommandMock).not.toHaveBeenCalled();
  });

  it("提權成功之前抵達的提示字元不會觸發重跑", async () => {
    // 這是最容易寫錯、而且後果最嚴重的一個案例：那條失敗指令跑完之後，
    // **一般**（未提權）shell 自己就會馬上送一次 OSC 133 B。如果重跑的條件
    // 只看「有沒有待重跑的指令」而不看「提權是否已經成功」，這個 B 會在使用
    // 者都還沒按確認之前就把指令重送進未提權的 shell，於是再次失敗、再次跳
    // banner，形成無限迴圈。
    renderView();
    await runFailingCommand();

    await act(async () => {
      capturedCallbacks.onPromptReady?.();
    });

    expect(submitCommandMock).not.toHaveBeenCalled();
  });

  it("等待就緒期間使用者自己送了別的指令，就放棄重跑", async () => {
    const user = userEvent.setup();
    renderView();
    await runFailingCommand();

    await user.click(screen.getByText(t.elevation_banner_confirm));
    await act(async () => {
      resolveElevate?.(true);
    });

    // 使用者不等了，自己先送了別的指令。
    await act(async () => {
      capturedCallbacks.onCommandStarted?.("whoami");
    });
    await act(async () => {
      capturedCallbacks.onPromptReady?.();
    });

    // 那條舊的失敗指令不該在這時候突然自己冒出來跑。
    expect(submitCommandMock).not.toHaveBeenCalled();
  });
});
