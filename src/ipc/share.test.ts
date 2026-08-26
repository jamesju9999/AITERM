import { describe, expect, it, vi, beforeEach } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

import { shareStart, shareApprove, sharePending, shareViewers } from "./share";

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
});

describe("share IPC", () => {
  it("passes the tab id when starting a share", async () => {
    invokeMock.mockResolvedValue({ sharing: true, code: "559207", port: 47823 });
    const s = await shareStart("tab-1");
    expect(invokeMock).toHaveBeenCalledWith("share_start", { tabId: "tab-1" });
    expect(s.code).toBe("559207");
  });

  it("sends the typed code to Rust rather than comparing here", async () => {
    // 比對必須在 Rust 端做——前端從來沒收到過主控端的驗證碼，所以連比對
    // 的材料都沒有。這個測試守著「前端只負責把使用者打的字送過去」。
    invokeMock.mockResolvedValue({ kind: "approved", viewerId: "v1" });
    await shareApprove("r1", "control", "4917");
    expect(invokeMock).toHaveBeenCalledWith("share_approve", {
      requestId: "r1",
      mode: "control",
      typedCode: "4917",
    });
  });

  it("never receives a sas field in pending requests", async () => {
    // 型別上就沒有這個欄位。這個測試是給未來的人看的：如果有人把 sas 加
    // 回後端的回傳結構，這裡會提醒他為什麼不該加。
    invokeMock.mockResolvedValue([
      { requestId: "r1", tabId: "t1", displayName: "Alice" },
    ]);
    const pending = await sharePending("t1");
    expect(Object.keys(pending[0])).toEqual(["requestId", "tabId", "displayName"]);
    expect(JSON.stringify(pending)).not.toContain("sas");
  });

  it("reads the viewer list for a tab", async () => {
    invokeMock.mockResolvedValue([
      { viewerId: "v1", displayName: "Alice", mode: "control" },
    ]);
    const viewers = await shareViewers("t1");
    expect(invokeMock).toHaveBeenCalledWith("share_viewers", { tabId: "t1" });
    expect(viewers[0].mode).toBe("control");
  });
});
