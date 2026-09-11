import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

const detectMock = vi.fn();
vi.mock("../../ipc/shell", () => ({
  detectPowerShell7: () => detectMock(),
}));

import { LocaleProvider } from "../../contexts/LocaleContext";
import type { ShellIdentity } from "../../hooks/useShellIdentity";
import { ShellWarningBadge } from "./index";

afterEach(() => {
  vi.clearAllMocks();
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
