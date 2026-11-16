import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const detectMock = vi.fn();
vi.mock("../../ipc/shell", () => ({
  detectPowerShell7: () => detectMock(),
}));

import { LocaleProvider } from "../../contexts/LocaleContext";
import type { ShellIdentity } from "../../hooks/useShellIdentity";
import { ShellWarningBadge, resetShellStartupNoticeForTest } from "./index";

const DISMISS_KEY = "aiterm.shellwarn.dismissed";

beforeEach(() => {
  resetShellStartupNoticeForTest();
  // 既有測試只關心徽章；啟動對話框另外在最後一組測試驗證。
  localStorage.setItem(DISMISS_KEY, "1");
});

afterEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
});

const desktop: ShellIdentity = { shell: "PowerShell", edition: "Desktop", version: "5.1.26100.33158" };
const core: ShellIdentity = { shell: "PowerShell", edition: "Core", version: "7.6.6" };

function mount(identity: ShellIdentity | null) {
  return render(
    <LocaleProvider>
      <ShellWarningBadge identity={identity} />
    </LocaleProvider>,
  );
}

describe("ShellWarningBadge", () => {
  it("PowerShell 7（Core）不顯示徽章", () => {
    detectMock.mockResolvedValue(null);
    mount(core);
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("還沒收到身分（例如 cmd.exe）不顯示徽章", () => {
    detectMock.mockResolvedValue(null);
    mount(null);
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("Core 的情況完全不呼叫後端探測", () => {
    detectMock.mockResolvedValue(null);
    mount(core);
    expect(detectMock).not.toHaveBeenCalled();
  });

  it("5.1 且找得到 pwsh.exe：叫使用者重開 AITerm，不叫他安裝", async () => {
    detectMock.mockResolvedValue("C:\\Program Files\\PowerShell\\7\\pwsh.exe");
    mount(desktop);
    await userEvent.click(await screen.findByRole("button"));

    await screen.findByText(/C:\\Program Files\\PowerShell\\7\\pwsh\.exe/);
    expect(screen.queryByText(/winget install/)).toBeNull();
  });

  it("5.1 且找不到 pwsh.exe：給安裝指令", async () => {
    detectMock.mockResolvedValue(null);
    mount(desktop);
    await userEvent.click(await screen.findByRole("button"));

    await screen.findByText(/winget install --id Microsoft\.PowerShell/);
  });

  it("再點一次收起面板", async () => {
    detectMock.mockResolvedValue(null);
    mount(desktop);
    const badge = await screen.findByRole("button");

    await userEvent.click(badge);
    await screen.findByText(/winget install/);

    await userEvent.click(badge);
    await waitFor(() => expect(screen.queryByText(/winget install/)).toBeNull());
  });
});

describe("ShellWarningBadge 啟動提醒", () => {
  beforeEach(() => localStorage.removeItem(DISMISS_KEY));

  it("舊版 PowerShell 第一次出現時跳對話框", async () => {
    detectMock.mockResolvedValue(null);
    mount(desktop);
    await screen.findByText(/偵測到舊版 PowerShell|Old PowerShell detected/);
  });

  it("PowerShell 7 不跳對話框", () => {
    detectMock.mockResolvedValue(null);
    mount(core);
    expect(screen.queryByText(/偵測到舊版 PowerShell|Old PowerShell detected/)).toBeNull();
  });

  it("每次啟動只跳一次：第二個舊版分頁不再跳", async () => {
    detectMock.mockResolvedValue(null);
    const first = mount(desktop);
    await screen.findByText(/偵測到舊版 PowerShell|Old PowerShell detected/);
    first.unmount();
    mount(desktop);
    expect(screen.queryByText(/偵測到舊版 PowerShell|Old PowerShell detected/)).toBeNull();
  });

  it("按「不再提醒」後永久不跳", async () => {
    detectMock.mockResolvedValue(null);
    const first = mount(desktop);
    await userEvent.click(await screen.findByRole("button", { name: /不再提醒|Don't remind me/ }));
    expect(localStorage.getItem(DISMISS_KEY)).toBe("1");
    first.unmount();
    resetShellStartupNoticeForTest();
    mount(desktop);
    expect(screen.queryByText(/偵測到舊版 PowerShell|Old PowerShell detected/)).toBeNull();
  });
});
