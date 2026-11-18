import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { MemoryRouter } from "react-router-dom";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { translations, LOCALE_STORAGE_KEY } from "../../lib/i18n";

const getConfigMock = vi.fn();
vi.mock("../../ipc/config", () => ({ getConfig: () => getConfigMock() }));

import { SetupChecklist } from "./SetupChecklist";

const t = translations["zh-TW"];
const cfg = (over: Record<string, unknown> = {}) => ({
  providers: [{ id: "p" }], mail_accounts: [], mcp_servers: [], db_connections: [], vcs_connections: [],
  claude_bridge: { enabled: false }, ...over,
});
const wrap = () => render(<MemoryRouter><LocaleProvider><SetupChecklist /></LocaleProvider></MemoryRouter>);

describe("SetupChecklist", () => {
  beforeEach(() => { getConfigMock.mockReset(); localStorage.clear(); localStorage.setItem(LOCALE_STORAGE_KEY, "zh-TW"); });

  it("lists what is not set up, each with a jump button", async () => {
    getConfigMock.mockResolvedValue(cfg());
    wrap();
    expect(await screen.findByText(t.home_setup_mail)).toBeInTheDocument();
    expect(screen.getByText(t.home_setup_db)).toBeInTheDocument();
    expect(screen.queryByText(t.home_setup_provider)).toBeNull(); // 已設定的不列
    expect(screen.getAllByRole("button", { name: t.home_setup_go }).length).toBe(5);
  });

  it("renders nothing when everything is configured", async () => {
    getConfigMock.mockResolvedValue(cfg({
      mail_accounts: [{ id: "a" }], mcp_servers: [{ id: "m" }], db_connections: [{ id: "d" }],
      vcs_connections: [{ id: "v" }], claude_bridge: { enabled: true },
    }));
    wrap();
    await waitFor(() => expect(getConfigMock).toHaveBeenCalled());
    expect(screen.queryByTestId("home-setup")).toBeNull();
  });

  it("flags a mail account that exists but has receiving disabled", async () => {
    getConfigMock.mockResolvedValue(cfg({ mail_accounts: [{ id: "a", enabled: false }] }));
    wrap();
    expect(await screen.findByText(t.home_setup_mail_off)).toBeInTheDocument();
  });

  it("'not needed' hides an optional item and remembers it; the provider cannot be dismissed", async () => {
    getConfigMock.mockResolvedValue(cfg({ providers: [] }));
    const first = wrap();
    await screen.findByText(t.home_setup_provider);
    // 供應商沒有「不需要」；其餘 5 項各有一顆
    expect(screen.getAllByRole("button", { name: t.home_setup_dismiss }).length).toBe(5);
    fireEvent.click(screen.getAllByRole("button", { name: t.home_setup_dismiss })[0]); // 信箱
    expect(screen.queryByText(t.home_setup_mail)).toBeNull();
    first.unmount();
    wrap();
    await screen.findByText(t.home_setup_provider);
    expect(screen.queryByText(t.home_setup_mail)).toBeNull(); // 重新載入後仍隱藏
  });

  it("shows nothing (and does not throw) if the config cannot be read", async () => {
    getConfigMock.mockRejectedValue(new Error("boom"));
    wrap();
    await waitFor(() => expect(getConfigMock).toHaveBeenCalled());
    expect(screen.queryByTestId("home-setup")).toBeNull();
  });
});
