import { describe, expect, it, vi, beforeEach } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";

const invokeAiChatCtx = vi.fn();
const abortAi = vi.fn().mockResolvedValue(undefined);
vi.mock("../../ipc/ai", () => ({
  invokeAiChatCtx: (...a: unknown[]) => invokeAiChatCtx(...a),
  abortAi: (...a: unknown[]) => abortAi(...a),
  formatAiError: () => "err",
}));
vi.mock("../../lib/terminalInstanceRegistry", () => ({
  serializeTerminal: () => "screen",
  fillTerminalInput: vi.fn().mockResolvedValue(true),
  submitTerminalInput: vi.fn().mockResolvedValue(true),
}));
vi.mock("../../contexts/LocaleContext", async () => {
  const { translations } = await vi.importActual<typeof import("../../lib/i18n")>("../../lib/i18n");
  return { useLocale: () => ({ locale: "zh-TW" as const, t: translations["zh-TW"], setLocale: () => {} }) };
});

import { SuggestionSidebar } from "./SuggestionSidebar";
import { MAX_GOAL_CHARS } from "../../lib/promptSuggestions";

const onClose = vi.fn();
const onNames = vi.fn();
const onGoal = vi.fn();
const onMilestones = vi.fn();
beforeEach(() => { invokeAiChatCtx.mockReset(); abortAi.mockClear(); onClose.mockClear(); onNames.mockClear(); onGoal.mockClear(); onMilestones.mockClear(); localStorage.clear(); });

const setup = (p: Partial<React.ComponentProps<typeof SuggestionSidebar>> = {}) =>
  render(
    <SuggestionSidebar
      sessionId="s1" disabled={false} aiCliRunning customNames={[]}
      onCustomNamesChange={onNames} onClose={onClose} goal="" onGoalChange={onGoal} milestones={undefined} onMilestonesChange={onMilestones} getIdleMs={() => 60_000} {...p}
    />,
  );

