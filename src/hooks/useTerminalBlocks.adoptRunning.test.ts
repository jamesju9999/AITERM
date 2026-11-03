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

// 後端開好分頁就直接把啟動指令寫進 PTY，前端接手時 OSC 133 C 早就過了，
// 沒有任何區塊——畫面只剩「輸入指令開始執行」，Claude 在跑也看不到。
describe("useTerminalBlocks — adoptRunningBlock（接手已在跑的指令）", () => {
  it("補建一個 running 區塊，之後的輸出會畫進卡片", async () => {
    const { result } = renderHook(() => useTerminalBlocks("s1", term));
    expect(result.current.blocks).toHaveLength(0);

    act(() => result.current.adoptRunningBlock("claude"));
    expect(result.current.blocks).toHaveLength(1);
    expect(result.current.blocks[0]).toMatchObject({ command: "claude", status: "running" });

    const frame = "WELCOME-BOX\r\nINPUT-BOX";
    await write(frame);
    act(() => result.current.appendOutput(frame));
    await waitFor(() => expect(textOf(result.current.blocks[0])).toContain("INPUT-BOX"));
  });

  it("指令結束（OSC 133 D）時照常結案", async () => {
    const { result } = renderHook(() => useTerminalBlocks("s2", term));
    act(() => result.current.adoptRunningBlock("claude"));
    await write("bye\r\n\x1b]133;D;0\x07");
    await waitFor(() => expect(result.current.blocks[0].status).not.toBe("running"));
  });

  it("已經有 running 區塊就不重複補建", () => {
    const { result } = renderHook(() => useTerminalBlocks("s3", term));
    act(() => result.current.adoptRunningBlock("claude"));
    act(() => result.current.adoptRunningBlock("claude"));
    expect(result.current.blocks).toHaveLength(1);
  });
});
