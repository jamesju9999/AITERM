import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";

vi.mock("../../contexts/LocaleContext", async () => {
  const { translations } = await vi.importActual<typeof import("../../lib/i18n")>("../../lib/i18n");
  return {
    useLocale: () => ({ locale: "zh-TW" as const, t: translations["zh-TW"], setLocale: () => {} }),
  };
});

import { ModeHint } from "./ModeHint";

/**
 * 三種模式的差別是「誰按下執行鍵」，但兩顆開關表達不出來——使用者看不出
 * 都不亮時 AI 到底會不會自己跑指令。這一行就是把當下會發生什麼講白。
 */
describe("ModeHint", () => {
  it("都沒開時說明 AI 只會建議、要自己點才執行", () => {
    render(<ModeHint mode="suggest" maxAgentSteps={700} mcpToolCount={20} />);
    expect(screen.getByText(/建議/)).toBeInTheDocument();
    expect(screen.getByText(/▶/)).toBeInTheDocument();
  });

  // 步數是設定值，不是寫死的 5——使用者實際設的是 700。
  it("Agent 模式要帶出實際的步數上限", () => {
    render(<ModeHint mode="agent" maxAgentSteps={700} mcpToolCount={20} />);
    expect(screen.getByText(/700 步/)).toBeInTheDocument();
  });

  // 設定 0 = 無限，內部存成 9999，畫面照狀態列既有寫法顯示 ∞。
  it("步數無限時顯示 ∞ 而不是 9999", () => {
    render(<ModeHint mode="agent" maxAgentSteps={9999} mcpToolCount={20} />);
    expect(screen.getByText(/∞ 步/)).toBeInTheDocument();
    expect(screen.queryByText(/9999/)).not.toBeInTheDocument();
  });

  // 這是唯一會明講「MCP 此時被忽略」的地方；按鈕變灰只說了「不能點」。
  it("Agent 模式要明講此時不使用 MCP 工具", () => {
    render(<ModeHint mode="agent" maxAgentSteps={700} mcpToolCount={20} />);
    expect(screen.getByText(/不使用 MCP 工具/)).toBeInTheDocument();
  });

  it("MCP 模式要帶出實際的工具數量", () => {
    render(<ModeHint mode="mcp" maxAgentSteps={700} mcpToolCount={20} />);
    expect(screen.getByText(/20 個 MCP 工具/)).toBeInTheDocument();
  });
});
