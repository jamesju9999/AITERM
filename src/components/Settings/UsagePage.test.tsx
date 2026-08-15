import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { UsagePage } from "./UsagePage";

const mockQuotaAll = vi.fn();
vi.mock("../../ipc/usage", async () => {
  const actual = await vi.importActual<typeof import("../../ipc/usage")>("../../ipc/usage");
  return {
    ...actual,
    usageQuotaAll: (force?: boolean) => mockQuotaAll(force),
  };
});

describe("UsagePage 配額區塊", () => {
  beforeEach(() => {
    mockQuotaAll.mockReset();
  });

  it("每個訂閱型 provider 一張卡，顯示方案與各窗", async () => {
    mockQuotaAll.mockResolvedValue([
      { status: "ok", quota: { provider_id: "anthropic-pro", plan: "Claude Pro", fetched_at: 0,
        windows: [
          { label: "5h", used_percent: 7, resets_at: null, severity: "normal", detail: null, is_primary: true },
          { label: "7d", used_percent: 4, resets_at: null, severity: "normal", detail: null, is_primary: false },
        ] } },
    ]);
    render(<UsagePage />);
    expect(await screen.findByText("anthropic-pro")).toBeInTheDocument();
    expect(screen.getByText("Claude Pro")).toBeInTheDocument();
    expect(screen.getAllByTestId("quota-badge")).toHaveLength(2);
  });

  it("沒有配額概念的 provider 不列在配額區", async () => {
    mockQuotaAll.mockResolvedValue([{ status: "not_applicable", provider_id: "ollama-local" }]);
    render(<UsagePage />);
    await waitFor(() => expect(mockQuotaAll).toHaveBeenCalled());
    expect(screen.queryByText("ollama-local")).not.toBeInTheDocument();
  });

  it("查詢失敗的 provider 顯示錯誤訊息但不影響其他張卡", async () => {
    mockQuotaAll.mockResolvedValue([
      { status: "failed", provider_id: "GPT5.6", message: "AuthFailed" },
      { status: "ok", quota: { provider_id: "anthropic-pro", plan: null, fetched_at: 0,
        windows: [{ label: "5h", used_percent: 7, resets_at: null,
                    severity: "normal", detail: null, is_primary: true }] } },
    ]);
    render(<UsagePage />);
    expect(await screen.findByText("anthropic-pro")).toBeInTheDocument();
    expect(screen.getByTestId("quota-failed-GPT5.6")).toBeInTheDocument();
  });

  it("重新整理鈕會強制略過快取", async () => {
    mockQuotaAll.mockResolvedValue([]);
    render(<UsagePage />);
    await waitFor(() => expect(mockQuotaAll).toHaveBeenCalledWith(false));
    await userEvent.click(screen.getByTestId("quota-refresh"));
    await waitFor(() => expect(mockQuotaAll).toHaveBeenCalledWith(true));
  });
});
