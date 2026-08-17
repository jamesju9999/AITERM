import { describe, expect, it, vi } from "vitest";
import { reportAgentStep, formatAgentStepForRemote, type AgentStepInfo } from "./agentStepReport";

const info: AgentStepInfo = {
  stepIndex: 2,
  maxSteps: 5,
  command: "ls -la",
  exitCode: 0,
  output: "total 0",
};

describe("reportAgentStep", () => {
  it("兩個 callback 都有掛時，兩者都會被呼叫且參數正確", () => {
    const sendRemoteResponse = vi.fn();
    const onAgentProgress = vi.fn();
    reportAgentStep(info, { sendRemoteResponse, onAgentProgress });
    expect(sendRemoteResponse).toHaveBeenCalledWith(formatAgentStepForRemote(info));
    expect(onAgentProgress).toHaveBeenCalledWith(2, 5);
  });

  // 兩個 callback 彼此獨立：sendRemoteResponse 缺席不能讓 onAgentProgress
  // 也跟著不執行。
  it("沒有 sendRemoteResponse 時仍會呼叫 onAgentProgress，且不拋錯", () => {
    const onAgentProgress = vi.fn();
    expect(() => reportAgentStep(info, { onAgentProgress })).not.toThrow();
    expect(onAgentProgress).toHaveBeenCalledWith(2, 5);
  });

  // 反過來：onAgentProgress 缺席時 sendRemoteResponse 仍要照常轉發。
  it("沒有 onAgentProgress 時仍會呼叫 sendRemoteResponse，且不拋錯", () => {
    const sendRemoteResponse = vi.fn();
    expect(() => reportAgentStep(info, { sendRemoteResponse })).not.toThrow();
    expect(sendRemoteResponse).toHaveBeenCalledWith(formatAgentStepForRemote(info));
  });

  it("兩個 callback 都不掛也不拋錯", () => {
    expect(() => reportAgentStep(info, {})).not.toThrow();
  });
});

describe("formatAgentStepForRemote", () => {
  it("組出 [步驟/總數] $ 指令 的標頭，成功時不帶 exit code 標記", () => {
    expect(formatAgentStepForRemote(info)).toBe("[2/5] $ ls -la\ntotal 0");
  });

  it("失敗時在標頭附上 exit code", () => {
    const failed: AgentStepInfo = { ...info, exitCode: 1, output: "" };
    expect(formatAgentStepForRemote(failed)).toBe("[2/5] $ ls -la ⚠️ exit 1");
  });

  // xterm 的 translateToString 理應已經是純文字，但這裡仍會防禦性剝掉殘留的
  // ANSI 跳脫碼（例如複製貼上的 prompt）——這是這個函式唯二的非平凡邏輯之一。
  it("剝掉輸出裡的 ANSI 跳脫碼", () => {
    const withEscapes: AgentStepInfo = { ...info, output: "\x1b[32mHello\x1b[0m World" };
    expect(formatAgentStepForRemote(withEscapes)).toBe("[2/5] $ ls -la\nHello World");
  });

  // Telegram 訊息上限 4096 字，超過 3500 字要從中間截斷並標註省略字數。
  // 用可分辨頭尾的字元組出輸入，確認前後各取的是正確那一半、中段確實被
  // 換成省略字數標記，不是隨便剪一刀。
  it("超過 3500 字時從中間截斷，保留頭尾各一半並標註省略字數", () => {
    const head = "H".repeat(1750);
    const middle = "M".repeat(1000);
    const tail = "T".repeat(1750);
    const long: AgentStepInfo = { ...info, output: head + middle + tail };
    // 總長 4500，超出 MAX(3500) 正好 1000 字；MAX/2 = 1750 正好對齊 head/tail 長度。
    const expected = `[2/5] $ ls -la\n${head}\n... (truncated, 1000 chars omitted) ...\n${tail}`;
    expect(formatAgentStepForRemote(long)).toBe(expected);
  });
});
