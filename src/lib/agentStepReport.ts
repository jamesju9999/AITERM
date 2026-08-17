/**
 * Agent 執行迴圈每完成一步（一個 shell 指令跑完）時，需要回報的資訊。
 * 由 runAgentLoop 內部組裝，往上傳給呼叫端決定要不要轉發 Telegram、
 * 更新首頁「進行中的任務」的進度等。
 */
export interface AgentStepInfo {
  /** 1-based step index for display（對應 AgentStatusBar「步驟 N/M」的計數）。 */
  stepIndex: number;
  maxSteps: number;
  command: string;
  exitCode: number;
  /** Already trimmed and length-capped（~2000 chars）by the agent loop. */
  output: string;
}

/** Format one agent step's command + output as a single Telegram message. */
export function formatAgentStepForRemote(info: AgentStepInfo): string {
  // xterm's translateToString already returns plain text, but defend
  // against stray escape codes from copy-pasted prompts etc.
  const cleaned = info.output.replace(/\x1b\[[0-9;?]*[a-zA-Z]/g, "").trim();
  const exitTag = info.exitCode === 0 ? "" : ` ⚠️ exit ${info.exitCode}`;
  const header = `[${info.stepIndex}/${info.maxSteps}] $ ${info.command}${exitTag}`;
  if (!cleaned) return header;

  // Telegram caps text messages at 4096 chars; reserve room for header + marker.
  const MAX = 3500;
  let body = cleaned;
  if (body.length > MAX) {
    const half = Math.floor(MAX / 2);
    body = `${body.slice(0, half)}\n... (truncated, ${body.length - MAX} chars omitted) ...\n${body.slice(-half)}`;
  }
  return `${header}\n${body}`;
}

/**
 * 一個 agent 步驟完成時該做的兩件事：轉發到 Telegram、回報進度給首頁的
 * 「進行中的任務」。兩個 callback 都是可選的、彼此獨立——任一個缺席都
 * 不影響另一個執行。
 */
export function reportAgentStep(
  info: AgentStepInfo,
  callbacks: {
    sendRemoteResponse?: (text: string) => void;
    onAgentProgress?: (done: number, total: number) => void;
  }
): void {
  callbacks.sendRemoteResponse?.(formatAgentStepForRemote(info));
  callbacks.onAgentProgress?.(info.stepIndex, info.maxSteps);
}
