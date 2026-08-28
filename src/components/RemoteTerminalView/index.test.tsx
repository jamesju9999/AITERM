import React from "react";
import { act } from "react";
import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// 事件訂閱的假實作：測試自己保留 callback，之後手動觸發。
const handlers: Record<string, (v: never) => void> = {};
let capturedOscHandler: ((data: string) => boolean) | null = null;
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

// 只包一層 spy 在真正的 appendOutput 上，其餘完全用真的 hook——這個檔案
// mock 掉整個 @xterm/xterm，導致 useTerminalBlocks 內部 finalizeBlock 用
// 的無頭 headless Terminal 也一併被 mock，parseAnsiToRenderedLines 量不到
// 真正的 buffer 內容，卡片本體永遠是空的。要驗證「PTY 位元組真的有餵給
// appendOutput、而且解碼正確」，比起修那一層更深的 mock，直接在這個接點
// 上釘一根探針最直接可靠。
let appendOutputSpy: ReturnType<typeof vi.fn> | null = null;
vi.mock("../../hooks/useTerminalBlocks", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../hooks/useTerminalBlocks")>();
  return {
    ...actual,
    useTerminalBlocks: (...args: Parameters<typeof actual.useTerminalBlocks>) => {
      const result = actual.useTerminalBlocks(...args);
      appendOutputSpy = vi.fn(result.appendOutput);
      return { ...result, appendOutput: appendOutputSpy };
    },
  };
});

// xterm 在 jsdom 下量不到尺寸，用假的。
const writeMock = vi.fn((_data: unknown, callback?: () => void) => {
  // xterm's `write(data, callback)` invokes `callback` once the write is
  // flushed. `ansiBlockParser.ts`'s `parseAnsiToRenderedLines` (called from
  // inside `useTerminalBlocks`' `finalizeBlock`) awaits exactly this
  // callback on an internal headless `Terminal` instance — which, because
  // this file mocks the whole @xterm/xterm module, is also this same mock.
  // Without invoking the callback here, that internal await never
  // resolves and any test that drives a block to completion hangs forever.
  callback?.();
});
const clearMock = vi.fn();
// 可變動的假 buffer 狀態，讓測試能模擬「全螢幕程式（vim/htop 等）進入/
// 離開 alternate buffer」這個切換，並手動觸發 useTerminalBlocks 內部訂閱
// 的 onBufferChange callback——`active` 物件跨測試共用同一個參照，
// beforeEach 只重置它的欄位，不重新賦值，這樣即使元件重新掛載出新的
// Terminal 實例，讀到的都是同一份、當下正確的狀態。
const mockBufferActive: { type: "normal" | "alternate" } = { type: "normal" };
let capturedBufferChangeHandler: (() => void) | null = null;
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
    parser = {
      registerOscHandler: vi.fn((_code: number, handler: (data: string) => boolean) => {
        capturedOscHandler = handler;
        return { dispose: vi.fn() };
      }),
    };
    buffer = {
      onBufferChange: vi.fn((cb: () => void) => {
        capturedBufferChangeHandler = cb;
        return { dispose: vi.fn() };
      }),
      active: mockBufferActive,
    };
  },
}));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit = vi.fn(); } }));

