import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";

const invokeAiChatCtx = vi.fn();
const abortAi = vi.fn().mockResolvedValue(undefined);
vi.mock("../../ipc/ai", () => ({
  invokeAiChatCtx: (...a: unknown[]) => invokeAiChatCtx(...a),
  abortAi: (...a: unknown[]) => abortAi(...a),
  formatAiError: (e: { kind: string }) => `ERR:${e.kind}`,
}));

let screenText: string | null = "\x1b[32m❯ 幫我寫一個函式\x1b[0m\nClaude: 完成了";
const fillTerminalInput = vi.fn().mockResolvedValue(true);
vi.mock("../../lib/terminalInstanceRegistry", () => ({
  serializeTerminal: () => screenText,
  fillTerminalInput: (...a: unknown[]) => fillTerminalInput(...a),
}));

vi.mock("../../contexts/LocaleContext", async () => {
  const { translations } = await vi.importActual<typeof import("../../lib/i18n")>("../../lib/i18n");
  return { useLocale: () => ({ locale: "zh-TW" as const, t: translations["zh-TW"], setLocale: () => {} }) };
});

import { PromptSuggestions } from "./PromptSuggestions";

const reply = (items: { title: string; prompt: string }[]) =>
  ({ content: JSON.stringify(items), tool_calls: [], tool_calling_unsupported: false });

const A = [{ title: "補測試", prompt: "幫剛才的函式補單元測試" }, { title: "重構", prompt: "把它重構得更簡潔" }];

async function flush() { await act(async () => { await vi.advanceTimersByTimeAsync(0); }); }
async function advance(ms: number) { await act(async () => { await vi.advanceTimersByTimeAsync(ms); }); }

const setup = (p: Partial<React.ComponentProps<typeof PromptSuggestions>> = {}) =>
  render(<PromptSuggestions sessionId="s1" providerId="p1" disabled={false} getIdleMs={() => 60_000} {...p} />);

beforeEach(() => {
  vi.useFakeTimers();
  invokeAiChatCtx.mockReset();
  abortAi.mockClear();
  fillTerminalInput.mockClear();
  screenText = "\x1b[32m❯ 幫我寫一個函式\x1b[0m\nClaude: 完成了";
  localStorage.clear();
});
afterEach(() => vi.useRealTimers());

