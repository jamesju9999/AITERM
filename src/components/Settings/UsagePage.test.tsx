import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { UsagePage } from "./UsagePage";

const mockQuotaAll = vi.fn();
const mockSummary = vi.fn();
vi.mock("../../ipc/usage", async () => {
  const actual = await vi.importActual<typeof import("../../ipc/usage")>("../../ipc/usage");
  return {
    ...actual,
    usageQuotaAll: (force?: boolean) => mockQuotaAll(force),
    usageSummary: (range: string) => mockSummary(range),
  };
});

describe("UsagePage 配額區塊", () => {
  beforeEach(() => {
    mockQuotaAll.mockReset();
    mockSummary.mockReset();
    // 本地累計區塊在同一個元件裡掛載時也會發請求；配額測試不關心它，
    // 給個安全預設值避免未 mock 的 usageSummary() 回傳 undefined 炸掉 .then()。
    mockQuotaAll.mockResolvedValue([]);
    mockSummary.mockResolvedValue([]);
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

describe("UsagePage 本地累計區塊", () => {
  beforeEach(() => {
    mockQuotaAll.mockReset();
    mockSummary.mockReset();
    mockQuotaAll.mockResolvedValue([]);
    mockSummary.mockResolvedValue([]);
  });

  it("預設載入今天的統計", async () => {
    mockSummary.mockResolvedValue([]);
    render(<UsagePage />);
    await waitFor(() => expect(mockSummary).toHaveBeenCalledWith("today"));
  });

  it("顯示每個 provider/model 的 token 與成本", async () => {
    mockSummary.mockResolvedValue([{
      provider_id: "anthropic-pro", model: "claude-sonnet-4-5",
      requests: 12, prompt_tokens: 5000, completion_tokens: 1200,
      cache_read_tokens: 40000, cache_write_tokens: 800,
      estimated_cost_usd: 0.0435,
    }]);
    render(<UsagePage />);
    expect(await screen.findByText("claude-sonnet-4-5")).toBeInTheDocument();
    expect(screen.getByText("anthropic-pro")).toBeInTheDocument();
    expect(screen.getByText("$0.0435")).toBeInTheDocument();
  });

  it("查不到單價時顯示破折號而不是 $0", async () => {
    mockSummary.mockResolvedValue([{
      provider_id: "local", model: "qwen3.6-27b",
      requests: 3, prompt_tokens: 100, completion_tokens: 50,
      cache_read_tokens: 0, cache_write_tokens: 0,
      estimated_cost_usd: null,
    }]);
    render(<UsagePage />);
    await screen.findByText("qwen3.6-27b");
    expect(screen.queryByText("$0")).not.toBeInTheDocument();
    expect(screen.getByTestId("cost-local-qwen3.6-27b")).toHaveTextContent("—");
  });

  it("顯示快取命中率", async () => {
    mockSummary.mockResolvedValue([{
      provider_id: "anthropic-pro", model: "claude-sonnet-4-5",
      requests: 1, prompt_tokens: 1000, completion_tokens: 10,
      cache_read_tokens: 9000, cache_write_tokens: 0,
      estimated_cost_usd: 0.01,
    }]);
    render(<UsagePage />);
    // 9000 / (1000 + 9000) = 90%
    expect(await screen.findByText("90%")).toBeInTheDocument();
  });

  it("切換區間會重新查詢", async () => {
    mockSummary.mockResolvedValue([]);
    render(<UsagePage />);
    await waitFor(() => expect(mockSummary).toHaveBeenCalledWith("today"));
    await userEvent.click(screen.getByRole("button", { name: /7/ }));
    await waitFor(() => expect(mockSummary).toHaveBeenCalledWith("days7"));
  });

  it("沒有資料時顯示空狀態", async () => {
    mockSummary.mockResolvedValue([]);
    render(<UsagePage />);
    expect(await screen.findByTestId("usage-empty")).toBeInTheDocument();
  });
});
