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

import { PromptSuggestions, AUTO_MIN_INTERVAL_MS } from "./PromptSuggestions";

const reply = (items: { title: string; prompt: string }[]) =>
  ({ content: JSON.stringify(items), tool_calls: [], tool_calling_unsupported: false });

const A = [{ title: "補測試", prompt: "幫剛才的函式補單元測試" }, { title: "重構", prompt: "把它重構得更簡潔" }];

async function flush() { await act(async () => { await vi.advanceTimersByTimeAsync(0); }); }
async function advance(ms: number) { await act(async () => { await vi.advanceTimersByTimeAsync(ms); }); }

const setup = (p: Partial<React.ComponentProps<typeof PromptSuggestions>> = {}) =>
  render(<PromptSuggestions sessionId="s1" providerId="p1" disabled={false} getIdleMs={() => 60_000} aiCliRunning redact {...p} />);

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
    expect(fillTerminalInput).toHaveBeenCalledWith("s1", "幫剛才的函式補單元測試");
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

  it("explains itself before the first run, including that screens are sent to the AI provider", () => {
    setup();
    expect(screen.getByText(/依目前終端機畫面/)).toBeTruthy();
    expect(screen.getByText(/送給你設定的 AI 供應商/)).toBeTruthy();
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

  it("with hideTitle there is no collapsible title and the content is always open", async () => {
    localStorage.setItem("aiterm-suggest-collapsed", "true");
    setup({ hideTitle: true });
    expect(screen.queryByRole("button", { name: /下一步建議/ })).toBeNull();
    expect(screen.getByText("產生建議")).toBeTruthy();
  });

  describe("goal", () => {
    it("includes the goal in the request", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      setup({ goal: "把舊系統轉成網頁版" });
      fireEvent.click(screen.getByText("產生建議"));
      await flush();
      expect(invokeAiChatCtx.mock.calls[0][0][0].content).toContain("把舊系統轉成網頁版");
    });

    it("sends no goal text when there is none", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      setup();
      fireEvent.click(screen.getByText("產生建議"));
      await flush();
      expect(invokeAiChatCtx.mock.calls[0][0][0].content).not.toContain("大目標");
    });

    it("drops the suggestions made for the old goal and goes back to the start when the goal changes", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      const props = { sessionId: "s1", providerId: "p1", disabled: false, getIdleMs: () => 60_000, aiCliRunning: true, redact: true };
      const { rerender } = render(<PromptSuggestions {...props} goal="舊目標" />);
      fireEvent.click(screen.getByText("產生建議"));
      await flush();
      expect(screen.getByText("補測試")).toBeTruthy();
      rerender(<PromptSuggestions {...props} goal="新目標" />);
      await flush();
      expect(screen.queryByText("補測試")).toBeNull();
      expect(screen.getByText("產生建議")).toBeTruthy();
    });

    it("ignores a reply that was still in flight when the goal changed", async () => {
      let resolveIt!: (v: unknown) => void;
      invokeAiChatCtx.mockImplementationOnce(() => new Promise((r) => { resolveIt = r; }));
      const props = { sessionId: "s1", providerId: "p1", disabled: false, getIdleMs: () => 60_000, aiCliRunning: true, redact: true };
      const { rerender } = render(<PromptSuggestions {...props} goal="舊目標" />);
      fireEvent.click(screen.getByText("產生建議"));
      await flush();
      rerender(<PromptSuggestions {...props} goal="新目標" />);
      await flush();
      expect(abortAi).toHaveBeenCalledWith("suggest-s1");
      await act(async () => { resolveIt(reply(A)); });
      await flush();
      expect(screen.queryByText("補測試")).toBeNull();
      expect(screen.getByText("產生建議")).toBeTruthy();
    });

    it("lets auto mode regenerate for the same screen after the goal changed", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      let idle = 100;
      const props = { sessionId: "s1", providerId: "p1", disabled: false, getIdleMs: () => idle, aiCliRunning: true, redact: true };
      const { rerender } = render(<PromptSuggestions {...props} goal="目標一" />);
      fireEvent.click(screen.getByTitle(/閒置後自動產生/));
      await advance(1000);
      idle = 60_000; await advance(1000);
      expect(invokeAiChatCtx).toHaveBeenCalledTimes(1);
      rerender(<PromptSuggestions {...props} goal="目標二" />);
      idle = 100; await advance(1000);
      idle = 60_000; await advance(1000);
      await advance(AUTO_MIN_INTERVAL_MS); // 兩次自動產生之間要隔冷卻時間
      expect(invokeAiChatCtx).toHaveBeenCalledTimes(2);
    });
  });

  describe("screen history", () => {
    const old1 = "舊畫面一-0\n舊畫面一-1\n舊畫面一-2\n舊畫面一-3\n舊畫面一-4\n舊畫面一-5";
    const old2 = "舊畫面二-0\n舊畫面二-1\n舊畫面二-2\n舊畫面二-3\n舊畫面二-4\n舊畫面二-5";

    it("sends the older screens, oldest first and before the current screen", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      setup({ getHistory: () => [old1, old2] });
      fireEvent.click(screen.getByText("產生建議"));
      await flush();
      const content = invokeAiChatCtx.mock.calls[0][0][0].content as string;
      expect(content).toContain("較早的畫面");
      expect(content.indexOf("舊畫面一-0")).toBeLessThan(content.indexOf("舊畫面二-0"));
      expect(content.indexOf("舊畫面二-0")).toBeLessThan(content.indexOf("Claude: 完成了"));
    });

    it("does not repeat the current screen when the newest history entry is just that screen", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      setup({ getHistory: () => [old1, "❯ 幫我寫一個函式\nClaude: 完成了"] });
      fireEvent.click(screen.getByText("產生建議"));
      await flush();
      const content = invokeAiChatCtx.mock.calls[0][0][0].content as string;
      expect(content).toContain("舊畫面一-0");
      expect(content.match(/Claude: 完成了/g)).toHaveLength(1);
    });

    it("sends no history section when there is none", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      setup({ getHistory: () => [] });
      fireEvent.click(screen.getByText("產生建議"));
      await flush();
      expect(invokeAiChatCtx.mock.calls[0][0][0].content).not.toContain("較早的畫面");
    });

    it("reads the history at generation time, not at mount", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      let hist: string[] = [];
      setup({ getHistory: () => hist });
      hist = [old1];
      fireEvent.click(screen.getByText("產生建議"));
      await flush();
      expect(invokeAiChatCtx.mock.calls[0][0][0].content).toContain("舊畫面一-0");
    });
  });

  describe("milestone focus", () => {
    it("sends the milestone section with the request", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      setup({ milestoneContext: "里程碑一節-MARK" });
      fireEvent.click(screen.getByText("產生建議"));
      await flush();
      expect(invokeAiChatCtx.mock.calls[0][0][0].content).toContain("里程碑一節-MARK");
    });

    it("sends nothing about milestones when there are none", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      setup();
      fireEvent.click(screen.getByText("產生建議"));
      await flush();
      expect(invokeAiChatCtx.mock.calls[0][0][0].content).not.toContain("里程碑");
    });

    it("shows which milestone the suggestions aim at", () => {
      setup({ focusLabel: "拆分登入模組" });
      expect(screen.getByText("朝向里程碑：拆分登入模組")).toBeTruthy();
    });

    it("says so when every milestone is done", () => {
      setup({ allMilestonesDone: true });
      expect(screen.getByText("全部里程碑已完成，建議改為驗收與收尾")).toBeTruthy();
    });

    it("shows neither line when there is no milestone information", () => {
      setup();
      expect(screen.queryByText(/朝向里程碑/)).toBeNull();
      expect(screen.queryByText(/全部里程碑已完成/)).toBeNull();
    });

    it("drops the old suggestions when the milestone context changes (they were made for another focus)", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      const props = { sessionId: "s1", providerId: "p1", disabled: false, getIdleMs: () => 60_000, aiCliRunning: true, redact: true };
      const { rerender } = render(<PromptSuggestions {...props} milestoneContext="焦點在第二個" />);
      fireEvent.click(screen.getByText("產生建議"));
      await flush();
      expect(screen.getByText("補測試")).toBeTruthy();
      rerender(<PromptSuggestions {...props} milestoneContext="焦點在第三個" />);
      await flush();
      expect(screen.queryByText("補測試")).toBeNull();
      expect(screen.getByText("產生建議")).toBeTruthy();
    });

    it("keeps the suggestions when nothing about the context changed", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      const props = { sessionId: "s1", providerId: "p1", disabled: false, getIdleMs: () => 60_000, aiCliRunning: true, redact: true };
      const { rerender } = render(<PromptSuggestions {...props} milestoneContext="同一份" />);
      fireEvent.click(screen.getByText("產生建議"));
      await flush();
      rerender(<PromptSuggestions {...props} milestoneContext="同一份" />);
      await flush();
      expect(screen.getByText("補測試")).toBeTruthy();
    });
  });

  describe("status line does not shift the layout", () => {
    const statusSlot = (c: HTMLElement) => c.querySelector(".aiterm-suggest__status") as HTMLElement | null;
    const cardsList = (c: HTMLElement) => c.querySelector(".aiterm-suggest__list") as HTMLElement;

    it("always renders the same status slot, so showing or hiding the busy message never moves the cards", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      const idle = { v: 60_000 };
      const { container } = setup({ getIdleMs: () => idle.v });
      fireEvent.click(screen.getByText("產生建議"));
      await flush();
      const slot = statusSlot(container)!;
      expect(slot).not.toBeNull();
      expect(slot.textContent).toBe("");
      const body = slot.parentElement!;
      const slotIndex = Array.from(body.children).indexOf(slot);
      const listIndex = Array.from(body.children).indexOf(cardsList(container));

      idle.v = 100; await advance(1000); // 終端機忙碌
      expect(screen.getByText("終端機執行中，閒置後再產生")).toBeTruthy();
      expect(statusSlot(container)).toBe(slot); // 同一個元素，不是重新長出來的
      expect(slot.textContent).toContain("終端機執行中");
      expect(Array.from(body.children).indexOf(slot)).toBe(slotIndex);
      expect(Array.from(body.children).indexOf(cardsList(container))).toBe(listIndex);

      idle.v = 60_000; await advance(1000); // 回到閒置
      expect(statusSlot(container)).toBe(slot);
      expect(slot.textContent).toBe("");
      expect(Array.from(body.children).indexOf(cardsList(container))).toBe(listIndex);
    });

    it("keeps the other messages (empty screen, no usable result) in the same slot", async () => {
      screenText = "  \n ";
      const { container } = setup();
      fireEvent.click(screen.getByText("產生建議"));
      await flush();
      expect(statusSlot(container)!.textContent).toContain("終端機沒有可讀的內容");
    });

    it("reserves a fixed height for the slot even when it is empty", () => {
      const { container } = setup();
      expect(statusSlot(container)!.style.minHeight).toBe("16px");
    });
  });

  describe("secret redaction", () => {
    const TOKEN = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    const sent = () => invokeAiChatCtx.mock.calls[0][0][0].content as string;

    it("masks secrets in the current screen before sending, and says how many", async () => {
      screenText = `❯ 部署\nexport GITHUB_TOKEN=${TOKEN}\npassword=hunter2hunter2\n完成`;
      invokeAiChatCtx.mockResolvedValue(reply(A));
      setup();
      fireEvent.click(screen.getByText("產生建議"));
      await flush();
      expect(sent()).not.toContain(TOKEN);
      expect(sent()).not.toContain("hunter2hunter2");
      expect(sent()).toContain("[已遮罩]");
      expect(sent()).toContain("完成"); // 其餘文字還在
      expect(screen.getByText("已在送出前遮罩 2 處疑似敏感資訊")).toBeTruthy();
    });

    it("masks secrets in the earlier screens too", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      setup({ getHistory: () => [`舊畫面一\n舊畫面二\n舊畫面三\n舊畫面四\n舊畫面五\nAuthorization: Bearer ${TOKEN}`] });
      fireEvent.click(screen.getByText("產生建議"));
      await flush();
      expect(sent()).toContain("舊畫面一");
      expect(sent()).not.toContain(TOKEN);
      expect(screen.getByText("已在送出前遮罩 1 處疑似敏感資訊")).toBeTruthy();
    });

    it("masks before truncating, so no fragment of a secret survives the cut at the 8000-char limit", async () => {
      // 讓「取尾端 8000 字」的切點落在 token 中間
      const suffix = "y ".repeat(3990).slice(0, 8000 - 20);
      screenText = `${"x ".repeat(2000)}${TOKEN}${suffix}`;
      invokeAiChatCtx.mockResolvedValue(reply(A));
      setup();
      fireEvent.click(screen.getByText("產生建議"));
      await flush();
      expect(sent()).not.toContain(TOKEN.slice(-20));
      expect(sent()).not.toContain(TOKEN.slice(-12));
    });

    it("sends the raw text and shows no count when redaction is turned off", async () => {
      screenText = `export GITHUB_TOKEN=${TOKEN}`;
      invokeAiChatCtx.mockResolvedValue(reply(A));
      setup({ redact: false });
      fireEvent.click(screen.getByText("產生建議"));
      await flush();
      expect(sent()).toContain(TOKEN);
      expect(screen.queryByText(/已在送出前遮罩/)).toBeNull();
    });

    it("clears the count together with the suggestions when the goal changes", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      screenText = `password=hunter2hunter2`;
      const props = { sessionId: "s1", providerId: "p1", disabled: false, getIdleMs: () => 60_000, aiCliRunning: true, redact: true };
      const { rerender } = render(<PromptSuggestions {...props} goal="舊目標" />);
      fireEvent.click(screen.getByText("產生建議"));
      await flush();
      expect(screen.getByText("已在送出前遮罩 1 處疑似敏感資訊")).toBeTruthy();
      rerender(<PromptSuggestions {...props} goal="新目標" />);
      await flush();
      expect(screen.queryByText(/已在送出前遮罩/)).toBeNull();
    });

    it("shows no count when nothing was masked", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      setup();
      fireEvent.click(screen.getByText("產生建議"));
      await flush();
      expect(screen.queryByText(/已在送出前遮罩/)).toBeNull();
    });

    it("the count refers to the latest request only", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      screenText = `password=hunter2hunter2`;
      setup();
      fireEvent.click(screen.getByText("產生建議"));
      await flush();
      expect(screen.getByText("已在送出前遮罩 1 處疑似敏感資訊")).toBeTruthy();
      screenText = "乾淨的畫面";
      fireEvent.click(screen.getByText("重新產生"));
      await flush();
      expect(screen.queryByText(/已在送出前遮罩/)).toBeNull();
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
    await advance(AUTO_MIN_INTERVAL_MS); // 兩次自動產生之間要隔冷卻時間
    expect(invokeAiChatCtx).toHaveBeenCalledTimes(2);
  });

  it("one busy period triggers at most one automatic generation, even if props change while idle", async () => {
    invokeAiChatCtx.mockResolvedValue(reply(A));
    let idle = 100;
    const props = { sessionId: "s1", providerId: "p1", getIdleMs: () => idle, aiCliRunning: true, redact: true };
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

  describe("auto mode guards", () => {
    const toggleAuto = () => fireEvent.click(screen.getByTitle(/閒置後自動產生/));

    it("never generates automatically while no AI tool is running, even after the terminal was busy", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      const idle = { v: 100 };
      setup({ getIdleMs: () => idle.v, aiCliRunning: false });
      toggleAuto();
      await advance(1000);
      idle.v = 60_000; await advance(2000);
      expect(invokeAiChatCtx).not.toHaveBeenCalled();
    });

    it("a busy period that ended before the tool started does not trigger a generation when the tool starts later", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      const idle = { v: 100 };
      const props = { sessionId: "s1", providerId: "p1", disabled: false, getIdleMs: () => idle.v, redact: true };
      const { rerender } = render(<PromptSuggestions {...props} aiCliRunning={false} />);
      toggleAuto();
      await advance(1000);
      idle.v = 60_000; await advance(2000); // 普通 shell 的輸出，沒有 AI 工具
      rerender(<PromptSuggestions {...props} aiCliRunning />); // 之後才啟動 AI 工具
      await advance(3000);
      expect(invokeAiChatCtx).not.toHaveBeenCalled();
    });

    it("waits out the minimum interval instead of calling again right away, then generates once", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      const idle = { v: 100 };
      setup({ getIdleMs: () => idle.v });
      toggleAuto();
      await advance(1000);
      idle.v = 60_000; await advance(1000);
      expect(invokeAiChatCtx).toHaveBeenCalledTimes(1);

      screenText = "第二輪的畫面";
      idle.v = 100; await advance(1000);
      idle.v = 60_000; await advance(1000);
      expect(invokeAiChatCtx).toHaveBeenCalledTimes(1); // 太快：先不呼叫

      await advance(AUTO_MIN_INTERVAL_MS);
      expect(invokeAiChatCtx).toHaveBeenCalledTimes(2); // 冷卻結束才呼叫
      await advance(AUTO_MIN_INTERVAL_MS * 2);
      expect(invokeAiChatCtx).toHaveBeenCalledTimes(2); // 只呼叫一次
    });

    it("cancels the waiting call if the terminal gets busy again before the interval is over", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      const idle = { v: 100 };
      setup({ getIdleMs: () => idle.v });
      toggleAuto();
      await advance(1000);
      idle.v = 60_000; await advance(1000);
      expect(invokeAiChatCtx).toHaveBeenCalledTimes(1);

      screenText = "第二輪";
      idle.v = 100; await advance(1000);
      idle.v = 60_000; await advance(1000); // 排進冷卻
      idle.v = 100; await advance(5000); // 又開始忙：等待中的呼叫要取消
      expect(invokeAiChatCtx).toHaveBeenCalledTimes(1);
      await advance(AUTO_MIN_INTERVAL_MS + 5000); // 一直忙，沒有轉閒置
      expect(invokeAiChatCtx).toHaveBeenCalledTimes(1);
    });

    it("does not fire the waiting call after auto is switched off", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      const idle = { v: 100 };
      setup({ getIdleMs: () => idle.v });
      toggleAuto();
      await advance(1000);
      idle.v = 60_000; await advance(1000);
      screenText = "第二輪";
      idle.v = 100; await advance(1000);
      idle.v = 60_000; await advance(1000);
      toggleAuto(); // 關掉
      await advance(AUTO_MIN_INTERVAL_MS + 1000);
      expect(invokeAiChatCtx).toHaveBeenCalledTimes(1);
    });

    it("does not fire the waiting call after the AI tool has ended", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      const idle = { v: 100 };
      const props = { sessionId: "s1", providerId: "p1", disabled: false, getIdleMs: () => idle.v, redact: true };
      const { rerender } = render(<PromptSuggestions {...props} aiCliRunning />);
      toggleAuto();
      await advance(1000);
      idle.v = 60_000; await advance(1000);
      screenText = "第二輪";
      idle.v = 100; await advance(1000);
      idle.v = 60_000; await advance(1000);
      rerender(<PromptSuggestions {...props} aiCliRunning={false} />); // 工具結束
      await advance(AUTO_MIN_INTERVAL_MS + 1000);
      expect(invokeAiChatCtx).toHaveBeenCalledTimes(1);
    });

    it("does not fire the waiting call if Ask AI started running in the meantime", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      const idle = { v: 100 };
      const props = { sessionId: "s1", providerId: "p1", getIdleMs: () => idle.v, aiCliRunning: true, redact: true };
      const { rerender } = render(<PromptSuggestions {...props} disabled={false} />);
      toggleAuto();
      await advance(1000);
      idle.v = 60_000; await advance(1000);
      screenText = "第二輪";
      idle.v = 100; await advance(1000);
      idle.v = 60_000; await advance(1000);
      rerender(<PromptSuggestions {...props} disabled />); // Ask AI 開始回答
      await advance(AUTO_MIN_INTERVAL_MS + 1000);
      expect(invokeAiChatCtx).toHaveBeenCalledTimes(1);
    });

    it("does not lose the trigger when Ask AI was busy at the moment the terminal went idle", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      const idle = { v: 100 };
      const props = { sessionId: "s1", providerId: "p1", getIdleMs: () => idle.v, aiCliRunning: true, redact: true };
      const { rerender } = render(<PromptSuggestions {...props} disabled />);
      toggleAuto();
      await advance(1000);
      idle.v = 60_000; await advance(1000); // 終端機轉閒置，但 Ask AI 還在跑
      expect(invokeAiChatCtx).not.toHaveBeenCalled();
      rerender(<PromptSuggestions {...props} disabled={false} />); // Ask AI 結束
      await advance(1000);
      expect(invokeAiChatCtx).toHaveBeenCalledTimes(1);
    });

    it("clears the waiting timer on unmount", async () => {
      invokeAiChatCtx.mockResolvedValue(reply(A));
      const idle = { v: 100 };
      const { unmount } = setup({ getIdleMs: () => idle.v });
      toggleAuto();
      await advance(1000);
      idle.v = 60_000; await advance(1000);
      screenText = "第二輪";
      idle.v = 100; await advance(1000);
      idle.v = 60_000; await advance(1000);
      unmount();
      await advance(AUTO_MIN_INTERVAL_MS + 1000);
      expect(invokeAiChatCtx).toHaveBeenCalledTimes(1);
    });
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
