import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const connectMock = vi.fn();
vi.mock("../../ipc/shareViewer", () => ({
  shareViewerConnect: (...a: unknown[]) => connectMock(...a),
}));

vi.mock("../../contexts/LocaleContext", async () => {
  const { translations } = await import("../../lib/i18n");
  return { useLocale: () => ({ t: translations["zh-TW"], locale: "zh-TW", setLocale: () => {} }) };
});

import { ConnectDialog } from "./index";

const onConnected = vi.fn();
const onCancel = vi.fn();

beforeEach(() => {
  connectMock.mockReset().mockResolvedValue({ connId: "conn-1", sas: "4917" });
  onConnected.mockReset();
  onCancel.mockReset();
});

describe("ConnectDialog", () => {
  it("keeps the manual address field out of the way at first", () => {
    // 平常乾淨；出事時才把退路攤開（見 spec 的「觀看端」）。
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    expect(screen.queryByPlaceholderText(/192\.168/)).not.toBeInTheDocument();
  });

  it("reveals the manual address field on demand", async () => {
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.click(screen.getByText(/直接輸入位址/));
    expect(screen.getByPlaceholderText(/192\.168/)).toBeInTheDocument();
  });

  it("connects with a manually entered host and port", async () => {
    // 2C 的 mDNS 還沒上線，手動位址是這個階段唯一的路——也是永遠可用的
    // 主路徑（見 spec 的決策紀錄）。
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.click(screen.getByText(/直接輸入位址/));
    await userEvent.type(screen.getByPlaceholderText(/192\.168/), "192.168.1.33:47823");
    await userEvent.type(screen.getByLabelText(/你的名字/), "Bob");
    await userEvent.type(screen.getByLabelText(/6 位數/), "559207");
    await userEvent.click(screen.getByRole("button", { name: /^連線$/ }));

    expect(connectMock).toHaveBeenCalledWith("192.168.1.33", 47823, "559207", "Bob");
    expect(onConnected).toHaveBeenCalledWith("conn-1", "4917", "192.168.1.33:47823");
  });

  it("rejects an address that is not host:port", async () => {
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.click(screen.getByText(/直接輸入位址/));
    await userEvent.type(screen.getByPlaceholderText(/192\.168/), "just-a-hostname");
    await userEvent.type(screen.getByLabelText(/6 位數/), "559207");
    await userEvent.click(screen.getByRole("button", { name: /^連線$/ }));

    expect(await screen.findByText(/位址格式不對/)).toBeInTheDocument();
    expect(connectMock).not.toHaveBeenCalled();
  });

  it("shows why connecting failed instead of closing silently", async () => {
    connectMock.mockRejectedValue("連不上 192.168.1.33:47823");
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.click(screen.getByText(/直接輸入位址/));
    await userEvent.type(screen.getByPlaceholderText(/192\.168/), "192.168.1.33:47823");
    await userEvent.type(screen.getByLabelText(/6 位數/), "559207");
    await userEvent.click(screen.getByRole("button", { name: /^連線$/ }));

    expect(await screen.findByText(/連不上/)).toBeInTheDocument();
    expect(onConnected).not.toHaveBeenCalled();
  });
});
