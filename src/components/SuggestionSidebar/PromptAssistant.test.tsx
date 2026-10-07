import { describe, expect, it, vi, beforeEach } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";

const invokeAiComplete = vi.fn();
const abortAi = vi.fn().mockResolvedValue(undefined);
vi.mock("../../ipc/ai", () => ({
  invokeAiComplete: (...a: unknown[]) => invokeAiComplete(...a),
  abortAi: (...a: unknown[]) => abortAi(...a),
  formatAiError: () => "ERR",
}));
let screenText: string | null = "目前畫面 CURRENT-SCREEN";
const callOrder: string[] = [];
const fillTerminalInput = vi.fn(async () => { callOrder.push("fill"); return true; });
const submitTerminalInput = vi.fn(async () => { callOrder.push("submit"); return true; });
vi.mock("../../lib/terminalInstanceRegistry", () => ({
  serializeTerminal: () => screenText,
  fillTerminalInput: (...a: unknown[]) => (fillTerminalInput as unknown as (...x: unknown[]) => Promise<boolean>)(...a),
  submitTerminalInput: (...a: unknown[]) => (submitTerminalInput as unknown as (...x: unknown[]) => Promise<boolean>)(...a),
}));
vi.mock("../../contexts/LocaleContext", async () => {
  const { translations } = await vi.importActual<typeof import("../../lib/i18n")>("../../lib/i18n");
  return { useLocale: () => ({ locale: "zh-TW" as const, t: translations["zh-TW"], setLocale: () => {} }) };
});

import { PromptAssistant } from "./PromptAssistant";
import { MAX_ASSIST_REQUEST_CHARS, PLAIN_COMPLETION_SYSTEM_PROMPT } from "../../lib/promptSuggestions";

const reply = (content: string | null) => ({ content, tool_calls: [], tool_calling_unsupported: false });
const TOKEN = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";

const setup = (p: Partial<React.ComponentProps<typeof PromptAssistant>> = {}) =>
  render(<PromptAssistant sessionId="s1" providerId="p1" goal="" milestoneContext="" redact {...p} />);
const need = () => screen.getByRole("textbox", { name: "你的需求" }) as HTMLTextAreaElement;
const result = () => screen.getByRole("textbox", { name: "改寫後的提示詞" }) as HTMLTextAreaElement;
const runBtn = () => screen.getByRole("button", { name: /AI 改寫成提示詞|改寫中/ }) as HTMLButtonElement;
const run = async (text = "幫我把登入改成 REST") => {
  fireEvent.change(need(), { target: { value: text } });
  await act(async () => { fireEvent.click(runBtn()); });
};
const sent = () => invokeAiComplete.mock.calls[0][0][0].content as string;

beforeEach(() => {
  invokeAiComplete.mockReset();
  abortAi.mockClear();
  fillTerminalInput.mockClear();
  submitTerminalInput.mockClear();
  callOrder.length = 0;
  screenText = "目前畫面 CURRENT-SCREEN";
});