describe("PromptSuggestions", () => {
  it("generates on click using a separate stream id and ANSI-stripped screen text", async () => {
    invokeAiChatCtx.mockResolvedValue(reply(A));
    setup();
    fireEvent.click(screen.getByText("產生建議"));
    await flush();
    expect(invokeAiChatCtx).toHaveBeenCalledTimes(1);
    const [messages, , connId, providerId] = invokeAiChatCtx.mock.calls[0];
    expect(connId).toBe("suggest-s1");
    expect(providerId).toBe("p1");
    expect(messages[0].role).toBe("user");
    expect(messages[0].content).toContain("Claude: 完成了");
    expect(messages[0].content).not.toContain("\x1b[");
    expect(screen.getByText("補測試")).toBeTruthy();
    expect(screen.getByText("重構")).toBeTruthy();
  });

  it("clicking a card fills the terminal with the prompt and does not send Enter", async () => {
    invokeAiChatCtx.mockResolvedValue(reply(A));
    setup();
    fireEvent.click(screen.getByText("產生建議"));
    await flush();
    fireEvent.click(screen.getByText("補測試"));
    await flush();
    expect(fillTerminalInput).toHaveBeenCalledWith("s1", "幫剛才的函式補單元測試");
    expect(fillTerminalInput.mock.calls[0][1]).not.toMatch(/[\r\n]/);
  });

  it("is disabled while Ask AI is streaming or an agent runs", async () => {
    setup({ disabled: true });
    const btn = screen.getByText("產生建議").closest("button")!;
    expect(btn.disabled).toBe(true);
    fireEvent.click(btn);
    await flush();
    expect(invokeAiChatCtx).not.toHaveBeenCalled();
  });

  it("is disabled while the terminal is still producing output", async () => {
    setup({ getIdleMs: () => 100 });
    await flush();
    expect(screen.getByText("產生建議").closest("button")!.disabled).toBe(true);
    expect(screen.getByText("終端機執行中，閒置後再產生")).toBeTruthy();
  });

  it("does not call the AI when the terminal has nothing readable", async () => {
    screenText = "  \n ";
    setup();
    fireEvent.click(screen.getByText("產生建議"));
    await flush();
    expect(invokeAiChatCtx).not.toHaveBeenCalled();
    expect(screen.getByText("終端機沒有可讀的內容")).toBeTruthy();
  });

  it("shows an error with a retry that works", async () => {
    invokeAiChatCtx.mockRejectedValueOnce({ kind: "network", message: "x" });
    setup();
    fireEvent.click(screen.getByText("產生建議"));
    await flush();
    expect(screen.getByText(/ERR:network/)).toBeTruthy();
    invokeAiChatCtx.mockResolvedValueOnce(reply(A));
    fireEvent.click(screen.getByText("重試"));
    await flush();
    expect(screen.getByText("補測試")).toBeTruthy();
    expect(screen.queryByText(/ERR:network/)).toBeNull();
  });

  it("says so when the reply has no usable suggestions", async () => {
    invokeAiChatCtx.mockResolvedValue({ content: "抱歉我不知道", tool_calls: [], tool_calling_unsupported: false });
    setup();
    fireEvent.click(screen.getByText("產生建議"));
    await flush();
    expect(screen.getByText("沒有產生可用的建議")).toBeTruthy();
  });

  it("discards a stale reply when a newer request was started", async () => {
    let resolveFirst!: (v: unknown) => void;
    invokeAiChatCtx.mockImplementationOnce(() => new Promise((r) => { resolveFirst = r; }));
    invokeAiChatCtx.mockResolvedValueOnce(reply([{ title: "新的", prompt: "new" }]));
    setup();
    fireEvent.click(screen.getByText("產生建議"));
    await flush();
    // 第二次：先取消上一個（loading 中按鈕變成「重新產生」仍可按）
    fireEvent.click(screen.getByText("重新產生"));
    await flush();
    expect(abortAi).toHaveBeenCalledWith("suggest-s1");
    expect(screen.getByText("新的")).toBeTruthy();
    await act(async () => { resolveFirst(reply([{ title: "舊的", prompt: "old" }])); });
    await flush();
    expect(screen.queryByText("舊的")).toBeNull();
    expect(screen.getByText("新的")).toBeTruthy();
  });

  it("auto mode generates once when the terminal goes from busy to idle, not again for the same screen", async () => {
    invokeAiChatCtx.mockResolvedValue(reply(A));
    let idle = 100;
    setup({ getIdleMs: () => idle });
    fireEvent.click(screen.getByTitle(/閒置後自動產生/));
    await advance(1000);
    expect(invokeAiChatCtx).not.toHaveBeenCalled();
    idle = 60_000;
    await advance(1000);
    expect(invokeAiChatCtx).toHaveBeenCalledTimes(1);
    // 又忙又閒，但畫面沒變 → 不重複產生
    idle = 100; await advance(1000);
    idle = 60_000; await advance(1000);
    expect(invokeAiChatCtx).toHaveBeenCalledTimes(1);
    // 畫面變了 → 會再產生
    screenText = "新的畫面內容";
    idle = 100; await advance(1000);
    idle = 60_000; await advance(1000);
    expect(invokeAiChatCtx).toHaveBeenCalledTimes(2);
  });

  it("one busy period triggers at most one automatic generation, even if props change while idle", async () => {
    invokeAiChatCtx.mockResolvedValue(reply(A));
    let idle = 100;
    const props = { sessionId: "s1", providerId: "p1", getIdleMs: () => idle };
    const { rerender } = render(<PromptSuggestions {...props} disabled={false} />);
    fireEvent.click(screen.getByTitle(/閒置後自動產生/));
    await advance(1000);
    idle = 60_000;
    await advance(1000);
    expect(invokeAiChatCtx).toHaveBeenCalledTimes(1);
    screenText = "畫面變了但終端機一直閒置";
    rerender(<PromptSuggestions {...props} disabled={true} />);
    rerender(<PromptSuggestions {...props} disabled={false} />);
    await flush();
    expect(invokeAiChatCtx).toHaveBeenCalledTimes(1);
  });

  it("does not generate automatically when auto is off", async () => {
    let idle = 100;
    setup({ getIdleMs: () => idle });
    await advance(1000);
    idle = 60_000;
    await advance(2000);
    expect(invokeAiChatCtx).not.toHaveBeenCalled();
  });

  it("does not update state or throw after unmount", async () => {
    let resolveIt!: (v: unknown) => void;
    invokeAiChatCtx.mockImplementation(() => new Promise((r) => { resolveIt = r; }));
    const { unmount } = setup();
    fireEvent.click(screen.getByText("產生建議"));
    await flush();
    unmount();
    expect(abortAi).toHaveBeenCalledWith("suggest-s1");
    await act(async () => { resolveIt(reply(A)); });
    await flush();
  });
});
