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

  // WarpInput 送出的 mission 有轉 Telegram，但企業任務那條目前沒有——
  // sendRemoteResponse 缺席時不能讓 onAgentProgress 也跟著不執行。
  it("沒有 sendRemoteResponse 時仍會呼叫 onAgentProgress，且不拋錯", () => {
    const onAgentProgress = vi.fn();
    expect(() => reportAgentStep(info, { onAgentProgress })).not.toThrow();
    expect(onAgentProgress).toHaveBeenCalledWith(2, 5);
  });

  // 反過來：onAgentProgress 缺席時（例如目前 WarpInput 的 onComplete 路徑
  // 沒有掛首頁進度）sendRemoteResponse 仍要照常轉發。
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
});
