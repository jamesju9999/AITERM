import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Terminal } from "@xterm/xterm";

vi.mock("../ipc/pty", () => ({
  writePty: vi.fn().mockResolvedValue(undefined),
}));

import { useTerminalBlocks } from "./useTerminalBlocks";

let term: Terminal;
const write = (data: string) => new Promise<void>((resolve) => term.write(data, resolve));

beforeEach(() => {
  term = new Terminal({ cols: 60, rows: 12, allowProposedApi: true });
});
afterEach(() => term.dispose());

const textOf = (b: { renderedLines?: unknown }) => JSON.stringify(b.renderedLines ?? []);

/** 舊輸出＋指令開始（OSC 133 C）：跟真的 shell 整合送出的順序一樣。 */
async function startCommand(result: { current: ReturnType<typeof useTerminalBlocks> }) {
  await write("OLD-OUTPUT-1\r\nOLD-OUTPUT-2\r\nPS> claude\r\n");
  act(() => {
    result.current.submitCommand("claude");
  });
  await write("\x1b]133;C\x07");
}

describe("useTerminalBlocks — 全螢幕式 TUI 在區塊卡裡的內容", () => {
  it("清畫面後從視窗最上方重畫：整個介面都在，舊輸出不會混進來", async () => {
    const { result } = renderHook(() => useTerminalBlocks("s1", term));
    await startCommand(result);

    const frame = "\x1b[2J\x1b[HWELCOME-BOX\r\nCHAT-HISTORY\r\nINPUT-BOX\r\nSTATUS-LINE";
    await write(frame);
    act(() => result.current.appendOutput(frame));

    await waitFor(() => expect(textOf(result.current.blocks[0])).toContain("STATUS-LINE"));
    const text = textOf(result.current.blocks[0]);
    expect(text).toContain("WELCOME-BOX");
    expect(text).toContain("CHAT-HISTORY");
    expect(text).toContain("INPUT-BOX");
    expect(text).not.toContain("OLD-OUTPUT");
  });

  it("不清畫面、游標往上移回去改寫（Ink 的重畫）：被改寫的上半部也在，沒動過的舊輸出不在", async () => {
    const { result } = renderHook(() => useTerminalBlocks("s2", term));
    await startCommand(result);

    const first = "frame-1 line A\r\nframe-1 line B";
    await write(first);
    act(() => result.current.appendOutput(first));
    // 游標回到視窗最上方（比標記列還高），把整個介面重畫一次
    const redraw = "\x1b[H\x1b[2KWELCOME-BOX\r\n\x1b[2KCHAT-HISTORY\r\n\x1b[2KINPUT-BOX\r\n\x1b[2KSTATUS-LINE\r\n\x1b[2K";
    await write(redraw);
    act(() => result.current.appendOutput(redraw));

    await waitFor(() => expect(textOf(result.current.blocks[0])).toContain("STATUS-LINE"));
    const text = textOf(result.current.blocks[0]);
    expect(text).toContain("WELCOME-BOX");
    expect(text).toContain("CHAT-HISTORY");
    expect(text).not.toContain("frame-1");
  });

  it("一般指令（沒動過上方）：只有指令自己的輸出，跟以前一樣", async () => {
    const { result } = renderHook(() => useTerminalBlocks("s3", term));
    await startCommand(result);

    const out = "hello\r\nworld";
    await write(out);
    act(() => result.current.appendOutput(out));

    await waitFor(() => expect(textOf(result.current.blocks[0])).toContain("world"));
    const text = textOf(result.current.blocks[0]);
    expect(text).toContain("hello");
    expect(text).not.toContain("OLD-OUTPUT");
    expect(text).not.toContain("PS> claude");
  });
});
