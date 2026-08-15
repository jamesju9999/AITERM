import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { AiPanel } from "./index";

// Ask AI 面板不是用 ModelPickerButton，而是自己的 provider 按鈕。
// 這個測試存在的理由：Task 8 只改了 ModelPickerButton，終端機與這個面板
// ——使用者最常看的兩個地方——都沒有徽章，是實機才發現的。
const mockQuota = vi.fn();
vi.mock("../../ipc/usage", async () => {
  const actual = await vi.importActual<typeof import("../../ipc/usage")>("../../ipc/usage");
  return { ...actual, usageQuota: (id: string, force?: boolean) => mockQuota(id, force) };
});

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockResolvedValue({
    default_provider: null, providers: [], execution_mode: "graded",
    submit_shortcut: "enter", onboarding_done: true, max_agent_steps: 0,
    default_tab: "terminal", enterprise_server_url: null,
    enterprise_device_id: null, enterprise_policy: null,
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

describe("Ask AI 面板的配額徽章", () => {
  beforeEach(() => {
    mockQuota.mockReset();
    mockQuota.mockResolvedValue({
      status: "ok",
      quota: {
        provider_id: "anthropic-pro", plan: "Claude Pro", fetched_at: 0,
        windows: [{
          label: "5h", used_percent: 7, resets_at: null,
          severity: "normal", detail: null, is_primary: true,
        }],
      },
    });
  });

  it("用 providerId 查配額並顯示徽章", async () => {
    render(
      <AiPanel
        sessionId="s1"
        isOpen
        providerName="Anthropic-Sonnet-4.5"
        providerId="anthropic-pro"
        onClose={() => {}}
        onExecuteCommand={() => {}}
        onOpenProviderPalette={() => {}}
      />,
    );
    // 必須用 id，不是 display_name —— 後端是用 id 找 provider 設定的。
    await waitFor(() => expect(mockQuota).toHaveBeenCalledWith("anthropic-pro", false));
    expect(await screen.findByTestId("quota-badge")).toHaveTextContent("5h 7%");
  });

  it("沒有 providerId 時不查也不顯示", async () => {
    render(
      <AiPanel
        sessionId="s1"
        isOpen
        providerName="Ollama"
        onClose={() => {}}
        onExecuteCommand={() => {}}
        onOpenProviderPalette={() => {}}
      />,
    );
    await screen.findByText("Ollama");
    expect(mockQuota).not.toHaveBeenCalled();
    expect(screen.queryByTestId("quota-badge")).not.toBeInTheDocument();
  });
});
