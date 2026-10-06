import { describe, expect, it, vi, beforeEach } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

vi.mock("../../ipc/ai", () => ({
  invokeAiChatCtx: vi.fn(),
  abortAi: vi.fn().mockResolvedValue(undefined),
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
beforeEach(() => { onClose.mockClear(); onNames.mockClear(); onGoal.mockClear(); localStorage.clear(); });

const setup = (p: Partial<React.ComponentProps<typeof SuggestionSidebar>> = {}) =>
  render(
    <SuggestionSidebar
      sessionId="s1" disabled={false} aiCliRunning customNames={[]}
      onCustomNamesChange={onNames} onClose={onClose} goal="" onGoalChange={onGoal} getIdleMs={() => 60_000} {...p}
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
});
