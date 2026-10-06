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

const onClose = vi.fn();
const onNames = vi.fn();
beforeEach(() => { onClose.mockClear(); onNames.mockClear(); localStorage.clear(); });

const setup = (p: Partial<React.ComponentProps<typeof SuggestionSidebar>> = {}) =>
  render(
    <SuggestionSidebar
      sessionId="s1" disabled={false} aiCliRunning customNames={[]}
      onCustomNamesChange={onNames} onClose={onClose} getIdleMs={() => 60_000} {...p}
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
});