describe("SuggestionSidebar", () => {
  it("shows whether an AI tool was detected or the sidebar was opened manually", () => {
    const a = setup({ aiCliRunning: true });
    expect(screen.getByText("偵測到 AI 工具執行中")).toBeTruthy();
    a.unmount();
    setup({ aiCliRunning: false });
    expect(screen.getByText("未偵測到 AI 工具（手動開啟）")).toBeTruthy();
  });

  it("hosts the suggestion generator with a single title (no duplicate collapsible header)", () => {
    setup();
    expect(screen.getByText("產生建議")).toBeTruthy();
    expect(screen.getAllByText("下一步建議")).toHaveLength(1);
  });

  it("the close button calls onClose", () => {
    setup();
    fireEvent.click(screen.getByRole("button", { name: "關閉建議側欄" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  describe("custom AI tool list", () => {
    const openMenu = () => fireEvent.click(screen.getByRole("button", { name: "自訂 AI 工具" }));

    it("is hidden until the menu is opened, then lists built-in and custom names", () => {
      setup({ customNames: ["mytool"] });
      expect(screen.queryByText("指令名稱，例如 mytool")).toBeNull();
      openMenu();
      expect(screen.getByText("claude")).toBeTruthy();
      expect(screen.getByText("codex")).toBeTruthy();
      expect(screen.getByText("mytool")).toBeTruthy();
    });

    it("adds a valid, normalised name", () => {
      setup({ customNames: ["old"] });
      openMenu();
      fireEvent.change(screen.getByPlaceholderText("指令名稱，例如 mytool"), { target: { value: "  MyTool.exe " } });
      fireEvent.click(screen.getByRole("button", { name: "新增" }));
      expect(onNames).toHaveBeenCalledWith(["old", "mytool"]);
    });

    it("rejects names with spaces or path separators and does not save", () => {
      setup();
      openMenu();
      const input = screen.getByPlaceholderText("指令名稱，例如 mytool");
      fireEvent.change(input, { target: { value: "my tool" } });
      fireEvent.click(screen.getByRole("button", { name: "新增" }));
      expect(screen.getByText("名稱不能是空的，也不能含空白或路徑符號")).toBeTruthy();
      fireEvent.change(input, { target: { value: "bin/tool" } });
      fireEvent.click(screen.getByRole("button", { name: "新增" }));
      expect(onNames).not.toHaveBeenCalled();
    });

    it("rejects a name that is built in or already added", () => {
      setup({ customNames: ["mytool"] });
      openMenu();
      const input = screen.getByPlaceholderText("指令名稱，例如 mytool");
      fireEvent.change(input, { target: { value: "Claude" } });
      fireEvent.click(screen.getByRole("button", { name: "新增" }));
      expect(screen.getByText("已經在清單裡了")).toBeTruthy();
      fireEvent.change(input, { target: { value: "mytool" } });
      fireEvent.click(screen.getByRole("button", { name: "新增" }));
      expect(onNames).not.toHaveBeenCalled();
    });

    it("removes a custom name, but built-in names have no remove button", () => {
      setup({ customNames: ["mytool", "other"] });
      openMenu();
      fireEvent.click(screen.getByRole("button", { name: "移除 mytool" }));
      expect(onNames).toHaveBeenCalledWith(["other"]);
      expect(screen.queryByRole("button", { name: "移除 claude" })).toBeNull();
    });
  });

  describe("goal", () => {
    const box = () => screen.getByRole("textbox", { name: "大目標" }) as HTMLTextAreaElement;

    it("invites the user to set a goal when there is none, and saves what they type", () => {
      setup();
      expect(screen.queryByRole("textbox", { name: "大目標" })).toBeNull();
      fireEvent.click(screen.getByRole("button", { name: "設定大目標（選填）" }));
      fireEvent.change(box(), { target: { value: "  將舊程式轉成網頁架構  " } });
      fireEvent.click(screen.getByRole("button", { name: "儲存" }));
      expect(onGoal).toHaveBeenCalledWith("將舊程式轉成網頁架構");
    });

    it("shows the current goal as text and lets the user edit it", () => {
      setup({ goal: "把舊系統轉成網頁版" });
      expect(screen.getByText("把舊系統轉成網頁版")).toBeTruthy();
      fireEvent.click(screen.getByRole("button", { name: "編輯大目標" }));
      expect(box().value).toBe("把舊系統轉成網頁版");
      fireEvent.change(box(), { target: { value: "改成雲端版" } });
      fireEvent.click(screen.getByRole("button", { name: "儲存" }));
      expect(onGoal).toHaveBeenCalledWith("改成雲端版");
    });

    it("clearing, or saving a blank box, removes the goal", () => {
      setup({ goal: "目標" });
      fireEvent.click(screen.getByRole("button", { name: "編輯大目標" }));
      fireEvent.click(screen.getByRole("button", { name: "清除" }));
      expect(onGoal).toHaveBeenLastCalledWith("");
      fireEvent.click(screen.getByRole("button", { name: "編輯大目標" }));
      fireEvent.change(box(), { target: { value: "   " } });
      fireEvent.click(screen.getByRole("button", { name: "儲存" }));
      expect(onGoal).toHaveBeenLastCalledWith("");
    });

    it("cancel leaves the goal untouched", () => {
      setup({ goal: "目標" });
      fireEvent.click(screen.getByRole("button", { name: "編輯大目標" }));
      fireEvent.change(box(), { target: { value: "亂改的" } });
      fireEvent.click(screen.getByRole("button", { name: "取消" }));
      expect(onGoal).not.toHaveBeenCalled();
      expect(screen.getByText("目標")).toBeTruthy();
    });

    it("limits the length of the goal", () => {
      setup();
      fireEvent.click(screen.getByRole("button", { name: "設定大目標（選填）" }));
      expect(box().maxLength).toBe(MAX_GOAL_CHARS);
    });
  });

  describe("AI polish of the goal", () => {
    const box = () => screen.getByRole("textbox", { name: "大目標" }) as HTMLTextAreaElement;
    const open = (goal = "") => {
      setup({ goal });
      fireEvent.click(screen.getByRole("button", { name: goal ? "編輯大目標" : "設定大目標（選填）" }));
    };
    const reply = (content: string | null) => ({ content, tool_calls: [], tool_calling_unsupported: false });
    const polishBtn = () => screen.getByRole("button", { name: /AI 潤飾|潤飾中/ }) as HTMLButtonElement;

    it("is disabled while the draft is blank", () => {
      open();
      expect(polishBtn().disabled).toBe(true);
      fireEvent.change(box(), { target: { value: "轉成網頁版" } });
      expect(polishBtn().disabled).toBe(false);
    });

    it("replaces the draft with the polished text on a separate stream id, without saving it", async () => {
      invokeAiChatCtx.mockResolvedValue(reply('```\n將舊程式的 Client-Server 架構轉換為網頁平台架構\n```'));
      open();
      fireEvent.change(box(), { target: { value: "轉成網頁版" } });
      await act(async () => { fireEvent.click(polishBtn()); });
      expect(invokeAiChatCtx).toHaveBeenCalledTimes(1);
      const [messages, , connId, providerId] = invokeAiChatCtx.mock.calls[0];
      expect(connId).toBe("goal-polish-s1");
      expect(providerId).toBeUndefined();
      expect(messages[0].content).toContain("轉成網頁版");
      expect(box().value).toBe("將舊程式的 Client-Server 架構轉換為網頁平台架構");
      expect(onGoal).not.toHaveBeenCalled();
    });

    it("shows a busy state while waiting and blocks a second request", async () => {
      invokeAiChatCtx.mockImplementation(() => new Promise(() => {}));
      open();
      fireEvent.change(box(), { target: { value: "轉成網頁版" } });
      await act(async () => { fireEvent.click(polishBtn()); });
      expect(polishBtn().textContent).toContain("潤飾中");
      expect(polishBtn().disabled).toBe(true);
      expect(invokeAiChatCtx).toHaveBeenCalledTimes(1);
    });

    it("can undo back to exactly what the user had typed", async () => {
      invokeAiChatCtx.mockResolvedValue(reply("潤飾後的目標"));
      open();
      fireEvent.change(box(), { target: { value: "我寫的原稿" } });
      await act(async () => { fireEvent.click(polishBtn()); });
      expect(box().value).toBe("潤飾後的目標");
      fireEvent.click(screen.getByRole("button", { name: "還原" }));
      expect(box().value).toBe("我寫的原稿");
      expect(screen.queryByRole("button", { name: "還原" })).toBeNull();
    });

    it("hides the undo button as soon as the user edits the polished text", async () => {
      invokeAiChatCtx.mockResolvedValue(reply("潤飾後的目標"));
      open();
      fireEvent.change(box(), { target: { value: "原稿" } });
      await act(async () => { fireEvent.click(polishBtn()); });
      fireEvent.change(box(), { target: { value: "潤飾後的目標，再自己改幾個字" } });
      expect(screen.queryByRole("button", { name: "還原" })).toBeNull();
    });

    it("saving after a polish stores the polished text", async () => {
      invokeAiChatCtx.mockResolvedValue(reply("潤飾後的目標"));
      open();
      fireEvent.change(box(), { target: { value: "原稿" } });
      await act(async () => { fireEvent.click(polishBtn()); });
      fireEvent.click(screen.getByRole("button", { name: "儲存" }));
      expect(onGoal).toHaveBeenCalledWith("潤飾後的目標");
    });

    it("keeps the draft and shows a message when the AI fails or returns nothing", async () => {
      invokeAiChatCtx.mockRejectedValueOnce({ kind: "network", message: "x" });
      open();
      fireEvent.change(box(), { target: { value: "原稿" } });
      await act(async () => { fireEvent.click(polishBtn()); });
      expect(screen.getByRole("alert").textContent).toContain("潤飾失敗");
      expect(box().value).toBe("原稿");

      invokeAiChatCtx.mockResolvedValueOnce(reply("   "));
      await act(async () => { fireEvent.click(polishBtn()); });
      expect(screen.getByRole("alert").textContent).toContain("沒有回傳可用的內容");
      expect(box().value).toBe("原稿");
    });

    it("two clicks in the same tick still send only one request", async () => {
      invokeAiChatCtx.mockImplementation(() => new Promise(() => {}));
      open();
      fireEvent.change(box(), { target: { value: "轉成網頁版" } });
      await act(async () => { polishBtn().click(); polishBtn().click(); });
      expect(invokeAiChatCtx).toHaveBeenCalledTimes(1);
    });

    it("a late reply from a cancelled request cannot overwrite a newer one", async () => {
      const resolvers: ((v: unknown) => void)[] = [];
      invokeAiChatCtx.mockImplementation(() => new Promise((r) => { resolvers.push(r); }));
      open();
      fireEvent.change(box(), { target: { value: "原稿" } });
      await act(async () => { fireEvent.click(polishBtn()); });
      fireEvent.click(screen.getByRole("button", { name: "取消" }));
      fireEvent.click(screen.getByRole("button", { name: "設定大目標（選填）" }));
      fireEvent.change(box(), { target: { value: "第二份原稿" } });
      await act(async () => { fireEvent.click(polishBtn()); });
      await act(async () => { resolvers[1](reply("第二次的潤飾結果")); });
      expect(box().value).toBe("第二次的潤飾結果");
      await act(async () => { resolvers[0](reply("遲到的第一次結果")); });
      expect(box().value).toBe("第二次的潤飾結果");
    });

    it("cancelling while waiting aborts the request and ignores the late reply", async () => {
      let resolveIt!: (v: unknown) => void;
      invokeAiChatCtx.mockImplementation(() => new Promise((r) => { resolveIt = r; }));
      open("舊目標");
      fireEvent.change(box(), { target: { value: "舊目標加一點" } });
      await act(async () => { fireEvent.click(polishBtn()); });
      fireEvent.click(screen.getByRole("button", { name: "取消" }));
      expect(abortAi).toHaveBeenCalledWith("goal-polish-s1");
      await act(async () => { resolveIt(reply("遲到的潤飾結果")); });
      expect(screen.queryByText("遲到的潤飾結果")).toBeNull();
      // 再開編輯框，內容是已儲存的目標，不是遲到的結果。
      fireEvent.click(screen.getByRole("button", { name: "編輯大目標" }));
      expect(box().value).toBe("舊目標");
    });
  });

  describe("milestones", () => {
    const state = { forGoal: "目標", items: [{ id: "a", text: "盤點 API", done: false }] };

    it("is hidden until there is a goal or existing milestones", () => {
      setup({ goal: "" });
      expect(screen.queryByRole("region", { name: "里程碑" })).toBeNull();
    });

    it("shows once a goal is set", () => {
      setup({ goal: "目標" });
      expect(screen.getByRole("region", { name: "里程碑" })).toBeTruthy();
    });

    it("keeps showing existing milestones even if the goal was cleared", () => {
      setup({ goal: "", milestones: state });
      expect(screen.getByText("盤點 API")).toBeTruthy();
    });

    it("passes edits up", () => {
      setup({ goal: "目標", milestones: state });
      fireEvent.click(screen.getByRole("checkbox", { name: "完成：盤點 API" }));
      expect(onMilestones).toHaveBeenCalledTimes(1);
      expect(onMilestones.mock.calls[0][0].items[0].done).toBe(true);
    });
  });
});