// jsdom doesn't implement ResizeObserver — kept as a no-op global shim in
// case something else in the render tree still expects it to exist
// (harmless if nothing does).
class FakeResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
(globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = FakeResizeObserver;

// jsdom doesn't implement Element.scrollTo either — same category as the
// ResizeObserver shim above. Needed as a real (stubbable) no-op, not just an
// optional-chained call site, because one of this file's tests spies on it
// with `vi.spyOn(HTMLElement.prototype, "scrollTo")`, which requires the
// property to already exist on the prototype.
if (!window.HTMLElement.prototype.scrollTo) {
  window.HTMLElement.prototype.scrollTo = () => {};
}

import { RemoteTerminalView } from "./index";

beforeEach(() => {
  for (const k of Object.keys(handlers)) delete handlers[k];
  capturedOscHandler = null;
  capturedBufferChangeHandler = null;
  mockBufferActive.type = "normal";
  appendOutputSpy = null;
  writeMock.mockClear();
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

  it("disables WarpInput while read-only", async () => {
    render(<RemoteTerminalView tabId="t1" connId="c8" sas="7777" isActive />);
    await waitFor(() => expect(handlers["granted:c8"]).toBeDefined());

    handlers["granted:c8"]({ mode: "read_only", cols: 80, rows: 24, hostOs: "linux" } as never);

    await waitFor(() => {
      const textarea = screen.getByPlaceholderText(/輸入指令|Type a command/i);
      expect(textarea).toBeDisabled();
    });
  });

  it("shows a hint and does not send /ai or /agent commands", async () => {
    render(<RemoteTerminalView tabId="t1" connId="c9" sas="8888" isActive />);
    await waitFor(() => expect(handlers["granted:c9"]).toBeDefined());
    handlers["granted:c9"]({ mode: "control", cols: 80, rows: 24, hostOs: "linux" } as never);

    const textarea = await screen.findByPlaceholderText(/輸入指令|Type a command/i);
    await waitFor(() => expect(textarea).not.toBeDisabled());

    await userEvent.type(textarea, "/ai fix this{Enter}");

    expect(await screen.findByText(/AI 指令目前不支援|not supported in remote/i)).toBeInTheDocument();
    expect(sendMock).not.toHaveBeenCalled();
  });

  it("clears the block list on resync, not just the xterm buffer", async () => {
    render(<RemoteTerminalView tabId="t1" connId="c10" sas="9999" isActive />);
    await waitFor(() => expect(handlers["granted:c10"]).toBeDefined());
    handlers["granted:c10"]({ mode: "control", cols: 80, rows: 24, hostOs: "linux" } as never);

    const textarea = await screen.findByPlaceholderText(/輸入指令|Type a command/i);
    await waitFor(() => expect(textarea).not.toBeDisabled());
    await userEvent.type(textarea, "echo hi{Enter}");

    // 先確認卡片真的出現了——不然這個測試就算沒接 clearAllBlocks 也會通過
    // （之前就是這樣：running 中的區塊本來就不會渲染，跟 resync 有沒有清
    // 卡片無關，測試從沒有真的紅過）。用捕捉到的 OSC 133 handler 模擬指令
    // 執行完成，讓區塊真正變成 completed 並產生 renderedLines。
    await waitFor(() => expect(capturedOscHandler).toBeTruthy());
    act(() => {
      capturedOscHandler!("D;0");
    });
    expect(await screen.findByText("echo hi")).toBeInTheDocument();

    await waitFor(() => expect(handlers["resync:c10"]).toBeDefined());
    handlers["resync:c10"](undefined as never);

    // Resync 之後不該還看得到 resync 之前追蹤的指令卡片標題。
    await waitFor(() => {
      expect(screen.queryByText("echo hi")).not.toBeInTheDocument();
    });
  });

  it("decodes incoming PTY bytes as UTF-8 and feeds them into appendOutput, not just onto the xterm screen", async () => {
    // 實機測試抓到的 bug：`onShareViewerData` 只把位元組寫進 xterm 畫面，
    // 從沒呼叫 `appendOutput`——分段卡片的 rawOutput 永遠是空字串，卡片
    // 因此只有指令文字跟耗時，完全看不到任何輸出內容（連 ls 的檔案清單
    // 都不見）。這個測試釘住兩件事：appendOutput 真的有被呼叫；傳進去的
    // 是正確解碼的 UTF-8 字串，不是 atob() 那種一 byte 一字元的 Latin1
    // 亂碼——這個 repo 的實際檔名經常含中文，錯誤解碼會直接看得出來。
    render(<RemoteTerminalView tabId="t1" connId="c11" sas="1234" isActive />);
    await waitFor(() => expect(handlers["granted:c11"]).toBeDefined());
    handlers["granted:c11"]({ mode: "control", cols: 80, rows: 24, hostOs: "linux" } as never);

    await waitFor(() => expect(handlers["data:c11"]).toBeDefined());
    const utf8Text = "20260818提供報告\r\n";
    // 用瀏覽器原生 API 編碼——這個檔案的其他地方也是這樣處理 base64
    // （見 onShareViewerData 本身用 atob 解碼），這裡刻意不用 Node 的
    // Buffer：這個 repo 的 tsconfig「types」欄位限制過，不含 node，用了
    // 會讓 `npx tsc -b` 報錯（見其他測試檔案踩過同一個坑的註解）。
    const utf8Bytes = new TextEncoder().encode(utf8Text);
    const base64 = btoa(String.fromCharCode(...utf8Bytes));
    act(() => {
      handlers["data:c11"](base64 as never);
    });

    await waitFor(() => expect(appendOutputSpy).not.toBeNull());
    expect(appendOutputSpy).toHaveBeenCalledWith(utf8Text);
  });

  it("即時窗格在指令執行中撐到最大高度、指令完成變成卡片後收回最小高度", async () => {
    const { container } = render(<RemoteTerminalView tabId="t1" connId="c12" sas="1212" isActive />);
    await waitFor(() => expect(handlers["granted:c12"]).toBeDefined());
    handlers["granted:c12"]({ mode: "control", cols: 80, rows: 24, hostOs: "linux" } as never);

    const textarea = await screen.findByPlaceholderText(/輸入指令|Type a command/i);
    await waitFor(() => expect(textarea).not.toBeDisabled());
    await userEvent.type(textarea, "echo hi{Enter}");

    // 指令送出後，模擬 shell 真的產生了一批輸出——即時窗格應該撐到最大
    // 高度（測試環境量不到 xterm 真正的字元格尺寸，會落到 14*1.1 的
    // fallback，MAX_LIVE_ROWS=16 對應 Math.round(16*14*1.1) = 246px）。
    await waitFor(() => expect(handlers["data:c12"]).toBeDefined());
    act(() => {
      handlers["data:c12"](btoa("hi\r\n") as never);
    });

    const liveFrame = () => container.querySelector(".aiterm-remote-terminal__live-frame") as HTMLElement;
    await waitFor(() => {
      expect(liveFrame().style.height).toBe("246px");
    });

    // 指令執行完畢、變成卡片——即時窗格應該收回最小高度
    // （MIN_LIVE_ROWS=3 對應 Math.round(3*14*1.1) = 46px）。
    await waitFor(() => expect(capturedOscHandler).toBeTruthy());
    act(() => {
      capturedOscHandler!("D;0");
    });
    await waitFor(() => {
      expect(liveFrame().style.height).toBe("46px");
    });
  });

  it("新卡片出現時自動捲動到最底部", async () => {
    const scrollToSpy = vi.spyOn(HTMLElement.prototype, "scrollTo").mockImplementation(() => {});
    try {
      render(<RemoteTerminalView tabId="t1" connId="c13" sas="1313" isActive />);
      await waitFor(() => expect(handlers["granted:c13"]).toBeDefined());
      handlers["granted:c13"]({ mode: "control", cols: 80, rows: 24, hostOs: "linux" } as never);

      const textarea = await screen.findByPlaceholderText(/輸入指令|Type a command/i);
      await waitFor(() => expect(textarea).not.toBeDisabled());
      await userEvent.type(textarea, "echo hi{Enter}");

      // 只關心指令完成、卡片出現那一刻的呼叫，清掉掛載/送出指令過程中
      // 可能發生的其他呼叫。
      scrollToSpy.mockClear();

      await waitFor(() => expect(capturedOscHandler).toBeTruthy());
      act(() => {
        capturedOscHandler!("D;0");
      });

      await waitFor(() => expect(screen.getByText("echo hi")).toBeInTheDocument());
      expect(scrollToSpy).toHaveBeenCalled();
    } finally {
      scrollToSpy.mockRestore();
    }
  });

  it("全螢幕程式（vim/htop 等）進入 alternate buffer 時，卡片列表與 WarpInput 隱藏、即時窗格撐滿；離開後恢復", async () => {
    // 實機審查抓到的迴歸：拿掉自動縮放字體（Task 1）+ 即時窗格高度夾在
    // MAX_LIVE_ROWS 並用 overflow:clip 硬裁（Task 2）疊加起來，會讓遠端
    // 觀看端看 vim/htop/tmux 這類全螢幕程式時，畫面被裁到只剩最後 16 行、
    // 其餘完全看不到也滑不到——跟本機終端機一樣，全螢幕程式使用中應該讓
    // 即時窗格撐滿、不裁切、卡片列表與輸入框讓開空間。
    const { container } = render(<RemoteTerminalView tabId="t1" connId="c14" sas="1414" isActive />);
    await waitFor(() => expect(handlers["granted:c14"]).toBeDefined());
    handlers["granted:c14"]({ mode: "control", cols: 80, rows: 24, hostOs: "linux" } as never);

    const liveFrame = () => container.querySelector(".aiterm-remote-terminal__live-frame") as HTMLElement;

    // 一般模式：卡片容器與輸入框都在，即時窗格會裁切。
    await waitFor(() => expect(screen.getByPlaceholderText(/輸入指令|Type a command/i)).toBeInTheDocument());
    expect(container.querySelector(".aiterm-remote-terminal__blocks")).toBeInTheDocument();
    expect(liveFrame().style.overflow).toBe("clip");

    await waitFor(() => expect(capturedBufferChangeHandler).toBeTruthy());
    mockBufferActive.type = "alternate";
    act(() => {
      capturedBufferChangeHandler!();
    });

    await waitFor(() => {
      expect(liveFrame().style.overflow).toBe("visible");
    });
    // 剛好塞下 granted 給的 24 列（測試環境量不到真正的字元格尺寸，會
    // 落到 14*1.1 的 fallback：Math.round(24*14*1.1) = 370px）——不是無
    // 條件撐滿容器的 100%，容器比內容需要的空間大時不該留下空白。
    expect(liveFrame().style.height).toBe("370px");
    expect(container.querySelector(".aiterm-remote-terminal__blocks")).not.toBeInTheDocument();
    expect(screen.queryByPlaceholderText(/輸入指令|Type a command/i)).not.toBeInTheDocument();

    // 離開全螢幕程式後應該完全恢復原本行為。
    mockBufferActive.type = "normal";
    act(() => {
      capturedBufferChangeHandler!();
    });
    await waitFor(() => {
      expect(liveFrame().style.overflow).toBe("clip");
    });
    expect(container.querySelector(".aiterm-remote-terminal__blocks")).toBeInTheDocument();
    expect(screen.getByPlaceholderText(/輸入指令|Type a command/i)).toBeInTheDocument();
  });

  it("全螢幕程式即時窗格的高度跟著主控端實際列數變化，不是無條件撐滿容器", async () => {
    // 實機回報的問題：容器（觀看端視窗）可能比內容（主控端的實際列數）
    // 需要的空間大，撐滿容器的話畫面下方會留一大片沒用到的空白。這裡
    // 用兩種不同列數分別驗證算出來的高度確實跟著列數走、成比例，而不是
    // 兩者都得到同一個「反正就是撐滿」的值——這樣才能真的證明高度是從
    // 主控端給的列數算出來的，不是巧合對到某個固定數字。
    const { container } = render(<RemoteTerminalView tabId="t1" connId="c16" sas="1616" isActive />);
    await waitFor(() => expect(handlers["granted:c16"]).toBeDefined());
    handlers["granted:c16"]({ mode: "control", cols: 80, rows: 10, hostOs: "linux" } as never);

    const liveFrame = () => container.querySelector(".aiterm-remote-terminal__live-frame") as HTMLElement;

    await waitFor(() => expect(capturedBufferChangeHandler).toBeTruthy());
    mockBufferActive.type = "alternate";
    act(() => {
      capturedBufferChangeHandler!();
    });

    // Math.round(10*14*1.1) = 154px。
    await waitFor(() => expect(liveFrame().style.height).toBe("154px"));

    // 主控端的終端機變大（例如視窗拉高）——即時窗格的高度應該跟著長大，
    // 不是維持原本那個值或直接跳去某個「撐滿」的固定值。
    act(() => {
      handlers["granted:c16"]({ mode: "", cols: 80, rows: 40 } as never);
    });

    // Math.round(40*14*1.1) = 616px。
    await waitFor(() => expect(liveFrame().style.height).toBe("616px"));
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
