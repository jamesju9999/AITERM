import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Terminal } from "@xterm/xterm";

vi.mock("../ipc/pty", () => ({
  writePty: vi.fn().mockResolvedValue(undefined),
}));

// 這個測試需要精確控制 finalizeBlock 的非同步解析何時 resolve，才能可靠地
// 建構出「delayed clear() 觸發時，游標剛好停在自動換行的接續行」這個複合
// 情境——不 mock 的話，parseAnsiToRenderedLines 自己的真實時序快到無法
// 穩定重現。
let resolveParse: (() => void) | null = null;
vi.mock("../lib/ansiBlockParser", () => ({
  parseAnsiToRenderedLines: vi.fn(
    () =>
      new Promise((resolve) => {
        resolveParse = () => resolve([{ spans: [{ text: "stubbed" }] }]);
      }),
  ),
}));

import { useTerminalBlocks } from "./useTerminalBlocks";

async function writeToTerm(term: Terminal, data: string) {
  await new Promise<void>((resolve) => term.write(data, resolve));
}

let term: Terminal;

beforeEach(() => {
  resolveParse = null;
  term = new Terminal({ cols: 80, rows: 24 });
});

afterEach(() => {
  term.dispose();
});

describe("clearAndRebasePromptEnd 遇到欄寬換行的複合情境", () => {
  it("delayed clear() 觸發時游標已經換行到接續行，不該把座標硬搬到第 0 行、切出錯誤內容", async () => {
    const { result } = renderHook(() => useTerminalBlocks("session-1", term));

    // 第一個指令：本機路徑建立一個區塊，走 finalizeBlock({clearOnParsed:
    // true}) 這條會呼叫 clearAndRebasePromptEnd 的既有路徑。
    act(() => {
      result.current.submitCommand("ls");
    });
    await act(async () => {
      await writeToTerm(term, "\x1b]133;D;0\x07");
    });
    expect(resolveParse).not.toBeNull();

    // 讓游標先往下移動幾行，確保接下來記錄的 B 落在非 0 的行號——這樣
    // 才能真正驗證「保持原本行號」跟「硬搬成 0」兩者會產生不同結果，
    // 不會因為 B 剛好本來就在第 0 行而讓測試失去意義。
    await act(async () => {
      await writeToTerm(term, "line1\r\nline2\r\nline3\r\n");
    });

    // 提示字元 13 字元 + B——這時候記錄在 row3, col13。
    const prompt = "user@host:~$ ";
    await act(async () => {
      await writeToTerm(term, prompt + "\x1b]133;B\x07");
    });

    // 打一個長度 90 的指令：row3 從 col13 到 col79 只有 67 欄可用，
    // 一定會自動換行到 row4——游標現在停在接續行，不是 B 記錄的 row3。
    // 這裡刻意「還沒」按 Enter，模擬使用者還在輸入時，前一個指令延遲
    // 已久的 clear() 才真正觸發。
    const longCommand = "a".repeat(90);
    await act(async () => {
      await writeToTerm(term, longCommand);
    });

    // 現在才讓第一個指令的非同步解析完成，觸發它延遲已久的
    // clearAndRebasePromptEnd——此時游標在 row4（接續行），不等於
    // promptEndRef 記錄的 row3，防呆條件不成立，不該搬遷座標。
    await act(async () => {
      resolveParse!();
      await Promise.resolve();
      await Promise.resolve();
    });

    // 使用者這才按下 Enter，完成輸入、觸發 C。
    await act(async () => {
      await writeToTerm(term, "\r\n\x1b]133;C\x07");
    });

    // 防呆邏輯正確運作的話：座標沒有被搬遷（還停在已經失效的 row3），
    // recoverUntrackedCommand 的 endRow < startRow 安全網會接手判斷
    // 失敗，不會建立第二個區塊——絕對不能是「建立了一個內容是從錯誤
    // 欄位切出來的第二個區塊」（例如把 clear() 保留下來的換行殘留字元
    // 誤判成指令文字）。
    expect(result.current.blocks).toHaveLength(1);
    expect(result.current.blocks[0].command).toBe("ls");
  });
});
