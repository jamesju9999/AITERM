import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ModelPickerButton } from "./ModelPickerButton";
import type { ProviderInfo } from "../ipc/provider";

const mockQuota = vi.fn();
const mockQuotaAll = vi.fn();
vi.mock("../ipc/usage", async () => {
  const actual = await vi.importActual<typeof import("../ipc/usage")>("../ipc/usage");
  return {
    ...actual,
    usageQuota: (id: string, force?: boolean) => mockQuota(id, force),
    usageQuotaAll: (force?: boolean) => mockQuotaAll(force),
  };
});

const providers: ProviderInfo[] = [
  { id: "anthropic-pro", display_name: "Anthropic", provider_type: "anthropic",
    base_url: null, oauth_client_id: null, model: "claude-sonnet-4-5",
    supports_json_mode: true, has_api_key: false, is_default: true, auth_method: "oauth" },
  { id: "ollama-local", display_name: "Ollama", provider_type: "ollama",
    base_url: null, oauth_client_id: null, model: "llama3",
    supports_json_mode: true, has_api_key: false, is_default: false, auth_method: null },
];

const okQuota = (pct: number, sev: "normal" | "warning" | "critical" = "normal") => ({
  status: "ok" as const,
  quota: {
    provider_id: "anthropic-pro", plan: "Claude Pro", fetched_at: 0,
    windows: [{ label: "5h", used_percent: pct, resets_at: null,
                severity: sev, detail: null, is_primary: true }],
  },
});

describe("ModelPickerButton 配額徽章", () => {
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mockQuota.mockReset();
    mockQuotaAll.mockReset();
    mockQuota.mockResolvedValue(okQuota(7));
    mockQuotaAll.mockResolvedValue([okQuota(7)]);
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("掛載時只查選中的那一個，不查全部", async () => {
    render(<ModelPickerButton providers={providers} selectedId="anthropic-pro" onChange={() => {}} />);
    await waitFor(() => expect(mockQuota).toHaveBeenCalledWith("anthropic-pro", false));
    expect(mockQuotaAll).not.toHaveBeenCalled();
  });

  it("severity 為 normal 時仍然顯示徽章", async () => {
    render(<ModelPickerButton providers={providers} selectedId="anthropic-pro" onChange={() => {}} />);
    expect(await screen.findByTestId("quota-badge")).toHaveTextContent("5h 7%");
  });

  it("下拉展開時才查全部", async () => {
    render(<ModelPickerButton providers={providers} selectedId="anthropic-pro" onChange={() => {}} />);
    await waitFor(() => expect(mockQuota).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("button"));
    await waitFor(() => expect(mockQuotaAll).toHaveBeenCalledWith(false));
  });

  it("切換 provider 會立即重查新選中者", async () => {
    const { rerender } = render(
      <ModelPickerButton providers={providers} selectedId="anthropic-pro" onChange={() => {}} />
    );
    await waitFor(() => expect(mockQuota).toHaveBeenCalledWith("anthropic-pro", false));
    rerender(<ModelPickerButton providers={providers} selectedId="ollama-local" onChange={() => {}} />);
    await waitFor(() => expect(mockQuota).toHaveBeenCalledWith("ollama-local", false));
  });

  it("多窗時徽章顯示最嚴重的那個，不是上游標記的代表窗", async () => {
    // 5h 剛重置 0%（上游標成代表窗），7d 已 96%。顯示綠色 0% 會誤導。
    mockQuota.mockResolvedValue({
      status: "ok",
      quota: {
        provider_id: "anthropic-pro", plan: "Claude Pro", fetched_at: 0,
        windows: [
          { label: "5h", used_percent: 0, resets_at: null,
            severity: "normal", detail: null, is_primary: true },
          { label: "7d", used_percent: 96, resets_at: null,
            severity: "critical", detail: null, is_primary: false },
        ],
      },
    });
    render(<ModelPickerButton providers={providers} selectedId="anthropic-pro" onChange={() => {}} />);
    const badge = await screen.findByTestId("quota-badge");
    expect(badge).toHaveTextContent("7d 96%");
    expect(badge.className).toContain("critical");
  });

  it("查詢失敗時按鈕仍可點開", async () => {
    mockQuota.mockResolvedValue({ status: "failed", provider_id: "anthropic-pro", message: "boom" });
    render(<ModelPickerButton providers={providers} selectedId="anthropic-pro" onChange={() => {}} />);
    await waitFor(() => expect(mockQuota).toHaveBeenCalled());
    expect(screen.queryByTestId("quota-badge")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button"));
    expect(screen.getByText("Ollama")).toBeInTheDocument();
  });

  it("沒有配額概念的 provider 不顯示徽章", async () => {
    mockQuota.mockResolvedValue({ status: "not_applicable", provider_id: "ollama-local" });
    render(<ModelPickerButton providers={providers} selectedId="ollama-local" onChange={() => {}} />);
    await waitFor(() => expect(mockQuota).toHaveBeenCalled());
    expect(screen.queryByTestId("quota-badge")).not.toBeInTheDocument();
  });

  it("每 5 分鐘輪詢一次", async () => {
    render(<ModelPickerButton providers={providers} selectedId="anthropic-pro" onChange={() => {}} />);
    await waitFor(() => expect(mockQuota).toHaveBeenCalledTimes(1));
    await vi.advanceTimersByTimeAsync(5 * 60 * 1000);
    await waitFor(() => expect(mockQuota).toHaveBeenCalledTimes(2));
  });
});
