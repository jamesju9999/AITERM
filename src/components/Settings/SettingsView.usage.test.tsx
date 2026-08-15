import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { MemoryRouter } from "react-router-dom";
import { SettingsView } from "./SettingsView";

// UsagePage 掛載時會查配額；這裡只驗「分頁到得了」，IPC 一律擋掉。
const mockQuotaAll = vi.fn();
vi.mock("../../ipc/usage", async () => {
  const actual = await vi.importActual<typeof import("../../ipc/usage")>("../../ipc/usage");
  return {
    ...actual,
    usageQuotaAll: (force?: boolean) => mockQuotaAll(force),
  };
});

// 預設分頁（GeneralPage）掛載時也會打 IPC。不擋的話會噴 unhandled rejection，
// Vitest 會警告「可能造成假陽性」—— 那會蓋掉這個測試真正想驗的東西。
vi.mock("../../ipc/config", async () => {
  const actual = await vi.importActual<typeof import("../../ipc/config")>("../../ipc/config");
  return { ...actual, getConfig: vi.fn().mockResolvedValue({ providers: [] }) };
});
vi.mock("../../ipc/telegram", async () => {
  const actual = await vi.importActual<typeof import("../../ipc/telegram")>("../../ipc/telegram");
  return { ...actual, getTelegramConfig: vi.fn().mockResolvedValue({}) };
});

describe("SettingsView 的用量分頁", () => {
  beforeEach(() => {
    mockQuotaAll.mockReset();
    mockQuotaAll.mockResolvedValue([]);
  });

  // 元件寫好卻沒掛進分頁清單，使用者就永遠點不到 —— 型別檢查與元件自己的
  // 測試都抓不到這件事，只有從 SettingsView 點進去才驗得出來。
  it("側欄有用量入口，點了會渲染配額區塊", async () => {
    render(
      <MemoryRouter>
        <SettingsView />
      </MemoryRouter>,
    );

    const entry = screen.getByRole("button", { name: /訂閱額度|Subscription quota/ });
    await userEvent.click(entry);

    await waitFor(() => expect(mockQuotaAll).toHaveBeenCalled());
    expect(screen.getByTestId("quota-refresh")).toBeInTheDocument();
  });
});