describe("PromptAssistant", () => {
  it("is always visible (no toggle to open), and cannot run on a blank request", () => {
    setup();
    expect(screen.queryByRole("button", { name: /^提示詞助手$/ })).toBeNull();
    expect(need()).toBeTruthy();
    expect(runBtn().disabled).toBe(true);
    fireEvent.change(need(), { target: { value: "   " } });
    expect(runBtn().disabled).toBe(true);
    fireEvent.change(need(), { target: { value: "做某件事" } });
    expect(runBtn().disabled).toBe(false);
    expect(need().maxLength).toBe(MAX_ASSIST_REQUEST_CHARS);
  });

  it("shows the result above the input row, like a chat composer", async () => {
    invokeAiComplete.mockResolvedValue(reply("改寫的提示詞"));
    setup();
    await run();
    expect(result().compareDocumentPosition(need()) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("sends the request with goal, milestones and the screen on its own stream id, and shows the result without touching the terminal", async () => {
    invokeAiComplete.mockResolvedValue(reply("請把登入流程改成 REST API，保留現有驗證。"));
    setup({ goal: "網頁化", milestoneContext: "里程碑一節-MARK" });
    await run();
    expect(invokeAiComplete).toHaveBeenCalledTimes(1);
    const [, , connId, providerId] = invokeAiComplete.mock.calls[0];
    expect(connId).toBe("prompt-assist-s1");
    expect(invokeAiComplete.mock.calls[0][1]).toBe(PLAIN_COMPLETION_SYSTEM_PROMPT);
    expect(providerId).toBe("p1");
    expect(sent()).toContain("幫我把登入改成 REST");
    expect(sent()).toContain("網頁化");
    expect(sent()).toContain("里程碑一節-MARK");
    expect(sent()).toContain("CURRENT-SCREEN");
    expect(result().value).toBe("請把登入流程改成 REST API，保留現有驗證。");
    expect(fillTerminalInput).not.toHaveBeenCalled();
    expect(submitTerminalInput).not.toHaveBeenCalled();
  });

  it("cleans a fenced or quoted reply", async () => {
    invokeAiComplete.mockResolvedValue(reply("```\n請重構登入\n```"));
    setup();
    await run();
    expect(result().value).toBe("請重構登入");
  });

  it("leaves the screen out when the terminal has nothing readable", async () => {
    screenText = "  \n ";
    invokeAiComplete.mockResolvedValue(reply("好"));
    setup();
    await run();
    expect(sent()).not.toContain("終端機畫面");
  });

  it("fills what is in the box (including the user's edits) and never presses Enter on 填入", async () => {
    invokeAiComplete.mockResolvedValue(reply("改寫的提示詞"));
    setup();
    await run();
    fireEvent.change(result(), { target: { value: "改寫的提示詞，再自己加一句" } });
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "填入" })); });
    expect(fillTerminalInput).toHaveBeenCalledWith("s1", "改寫的提示詞，再自己加一句");
    expect(submitTerminalInput).not.toHaveBeenCalled();
  });

  it("填入並送出 fills first and then presses Enter", async () => {
    invokeAiComplete.mockResolvedValue(reply("改寫的提示詞"));
    setup();
    await run();
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "填入並送出" })); });
    expect(callOrder).toEqual(["fill", "submit"]);
    expect(submitTerminalInput).toHaveBeenCalledWith("s1");
  });

  it("does not submit when filling failed (terminal gone)", async () => {
    invokeAiComplete.mockResolvedValue(reply("改寫的提示詞"));
    fillTerminalInput.mockResolvedValueOnce(false as never);
    setup();
    await run();
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "填入並送出" })); });
    expect(submitTerminalInput).not.toHaveBeenCalled();
  });

  it("does not fill a blank box", async () => {
    invokeAiComplete.mockResolvedValue(reply("改寫的提示詞"));
    setup();
    await run();
    fireEvent.change(result(), { target: { value: "  " } });
    expect((screen.getByRole("button", { name: "填入" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "填入並送出" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("rewrite again re-runs with the same request; clear empties everything", async () => {
    invokeAiComplete.mockResolvedValueOnce(reply("第一版")).mockResolvedValueOnce(reply("第二版"));
    setup();
    await run("同一個需求");
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "重新改寫" })); });
    expect(invokeAiComplete).toHaveBeenCalledTimes(2);
    expect(invokeAiComplete.mock.calls[1][0][0].content).toContain("同一個需求");
    expect(result().value).toBe("第二版");
    fireEvent.click(screen.getByRole("button", { name: "清除" }));
    expect(need().value).toBe("");
    expect(screen.queryByRole("textbox", { name: "改寫後的提示詞" })).toBeNull();
  });

  it("shows a busy label and blocks a second request, even for two clicks in one tick", async () => {
    invokeAiComplete.mockImplementation(() => new Promise(() => {}));
    setup();
    fireEvent.change(need(), { target: { value: "需求" } });
    await act(async () => { runBtn().click(); runBtn().click(); });
    expect(invokeAiComplete).toHaveBeenCalledTimes(1);
    expect(runBtn().textContent).toContain("改寫中");
    expect(runBtn().disabled).toBe(true);
  });

  it("shows an error and keeps the request when the AI fails or returns nothing", async () => {
    invokeAiComplete.mockRejectedValueOnce({ kind: "network", message: "x" });
    setup();
    await run("保留的需求");
    expect(screen.getByRole("alert").textContent).toContain("改寫失敗");
    expect(need().value).toBe("保留的需求");
    invokeAiComplete.mockResolvedValueOnce(reply("   "));
    await act(async () => { fireEvent.click(runBtn()); });
    expect(screen.getByRole("alert").textContent).toContain("沒有回傳可用的內容");
    expect(screen.queryByRole("textbox", { name: "改寫後的提示詞" })).toBeNull();
  });

  it("clearing while waiting aborts the request and ignores the late reply", async () => {
    let resolveIt!: (v: unknown) => void;
    invokeAiComplete.mockImplementation(() => new Promise((r) => { resolveIt = r; }));
    setup();
    fireEvent.change(need(), { target: { value: "需求" } });
    await act(async () => { fireEvent.click(runBtn()); });
    fireEvent.click(screen.getByRole("button", { name: "清除" }));
    expect(abortAi).toHaveBeenCalledWith("prompt-assist-s1");
    await act(async () => { resolveIt(reply("遲到的結果")); });
    expect(screen.queryByText("遲到的結果")).toBeNull();
    expect(screen.queryByRole("textbox", { name: "改寫後的提示詞" })).toBeNull();
    expect(runBtn().disabled).toBe(true); // 需求也清空了
  });

  it("aborts an in-flight request when unmounted", async () => {
    invokeAiComplete.mockImplementation(() => new Promise(() => {}));
    const { unmount } = setup();
    fireEvent.change(need(), { target: { value: "需求" } });
    await act(async () => { fireEvent.click(runBtn()); });
    unmount();
    expect(abortAi).toHaveBeenCalledWith("prompt-assist-s1");
  });

  describe("secret redaction", () => {
    it("masks the screen before sending (and before cutting it to size), and says how many", async () => {
      invokeAiComplete.mockResolvedValue(reply("好"));
      const suffix = "y ".repeat(2000).slice(0, 4000 - 20);
      screenText = `${"x ".repeat(500)}${TOKEN}${suffix}\npassword=hunter2hunter2`;
      setup();
      await run();
      expect(sent()).not.toContain(TOKEN.slice(-20));
      expect(sent()).not.toContain("hunter2hunter2");
      expect(screen.getByText("已在送出前遮罩 2 處疑似敏感資訊")).toBeTruthy();
    });

    it("sends the raw screen and shows no count when redaction is off", async () => {
      invokeAiComplete.mockResolvedValue(reply("好"));
      screenText = `export GITHUB_TOKEN=${TOKEN}`;
      setup({ redact: false });
      await run();
      expect(sent()).toContain(TOKEN);
      expect(screen.queryByText(/已在送出前遮罩/)).toBeNull();
    });
  });

  it("does not mask what the user typed themselves", async () => {
    invokeAiComplete.mockResolvedValue(reply("好"));
    setup();
    await run(`請用 password=hunter2hunter2 登入測試機`);
    expect(sent()).toContain("hunter2hunter2");
  });
});
