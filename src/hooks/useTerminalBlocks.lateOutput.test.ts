import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Terminal } from "@xterm/xterm";

vi.mock("../ipc/pty", () => ({ writePty: vi.fn().mockResolvedValue(undefined) }));

import { useTerminalBlocks } from "./useTerminalBlocks";

const write = (term: Terminal, data: string) => new Promise<void>((r) => term.write(data, r));
const textOf = (b: { renderedLines?: { spans: { text: string }[] }[] }) =>
  (b.renderedLines ?? []).map((l) => l.spans.map((s) => s.text).join(""));

let term: Terminal;
beforeEach(() => {
  term = new Terminal({ cols: 80, rows: 24, allowProposedApi: true });
});
afterEach(() => term.dispose());

describe("Windows ConPTY：D 比輸出文字早到（實機 Get-ExecutionPolicy -List 只剩表頭）", () => {
  it("結案後文字才補到，OSC 133 P 到達時把尾段補回卡片", async () => {
    const { result } = renderHook(() =>
      useTerminalBlocks("s1", term, undefined, undefined, undefined, undefined, undefined, "windows"),
    );
    await act(async () => {
      await write(term, "PS C:\\> \x1b]133;B\x07");
    });
    act(() => {
      result.current.submitCommand("Get-ExecutionPolicy -List");
    });
    await act(async () => {
      await write(term, "\r\n\x1b]133;C\x07");
      // 表頭先到，D 在資料列之前就到了
      await write(term, "Scope Policy\r\n----- ------\r\n\x1b]133;D;0\x07");
    });
    expect(textOf(result.current.blocks[0]).join("\n")).not.toContain("Undefined");

    await act(async () => {
      // 延後的資料列，接著才是提示字元（P 在文字最前面）
      await write(term, "Process Undefined\r\nUser Undefined\r\n\x1b]133;P\x07PS C:\\> \x1b]133;B\x07");
    });
    const lines = textOf(result.current.blocks[0]);
    expect(lines.join("\n")).toContain("Process Undefined");
    expect(lines.join("\n")).toContain("User Undefined");
    expect(lines.join("\n")).not.toContain("PS C:\\> \x1b"); // 提示字元不混進輸出
    expect(lines.filter((l) => l.startsWith("PS C:\\>"))).toHaveLength(0);
  });
});

describe("尾段補讀不能蓋到後面的區塊", () => {
  it("執行中又從輸入框送出新指令（例如回答 winget 的 Y/N）：舊區塊不會把新區塊的輸出也讀進去", async () => {
    const { result } = renderHook(() =>
      useTerminalBlocks("s1", term, undefined, undefined, undefined, undefined, undefined, "windows"),
    );
    await act(async () => {
      await write(term, "PS C:\\> \x1b]133;B\x07");
    });
    act(() => {
      result.current.submitCommand("winget install x");
    });
    await act(async () => {
      await write(term, "\r\n\x1b]133;C\x07[Y] yes [N] no: ");
    });
    // 使用者在輸入框回答 Y：舊區塊被強制結案，新區塊開始
    act(() => {
      result.current.submitCommand("Y");
    });
    await act(async () => {
      await write(term, "Y\r\ninstalling...\r\ndone\r\n\x1b]133;D;0\x07\x1b]133;P\x07PS C:\\> \x1b]133;B\x07");
    });
    const [first] = result.current.blocks;
    expect(textOf(first).join("\n")).not.toContain("done");
  });
});
