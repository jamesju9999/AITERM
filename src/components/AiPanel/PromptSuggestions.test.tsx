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
const submitTerminalInput = vi.fn().mockResolvedValue(true);
vi.mock("../../lib/terminalInstanceRegistry", () => ({
  serializeTerminal: () => screenText,
  fillTerminalInput: (...a: unknown[]) => fillTerminalInput(...a),
  submitTerminalInput: (...a: unknown[]) => submitTerminalInput(...a),
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
  submitTerminalInput.mockClear();
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
    expect(fillTerminalInput).toHaveBeenCalledWith("s1", "幫剛才的函式補單元測試", { replace: false });
    expect(fillTerminalInput.mock.calls[0][1]).not.toMatch(/[\r\n]/);
  });

  it("a single click only fills; it never submits", async () => {
    invokeAiChatCtx.mockResolvedValue(reply(A));
    setup();
    fireEvent.click(screen.getByText("產生建議"));
    await flush();
    fireEvent.click(screen.getByText("補測試"), { detail: 1 });
    await flush();
    expect(fillTerminalInput).toHaveBeenCalledTimes(1);
    expect(submitTerminalInput).not.toHaveBeenCalled();
  });

  it("a double click fills once (first click) and then submits once, without pasting twice", async () => {
    invokeAiChatCtx.mockResolvedValue(reply(A));
    setup();
    fireEvent.click(screen.getByText("產生建議"));
    await flush();
    const card = screen.getByText("補測試");
    fireEvent.click(card, { detail: 1 });
    fireEvent.click(card, { detail: 2 });
    await flush();
    expect(fillTerminalInput).toHaveBeenCalledTimes(1);
    expect(submitTerminalInput).toHaveBeenCalledTimes(1);
    expect(submitTerminalInput).toHaveBeenCalledWith("s1");
  });

  it("shows the full prompt text under each title so the user can judge before clicking", async () => {
    invokeAiChatCtx.mockResolvedValue(reply(A));
    setup();
    fireEvent.click(screen.getByText("產生建議"));
    await flush();
    expect(screen.getByText("幫剛才的函式補單元測試")).toBeTruthy();
    expect(screen.getByText("把它重構得更簡潔")).toBeTruthy();
  });

  it("explains itself before the first run", () => {
    setup();
    expect(screen.getByText(/依目前終端機畫面/)).toBeTruthy();
  });

  it("confirms a single click with 已填入 and a double click with 已送出, then clears it", async () => {
    invokeAiChatCtx.mockResolvedValue(reply(A));
    setup();
    fireEvent.click(screen.getByText("產生建議"));
    await flush();
    expect(screen.queryByText("已填入")).toBeNull();
    const card = screen.getByText("補測試");
    fireEvent.click(card, { detail: 1 });
    await flush();
    expect(screen.getByText("已填入")).toBeTruthy();
    fireEvent.click(card, { detail: 2 });
    await flush();
    expect(screen.getByText("已送出")).toBeTruthy();
    expect(screen.queryByText("已填入")).toBeNull();
    await advance(2000);
    expect(screen.queryByText("已送出")).toBeNull();
  });

  it("can be collapsed, keeps the results, and remembers the choice", async () => {
    invokeAiChatCtx.mockResolvedValue(reply(A));
    const first = setup();
    fireEvent.click(screen.getByText("產生建議"));
    await flush();
    const toggle = screen.getByRole("button", { name: /下一步建議/ });
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    fireEvent.click(toggle);
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByText("補測試")).toBeNull();
    fireEvent.click(toggle);
    expect(screen.getByText("補測試")).toBeTruthy();
    fireEvent.click(toggle);
    first.unmount();
    setup();
    expect(screen.getByRole("button", { name: /下一步建議/ }).getAttribute("aria-expanded")).toBe("false");
  });

  it("shows a busy placeholder while generating", async () => {
    invokeAiChatCtx.mockImplementation(() => new Promise(() => {}));
    setup();
    fireEvent.click(screen.getByText("產生建議"));
    await flush();
    expect(screen.getByRole("status", { name: "產生建議中…" })).toBeTruthy();
  });

  it("hides the old cards while regenerating so stale suggestions cannot be clicked", async () => {
    invokeAiChatCtx.mockResolvedValueOnce(reply(A));
    setup();
    fireEvent.click(screen.getByText("產生建議"));
    await flush();
    expect(screen.getByText("補測試")).toBeTruthy();
    invokeAiChatCtx.mockImplementationOnce(() => new Promise(() => {}));
    fireEvent.click(screen.getByText("重新產生"));
    await flush();
    expect(screen.queryByText("補測試")).toBeNull();
    expect(screen.getByRole("status", { name: "產生建議中…" })).toBeTruthy();
  });

  describe("replacing a previous, unsent suggestion", () => {
    const run = async (getIdleMs?: () => number) => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      setup(getIdleMs ? { getIdleMs } : {});
      fireEvent.click(screen.getByText("產生建議"));
      await flush();
    };

    it("clicking another suggestion replaces the one that was filled but not sent", async () => {
      await run();
      fireEvent.click(screen.getByText("補測試"), { detail: 1 });
      fireEvent.click(screen.getByText("重構"), { detail: 1 });
      await flush();
      expect(fillTerminalInput.mock.calls[0][2]).toEqual({ replace: false });
      expect(fillTerminalInput.mock.calls[1][1]).toBe("把它重構得更簡潔");
      expect(fillTerminalInput.mock.calls[1][2]).toEqual({ replace: true });
    });

    it("does not replace after the previous one was sent with a double click", async () => {
      await run();
      const card = screen.getByText("補測試");
      fireEvent.click(card, { detail: 1 });
      fireEvent.click(card, { detail: 2 });
      fireEvent.click(screen.getByText("重構"), { detail: 1 });
      await flush();
      expect(fillTerminalInput.mock.calls[1][2]).toEqual({ replace: false });
    });

    it("does not replace once the terminal has started working (the user sent it themselves)", async () => {
      let idle = 60_000;
      await run(() => idle);
      fireEvent.click(screen.getByText("補測試"), { detail: 1 });
      idle = 100; await advance(5000);
      idle = 60_000; await advance(1000);
      fireEvent.click(screen.getByText("重構"), { detail: 1 });
      await flush();
      expect(fillTerminalInput.mock.calls[1][2]).toEqual({ replace: false });
    });

    it("still replaces after the short echo burst our own fill causes", async () => {
      let idle = 60_000;
      await run(() => idle);
      fireEvent.click(screen.getByText("補測試"), { detail: 1 });
      idle = 100; await advance(2000); // 回顯：只忙一兩次輪詢
      idle = 60_000; await advance(1000);
      fireEvent.click(screen.getByText("重構"), { detail: 1 });
      await flush();
      expect(fillTerminalInput.mock.calls[1][2]).toEqual({ replace: true });
    });
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
