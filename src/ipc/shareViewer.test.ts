import { describe, expect, it, vi, beforeEach } from "vitest";

const invokeMock = vi.fn();
const listenMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: (...a: unknown[]) => listenMock(...a) }));

import {
  shareViewerConnect,
  shareViewerSend,
  onShareViewerSas,
  onShareViewerData,
} from "./shareViewer";

beforeEach(() => {
  invokeMock.mockReset();
  listenMock.mockReset();
  invokeMock.mockResolvedValue("conn-1");
  listenMock.mockResolvedValue(() => {});
});

describe("share viewer IPC", () => {
  it("passes host, port, code and display name when connecting", async () => {
    const id = await shareViewerConnect("192.168.1.33", 47823, "559207", "Bob");
    expect(invokeMock).toHaveBeenCalledWith("share_viewer_connect", {
      host: "192.168.1.33",
      port: 47823,
      code: "559207",
      displayName: "Bob",
    });
    expect(id).toBe("conn-1");
  });

  it("scopes the sas event to the connection id", async () => {
    // 每條連線一組事件名稱，比照本機 PTY 的 `pty://data/{id}`。同時開兩個
    // 遠端分頁時，兩邊的畫面不能混在一起。
    await onShareViewerSas("conn-1", () => {});
    expect(listenMock).toHaveBeenCalledWith(
      "share-viewer://sas/conn-1",
      expect.any(Function),
    );
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
