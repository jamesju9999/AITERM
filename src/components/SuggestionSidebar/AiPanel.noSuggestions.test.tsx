import { describe, expect, it, vi, beforeEach } from "vitest";
import { act, render, screen } from "@testing-library/react";

const DEFAULT_CONFIG = {
  default_provider: null, providers: [], execution_mode: "graded",
  submit_shortcut: "enter", onboarding_done: true, max_agent_steps: 0,
  default_tab: "terminal", enterprise_server_url: null, enterprise_device_id: null, enterprise_policy: null,
};
const calls: { cmd: string; payload?: Record<string, unknown> }[] = [];
const listenMock = vi.fn().mockResolvedValue(() => {});
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, payload?: Record<string, unknown>) => {
    calls.push({ cmd, payload });
    if (cmd === "get_config") return Promise.resolve(DEFAULT_CONFIG);
    if (cmd === "get_mcp_tools") return Promise.resolve([]);
    if (cmd === "ai_chat_ctx") {
      return Promise.resolve({
        content: JSON.stringify([{ title: "補測試", prompt: "幫剛才的修改補單元測試" }]),
        tool_calls: [], tool_calling_unsupported: false,
      });
    }
    return Promise.resolve(null);
  },
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: (...a: unknown[]) => listenMock(...a) }));
vi.mock("../../lib/terminalInstanceRegistry", () => ({
  serializeTerminal: () => "❯ 修好登入的 bug\nClaude: 已修好",
  fillTerminalInput: vi.fn().mockResolvedValue(true),
  submitTerminalInput: vi.fn().mockResolvedValue(true),
}));
vi.mock("../../contexts/LocaleContext", async () => {
  const { translations } = await vi.importActual<typeof import("../../lib/i18n")>("../../lib/i18n");
  return { useLocale: () => ({ locale: "zh-TW" as const, t: translations["zh-TW"], setLocale: () => {} }) };
});

import { AiPanel } from "../AiPanel";

beforeEach(() => { calls.length = 0; listenMock.mockClear(); });

const mount = () =>
  render(
    <AiPanel
      sessionId="s1" isOpen providerName="x" onClose={vi.fn()} onExecuteCommand={vi.fn()}
      onOpenProviderPalette={vi.fn()} getIdleMs={() => 60_000}
    />,
  );

describe("AiPanel", () => {
  it("no longer hosts the suggestion bar (it lives in the docked sidebar now) and keeps the attachment control", async () => {
    const { container } = mount();
    await act(async () => {});
    expect(screen.queryByText("產生建議")).toBeNull();
    expect(container.querySelector(".aiterm-suggest")).toBeNull();
    expect(container.querySelector(".aiterm-pill-paperclip-btn")).not.toBeNull();
    expect(calls.some((c) => c.cmd === "ai_chat_ctx")).toBe(false);
  });
});
