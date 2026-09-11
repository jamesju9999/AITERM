import { describe, expect, it, vi, beforeEach } from "vitest";

const invokeMock = vi.fn();
const listenMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: (...a: unknown[]) => listenMock(...a) }));

import { shareViewerConnect, shareViewerSend, onShareViewerData } from "./shareViewer";

beforeEach(() => {
  invokeMock.mockReset();
  listenMock.mockReset();
  invokeMock.mockResolvedValue({ connId: "conn-1", sas: "4917" });
  listenMock.mockResolvedValue(() => {});
});

describe("share viewer IPC", () => {
  it("passes host, port, code and display name when connecting", async () => {
    const r = await shareViewerConnect({
      host: "192.168.1.33",
      port: 47823,
      code: "559207",
      displayName: "Bob",
    });
    expect(invokeMock).toHaveBeenCalledWith("share_viewer_connect", {
      host: "192.168.1.33",
      port: 47823,
      code: "559207",
      displayName: "Bob",
      key: undefined,
    });
    expect(r.connId).toBe("conn-1");
  });

  it("returns the SAS with the connection rather than pushing it as an event", async () => {
    // 事件會在訂閱者存在之前就發出去——實機測試抓到觀看端的驗證碼永遠空白，
    // 就是因為元件要等分頁開好才掛載，那時事件早就過去了。回傳值沒有時間差。
    const r = await shareViewerConnect({
      host: "192.168.1.33",
      port: 47823,
      code: "559207",
      displayName: "Bob",
    });
    expect(r.sas).toBe("4917");
  });

  it("passes the key through to the backend when given", async () => {
    invokeMock.mockResolvedValue({ connId: "c1", sas: "" });
    await shareViewerConnect({
      host: "10.0.0.5",
      port: 8022,
      code: "",
      displayName: "James",
      key: "ab".repeat(32),
    });
    expect(invokeMock).toHaveBeenCalledWith("share_viewer_connect", {
      host: "10.0.0.5",
      port: 8022,
      code: "",
      displayName: "James",
      key: "ab".repeat(32),
    });
  });

  it("omits the key for code-mode connections", async () => {
    // 短碼模式必須送 undefined 而不是空字串——空字串在後端會被當成「有金鑰
    // 但是空的」，握手直接失敗，而且錯誤訊息會指向金鑰不符，完全誤導。
    invokeMock.mockResolvedValue({ connId: "c1", sas: "1234" });
    await shareViewerConnect({
      host: "10.0.0.5",
      port: 8022,
      code: "384719",
      displayName: "James",
    });
    expect(invokeMock).toHaveBeenCalledWith("share_viewer_connect", {
      host: "10.0.0.5",
      port: 8022,
      code: "384719",
      displayName: "James",
      key: undefined,
    });
  });

  it("scopes the data event to the connection id", async () => {
    await onShareViewerData("conn-2", () => {});
    expect(listenMock).toHaveBeenCalledWith(
      "share-viewer://data/conn-2",
      expect.any(Function),
    );
  });

  it("sends keystrokes as a plain string", async () => {
    invokeMock.mockResolvedValue(undefined);
    await shareViewerSend("conn-1", "ls\n");
    expect(invokeMock).toHaveBeenCalledWith("share_viewer_send", {
      connId: "conn-1",
      data: "ls\n",
    });
  });
});
