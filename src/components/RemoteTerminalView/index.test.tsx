import React from "react";
import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

// 事件訂閱的假實作：測試自己保留 callback，之後手動觸發。
const handlers: Record<string, (v: never) => void> = {};
function captureHandler(name: string) {
  return (connId: string, cb: (v: never) => void) => {
    handlers[`${name}:${connId}`] = cb;
    return Promise.resolve(() => {});
  };
}

const sendMock = vi.fn();
const disconnectMock = vi.fn().mockResolvedValue(undefined);

vi.mock("../../ipc/shareViewer", () => ({
  onShareViewerGranted: captureHandler("granted"),
  onShareViewerData: captureHandler("data"),
  onShareViewerResync: captureHandler("resync"),
  onShareViewerControlChanged: captureHandler("control"),
  onShareViewerEnded: captureHandler("ended"),
  shareViewerSend: (...a: unknown[]) => sendMock(...a),
  shareViewerDisconnect: (...a: unknown[]) => disconnectMock(...a),
}));

// xterm 在 jsdom 下量不到尺寸，用假的。
const writeMock = vi.fn();
const clearMock = vi.fn();
vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    write = writeMock;
    clear = clearMock;
    open = vi.fn();
    dispose = vi.fn();
    onData = vi.fn();
    loadAddon = vi.fn();
    scrollToBottom = vi.fn();
    resize = vi.fn();
    cols = 80;
    options: Record<string, unknown> = {};
    parser = { registerOscHandler: vi.fn(() => ({ dispose: vi.fn() })) };
    buffer = { onBufferChange: vi.fn(() => ({ dispose: vi.fn() })), active: { type: "normal" } };
  },
}));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit = vi.fn(); } }));

// jsdom doesn't implement ResizeObserver; RemoteTerminalView's font-fitting
// effect calls it unconditionally on mount (same pattern as TerminalView's
// tests).
class FakeResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
(globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = FakeResizeObserver;

import { RemoteTerminalView } from "./index";

beforeEach(() => {
  for (const k of Object.keys(handlers)) delete handlers[k];
  writeMock.mockReset();
  clearMock.mockReset();
  sendMock.mockReset();
  disconnectMock.mockReset().mockResolvedValue(undefined);
});

describe("RemoteTerminalView", () => {
  it("shows its own verification code for the user to read out", async () => {
    // 觀看端**必須**顯示自己算出的碼——那是要唸給對方聽的。主控端相反，
    // 那邊絕不顯示自己的碼（否則會照抄而不問對方）。兩邊不對稱是刻意的。
    //
    // SAS 走 prop 而不是事件：它在連線建立當下就已知，而這個元件要等分頁
    // 開好才掛載——用事件送必然遺失。實機測試就是這樣抓到的。
    render(<RemoteTerminalView tabId="t1" connId="c1" sas="4917" isActive />);
    expect(await screen.findByText("4917")).toBeInTheDocument();
  });

  it("does not send keystrokes while read-only", async () => {
    render(<RemoteTerminalView tabId="t1" connId="c2" sas="1111" isActive />);
    await waitFor(() => expect(handlers["granted:c2"]).toBeDefined());

    handlers["granted:c2"]({ mode: "read_only", cols: 80, rows: 24 } as never);

    // 伺服器端還有一道 may_send_input 檢查，但前端這層是給使用者的回饋：
    // 唯讀時按鍵**根本不送出**，而不是送了被拒絕。
    await waitFor(() => expect(screen.getByText(/唯讀|Read-only/)).toBeInTheDocument());
    expect(sendMock).not.toHaveBeenCalled();
  });

  it("clears the screen before a resync replay", async () => {
    // 漏掉的位元組可能截斷 ANSI 逃脫序列——帶著壞掉的畫面繼續是不會自己好的。
    render(<RemoteTerminalView tabId="t1" connId="c3" sas="2222" isActive />);
    await waitFor(() => expect(handlers["resync:c3"]).toBeDefined());

    handlers["resync:c3"](undefined as never);

    await waitFor(() => expect(clearMock).toHaveBeenCalled());
  });

  it("keeps the last screen and explains why the connection ended", async () => {
    render(<RemoteTerminalView tabId="t1" connId="c4" sas="3333" isActive />);
    await waitFor(() => expect(handlers["ended:c4"]).toBeDefined());

    handlers["ended:c4"]("host_stopped_sharing" as never);

    // 不清空畫面——最後看到的內容仍要能閱讀。
    expect(clearMock).not.toHaveBeenCalled();
    expect(
      await screen.findByText(/對方停止分享了|They stopped sharing/),
    ).toBeInTheDocument();
  });

  it("shows a human sentence for an unrecognised end reason", async () => {
    // spec 要求「不能有『未知錯誤』」。真的收到沒見過的 reason 時（例如
    // 對方是更新版），也要給一句人話而不是原始字串。
    render(<RemoteTerminalView tabId="t1" connId="c5" sas="4444" isActive />);
    await waitFor(() => expect(handlers["ended:c5"]).toBeDefined());

    handlers["ended:c5"]("something_from_the_future" as never);

    expect(screen.queryByText("something_from_the_future")).not.toBeInTheDocument();
  });

  describe("disconnect timing (StrictMode dev-mode trap)", () => {
    // 實機測試抓到的 bug：連線是在這個元件掛載**之前**建立的（見
    // `shareViewerConnect` 的說明），所以 StrictMode 在 dev 模式下模擬
    // 「掛載→卸載→重新掛載」時，模擬卸載觸發的 `shareViewerDisconnect`
    // 沒有對應的「重新連線」可以復原——後端把連線刪掉後，`connId` 不變
    // 就不會重連，之後打字全部送進一條死連線，控制模式看起來像唯讀。
    afterEach(() => {
      vi.useRealTimers();
    });

    it("does not disconnect when StrictMode's simulated unmount is followed by an immediate remount", () => {
      vi.useFakeTimers();
      render(
        <React.StrictMode>
          <RemoteTerminalView tabId="t1" connId="c6" sas="5555" isActive />
        </React.StrictMode>,
      );

      // StrictMode 的模擬卸載已經在 render() 完成時跑過一輪；真正斷線的
      // setTimeout 還沒被清空的話，代表重新掛載沒有把它取消掉。
      vi.runAllTimers();

      expect(disconnectMock).not.toHaveBeenCalled();
    });

    it("disconnects for real when the component actually unmounts with no remount", () => {
      vi.useFakeTimers();
      const { unmount } = render(
        <RemoteTerminalView tabId="t1" connId="c7" sas="6666" isActive />,
      );

      unmount();
      vi.runAllTimers();

      expect(disconnectMock).toHaveBeenCalledWith("c7");
    });
  });
});
