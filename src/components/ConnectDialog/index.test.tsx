import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const connectMock = vi.fn();
vi.mock("../../ipc/shareViewer", () => ({
  shareViewerConnect: (...a: unknown[]) => connectMock(...a),
}));

const discoverMock = vi.fn();
vi.mock("../../ipc/share", () => ({
  shareDiscover: (...a: unknown[]) => discoverMock(...a),
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
  discoverMock.mockReset().mockResolvedValue({ kind: "notFound" });
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

  it("connects with an address and a key but no short code", async () => {
    // 金鑰模式**一定**沒有短碼——身分完全由金鑰決定，觀看端連送出去的
    // `code` 都是空字串。實機回報的 bug：送出鈕的啟用條件寫死成「短碼剛好
    // 6 位」，所以金鑰填好了按鈕還是灰的，整條 CLI host 的路完全走不通。
    //
    // 既有的每一條對話框測試都會順手打一組 6 位短碼，所以它們全綠也抓不到
    // 這件事——那正是這條測試存在的理由。
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.click(screen.getByText(/直接輸入位址/));
    await userEvent.type(screen.getByPlaceholderText(/192\.168/), "127.0.0.1:18022");
    await userEvent.type(screen.getByLabelText(/你的名字/), "Bob");
    await userEvent.type(screen.getByLabelText(/金鑰/), "ab".repeat(32));

    const submit = screen.getByRole("button", { name: /^連線$/ });
    expect(submit).toBeEnabled();
    await userEvent.click(submit);

    expect(connectMock).toHaveBeenCalledWith({
      host: "127.0.0.1",
      port: 18022,
      code: "",
      displayName: "Bob",
      key: "ab".repeat(32),
    });
  });

  it("still requires a short code when no key is given", async () => {
    // 反面：沒有金鑰時，送出鈕仍然必須等到短碼滿 6 位才能按。放寬成「有位址
    // 就能按」會讓短碼模式在還沒填完碼時就送出去。
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.click(screen.getByText(/直接輸入位址/));
    await userEvent.type(screen.getByPlaceholderText(/192\.168/), "127.0.0.1:18022");
    expect(screen.getByRole("button", { name: /^連線$/ })).toBeDisabled();
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

    expect(connectMock).toHaveBeenCalledWith({
      host: "192.168.1.33",
      port: 47823,
      code: "559207",
      displayName: "Bob",
      key: undefined,
    });
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

  it("connects straight through when mDNS finds exactly one match", async () => {
    discoverMock.mockResolvedValue({ kind: "found", host: "192.168.1.50", port: 9000 });
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.type(screen.getByLabelText(/6 位數/), "632706");
    await userEvent.click(screen.getByRole("button", { name: /^連線$/ }));

    expect(discoverMock).toHaveBeenCalledWith("632706");
    expect(connectMock).toHaveBeenCalledWith({
      host: "192.168.1.50",
      port: 9000,
      code: "632706",
      displayName: "AITerm",
      key: undefined,
    });
    expect(onConnected).toHaveBeenCalledWith("conn-1", "4917", "192.168.1.50:9000");
  });

  it("falls back to the manual address field when mDNS finds nothing", async () => {
    discoverMock.mockResolvedValue({ kind: "notFound" });
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.type(screen.getByLabelText(/6 位數/), "632706");
    await userEvent.click(screen.getByRole("button", { name: /^連線$/ }));

    expect(await screen.findByText(/找不到這組編號/)).toBeInTheDocument();
    expect(screen.getByPlaceholderText(/192\.168/)).toBeInTheDocument();
    expect(connectMock).not.toHaveBeenCalled();
  });

  it("shows a distinct message when mDNS finds more than one match", async () => {
    discoverMock.mockResolvedValue({ kind: "ambiguous" });
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.type(screen.getByLabelText(/6 位數/), "632706");
    await userEvent.click(screen.getByRole("button", { name: /^連線$/ }));

    expect(await screen.findByText(/不只一台機器/)).toBeInTheDocument();
    expect(screen.getByPlaceholderText(/192\.168/)).toBeInTheDocument();
    expect(connectMock).not.toHaveBeenCalled();
  });

  it("skips mDNS entirely once the manual address field has something in it", async () => {
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.click(screen.getByText(/直接輸入位址/));
    await userEvent.type(screen.getByPlaceholderText(/192\.168/), "192.168.1.33:47823");
    await userEvent.type(screen.getByLabelText(/6 位數/), "632706");
    await userEvent.click(screen.getByRole("button", { name: /^連線$/ }));

    expect(discoverMock).not.toHaveBeenCalled();
    expect(connectMock).toHaveBeenCalledWith({
      host: "192.168.1.33",
      port: 47823,
      code: "632706",
      displayName: "AITerm",
      key: undefined,
    });
  });
});
