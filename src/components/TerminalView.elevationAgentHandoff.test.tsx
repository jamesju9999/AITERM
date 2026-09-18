import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";

// 同 TerminalView.elevationBannerRace.test.tsx 的 baseline：mock 三個 Tauri
// entry point 就能讓 TerminalView 在 jsdom 完整掛載。`pty_elevate` 的回覆
// 由測試自己控制時機，因為這個檔案要驗的正是「提權成功之前／之後」的差別。
let resolveElevate: ((started: boolean) => void) | null = null;
let permissionDenied = true;
const invokeMock = vi.fn((cmd: string) => {
  if (cmd === "pty_create") return Promise.resolve("test-session");
  if (cmd === "pty_check_permission_denied") return Promise.resolve(permissionDenied);
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

// AI 面板換成替身，只為了拿到 TerminalView 交給它的 onExecuteCommand／
// onInterruptCommand——agent 迴圈就是透過這兩個入口送指令、等結果。
type Block = { id: string; command: string; status: string; exitCode?: number; startTime: number; rawOutput: string };
const aiPanelProps: {
  onExecuteCommand?: (cmd: string, onComplete?: (block: Block) => void) => void;
  onInterruptCommand?: () => void;
  onAgentAborted?: () => void;
} = {};
vi.mock("./AiPanel", () => ({
  AiPanel: (props: typeof aiPanelProps) => {
    aiPanelProps.onExecuteCommand = props.onExecuteCommand;
    aiPanelProps.onInterruptCommand = props.onInterruptCommand;
    aiPanelProps.onAgentAborted = props.onAgentAborted;
    return null;
  },
}));

import { TerminalView } from "./TerminalView";
import { LocaleProvider } from "../contexts/LocaleContext";
import { translations } from "../lib/i18n";

const t = translations["zh-TW"];

const FAILED_CMD = "DISM.exe /Online /Cleanup-Image /CheckHealth";

function block(exitCode: number, id = "b1"): Block {
  return { id, command: FAILED_CMD, status: exitCode === 0 ? "completed" : "failed", exitCode, startTime: 0, rawOutput: "" };
}

beforeEach(() => {
  resolveElevate = null;
  permissionDenied = true;
  invokeMock.mockClear();
  submitCommandMock.mockReset();
  capturedCallbacks.onCommandSettled = undefined;
  capturedCallbacks.onCommandStarted = undefined;
  capturedCallbacks.onPromptReady = undefined;
  aiPanelProps.onExecuteCommand = undefined;
  aiPanelProps.onInterruptCommand = undefined;
});

afterEach(() => {
  vi.useRealTimers();
});

function renderView() {
  return render(
    <LocaleProvider>
      <MemoryRouter>
        <TerminalView tabId="tab-1" />
      </MemoryRouter>
    </LocaleProvider>,
  );
}

/**
 * 模擬 agent 送出一條指令、shell 以 exitCode 結束，照 useTerminalBlocks 的
 * 真實時序：submitCommand 同步呼叫 onCommandStarted；OSC 133 D 時
 * finalizeBlock 把完成 callback 排進 setTimeout(50)，接著同步呼叫
 * onCommandSettled——所以 onCommandSettled（和它發出的權限查詢）一定比完成
 * callback 早。權限查詢在這裡是立即 resolve 的 Promise，也就是「查詢在
 * callback 之前就回來了」這個最容易漏接的情境。回傳 agent 的完成 callback 替身。
 */
async function agentRunsFailingCommand(exitCode = 740) {
  const onComplete = vi.fn();
  submitCommandMock.mockImplementation(() => capturedCallbacks.onCommandStarted?.(FAILED_CMD));
  await act(async () => {
    aiPanelProps.onExecuteCommand?.(FAILED_CMD, onComplete);
  });
  submitCommandMock.mockImplementation(() => {});
  const blocksComplete = submitCommandMock.mock.calls[0][1] as (b: Block) => void;
  await act(async () => {
    capturedCallbacks.onCommandSettled?.(exitCode);
  });
  await act(async () => {
    blocksComplete(block(exitCode));
  });
  submitCommandMock.mockClear();
  return onComplete;
}

async function waitForAiPanel() {
  await act(async () => {});
  await vi.waitFor(() => expect(aiPanelProps.onExecuteCommand).toBeTruthy());
}

describe("agent 的指令因權限不足失敗時，提權重跑的結果要交回 agent", () => {
  it("提權並重跑成功後，agent 收到的是重跑的結果，而不是 740", async () => {
    const user = userEvent.setup();
    renderView();
    await waitForAiPanel();
    const onComplete = await agentRunsFailingCommand();

    expect(await screen.findByText(t.elevation_banner_question)).toBeTruthy();
    // 關鍵：740 不能先交給 agent——它一收到就會照 system prompt 結束迴圈，
    // 之後重跑成功也沒有人接手。
    expect(onComplete).not.toHaveBeenCalled();

    await user.click(screen.getByText(t.elevation_banner_confirm));
    await act(async () => { resolveElevate?.(true); });
    await act(async () => { capturedCallbacks.onPromptReady?.(); });

    expect(submitCommandMock).toHaveBeenCalledTimes(1);
    expect(submitCommandMock.mock.calls[0][0]).toBe(FAILED_CMD);
    const retryComplete = submitCommandMock.mock.calls[0][1] as (b: Block) => void;
    expect(typeof retryComplete).toBe("function");

    const retryBlock = block(0, "b2");
    await act(async () => { retryComplete(retryBlock); });

    expect(onComplete).toHaveBeenCalledTimes(1);
    expect(onComplete).toHaveBeenCalledWith(retryBlock);
  });

  it("不是權限問題的失敗，照常把原本的結果交給 agent", async () => {
    permissionDenied = false;
    renderView();
    await waitForAiPanel();
    const onComplete = await agentRunsFailingCommand(1);

    await vi.waitFor(() => expect(onComplete).toHaveBeenCalledTimes(1));
    expect(onComplete.mock.calls[0][0].exitCode).toBe(1);
  });

  it("成功的指令立刻交給 agent，不查權限", async () => {
    renderView();
    await waitForAiPanel();
    const onComplete = await agentRunsFailingCommand(0);

    expect(onComplete).toHaveBeenCalledTimes(1);
    expect(invokeMock).not.toHaveBeenCalledWith("pty_check_permission_denied");
  });

  it("使用者在橫幅按取消，agent 收到原本的 740", async () => {
    const user = userEvent.setup();
    renderView();
    await waitForAiPanel();
    const onComplete = await agentRunsFailingCommand();
    await screen.findByText(t.elevation_banner_question);

    await user.click(screen.getByText(t.elevation_banner_cancel));

    expect(onComplete).toHaveBeenCalledTimes(1);
    expect(onComplete.mock.calls[0][0].exitCode).toBe(740);
  });

  it("使用者在 UAC 按取消，agent 收到原本的 740", async () => {
    const user = userEvent.setup();
    renderView();
    await waitForAiPanel();
    const onComplete = await agentRunsFailingCommand();
    await screen.findByText(t.elevation_banner_question);

    await user.click(screen.getByText(t.elevation_banner_confirm));
    await act(async () => { resolveElevate?.(false); });

    expect(onComplete).toHaveBeenCalledTimes(1);
    expect(onComplete.mock.calls[0][0].exitCode).toBe(740);
  });

  it("使用者不理橫幅、自己跑了別的指令，agent 收到原本的 740", async () => {
    renderView();
    await waitForAiPanel();
    const onComplete = await agentRunsFailingCommand();
    await screen.findByText(t.elevation_banner_question);

    await act(async () => { capturedCallbacks.onCommandStarted?.("whoami"); });

    expect(onComplete).toHaveBeenCalledTimes(1);
    expect(onComplete.mock.calls[0][0].exitCode).toBe(740);
  });

  it("卡住偵測的「中斷」也會放行，agent 不會永遠等下去", async () => {
    renderView();
    await waitForAiPanel();
    const onComplete = await agentRunsFailingCommand();
    await screen.findByText(t.elevation_banner_question);

    await act(async () => { aiPanelProps.onInterruptCommand?.(); });

    expect(onComplete).toHaveBeenCalledTimes(1);
    expect(onComplete.mock.calls[0][0].exitCode).toBe(740);
  });

  it("使用者按了 AI 面板的停止鍵，agent 立刻收到原本的 740", async () => {
    renderView();
    await waitForAiPanel();
    const onComplete = await agentRunsFailingCommand();
    await screen.findByText(t.elevation_banner_question);

    await act(async () => { aiPanelProps.onAgentAborted?.(); });

    expect(onComplete).toHaveBeenCalledTimes(1);
    expect(onComplete.mock.calls[0][0].exitCode).toBe(740);
  });

  it("強制結案（-1）不等權限查詢，直接交給 agent", async () => {
    // -1 是 finalizeBlock 的強制結案哨兵值，之後不會有 onCommandSettled
    // 來決定要不要放行——扣住就永遠放不掉。
    renderView();
    await waitForAiPanel();
    const onComplete = vi.fn();
    await act(async () => { aiPanelProps.onExecuteCommand?.(FAILED_CMD, onComplete); });
    const blocksComplete = submitCommandMock.mock.calls[0][1] as (b: Block) => void;
    await act(async () => { blocksComplete(block(-1)); });

    expect(onComplete).toHaveBeenCalledTimes(1);
  });
});

describe("放行的決定比完成 callback 早到", () => {
  it("callback 還沒抵達前使用者就按了取消，晚到的 callback 直接交出 740", async () => {
    const user = userEvent.setup();
    renderView();
    await waitForAiPanel();
    const onComplete = vi.fn();
    submitCommandMock.mockImplementation(() => capturedCallbacks.onCommandStarted?.(FAILED_CMD));
    await act(async () => { aiPanelProps.onExecuteCommand?.(FAILED_CMD, onComplete); });
    submitCommandMock.mockImplementation(() => {});
    const blocksComplete = submitCommandMock.mock.calls[0][1] as (b: Block) => void;
    await act(async () => { capturedCallbacks.onCommandSettled?.(740); });
    await screen.findByText(t.elevation_banner_question);

    await user.click(screen.getByText(t.elevation_banner_cancel));
    await act(async () => { blocksComplete(block(740)); });

    expect(onComplete).toHaveBeenCalledTimes(1);
    expect(onComplete.mock.calls[0][0].exitCode).toBe(740);
  });

  it("上一條的放行不會讓下一條權限不足的 agent 指令被直接交出", async () => {
    renderView();
    await waitForAiPanel();
    // 第一條：非權限問題的失敗，會放行。
    permissionDenied = false;
    await agentRunsFailingCommand(1);
    // 第二條：權限不足，應該被扣住。
    permissionDenied = true;
    const second = await agentRunsFailingCommand(740);

    expect(second).not.toHaveBeenCalled();
  });
});
