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
    const r = await shareViewerConnect("192.168.1.33", 47823, "559207", "Bob");
    expect(invokeMock).toHaveBeenCalledWith("share_viewer_connect", {
      host: "192.168.1.33",
      port: 47823,
      code: "559207",
      displayName: "Bob",
    });
    expect(r.connId).toBe("conn-1");
  });

  it("returns the SAS with the connection rather than pushing it as an event", async () => {
    // 事件會在訂閱者存在之前就發出去——實機測試抓到觀看端的驗證碼永遠空白，
    // 就是因為元件要等分頁開好才掛載，那時事件早就過去了。回傳值沒有時間差。
    const r = await shareViewerConnect("192.168.1.33", 47823, "559207", "Bob");
    expect(r.sas).toBe("4917");
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
