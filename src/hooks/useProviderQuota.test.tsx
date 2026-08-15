import { renderHook, waitFor, act } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { useProviderQuota } from "./useProviderQuota";

const mockQuota = vi.fn();
vi.mock("../ipc/usage", async () => {
  const actual = await vi.importActual<typeof import("../ipc/usage")>("../ipc/usage");
  return { ...actual, usageQuota: (id: string, force?: boolean) => mockQuota(id, force) };
});

const okQuota = (windows: unknown[]) => ({
  status: "ok" as const,
  quota: { provider_id: "p", plan: null, fetched_at: 0, windows },
});

const win = (over: Record<string, unknown> = {}) => ({
  label: "5h", used_percent: 7, resets_at: null,
  severity: "normal", detail: null, is_primary: true, ...over,
});

describe("useProviderQuota", () => {
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mockQuota.mockReset();
    mockQuota.mockResolvedValue(okQuota([win()]));
  });
  afterEach(() => { vi.useRealTimers(); });

  it("用傳入的 provider id 查詢，回傳代表窗", async () => {
    const { result } = renderHook(() => useProviderQuota("anthropic-pro"));
    await waitFor(() => expect(mockQuota).toHaveBeenCalledWith("anthropic-pro", false));
    await waitFor(() => expect(result.current?.label).toBe("5h"));
  });

  it("severity 為 normal 時仍然回傳窗（常駐顯示，不是只有超標才出現）", async () => {
    const { result } = renderHook(() => useProviderQuota("p"));
    await waitFor(() => expect(result.current?.severity).toBe("normal"));
  });

  it("多窗時取最嚴重的，不是上游標記的代表窗", async () => {
    // 5h 剛重置 0%（上游標成代表窗），7d 已 96%。顯示綠色 0% 會誤導。
    mockQuota.mockResolvedValue(okQuota([
      win({ label: "5h", used_percent: 0, severity: "normal", is_primary: true }),
      win({ label: "7d", used_percent: 96, severity: "critical", is_primary: false }),
    ]));
    const { result } = renderHook(() => useProviderQuota("p"));
    await waitFor(() => expect(result.current?.label).toBe("7d"));
  });

  it("沒有配額概念的 provider 回 null", async () => {
    mockQuota.mockResolvedValue({ status: "not_applicable", provider_id: "ollama" });
    const { result } = renderHook(() => useProviderQuota("ollama"));
    await waitFor(() => expect(mockQuota).toHaveBeenCalled());
    expect(result.current).toBeNull();
  });

  it("查詢失敗回 null，不拋出", async () => {
    mockQuota.mockRejectedValue(new Error("boom"));
    const { result } = renderHook(() => useProviderQuota("p"));
    await waitFor(() => expect(mockQuota).toHaveBeenCalled());
    expect(result.current).toBeNull();
  });

  it("providerId 為空時完全不查", async () => {
    renderHook(() => useProviderQuota(""));
    await act(async () => { await Promise.resolve(); });
    expect(mockQuota).not.toHaveBeenCalled();
  });

  it("每 5 分鐘輪詢一次", async () => {
    renderHook(() => useProviderQuota("p"));
    await waitFor(() => expect(mockQuota).toHaveBeenCalledTimes(1));
    await vi.advanceTimersByTimeAsync(5 * 60 * 1000);
    await waitFor(() => expect(mockQuota).toHaveBeenCalledTimes(2));
  });

  it("視窗隱藏時跳過輪詢，回到前景時立即補查", async () => {
    const spy = vi.spyOn(document, "hidden", "get").mockReturnValue(true);
    renderHook(() => useProviderQuota("p"));
    await waitFor(() => expect(mockQuota).toHaveBeenCalledTimes(1));
    await vi.advanceTimersByTimeAsync(5 * 60 * 1000);
    expect(mockQuota).toHaveBeenCalledTimes(1);

    spy.mockReturnValue(false);
    document.dispatchEvent(new Event("visibilitychange"));
    await waitFor(() => expect(mockQuota).toHaveBeenCalledTimes(2));
    spy.mockRestore();
  });

  it("切換 provider 會用新 id 重查", async () => {
    const { rerender } = renderHook(({ id }) => useProviderQuota(id), {
      initialProps: { id: "a" },
    });
    await waitFor(() => expect(mockQuota).toHaveBeenCalledWith("a", false));
    rerender({ id: "b" });
    await waitFor(() => expect(mockQuota).toHaveBeenCalledWith("b", false));
  });
});
