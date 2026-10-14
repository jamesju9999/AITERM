import { useLocale } from "../../contexts/LocaleContext";

/** 「誰按下執行鍵、能用什麼工具」。agent 是遠端終端機（舊的 <cmd> 迴圈）用的；本機用 agent-terminal／agent-mcp。 */
export type PanelMode = "suggest" | "agent" | "agent-terminal" | "agent-mcp";

interface ModeHintProps {
  mode: PanelMode;
  /** 設定裡的 max_agent_steps；0（無限）在上層已存成 9999。 */
  maxAgentSteps: number;
  mcpToolCount: number;
}

const ICONS: Record<PanelMode, string> = {
  suggest: "💬",
  agent: "⚡",
  "agent-terminal": "⚡",
  "agent-mcp": "⚡",
};

/**
 * 貼在輸入框上方的一行字，說明「現在送出會發生什麼」。
 *
 * 兩顆開關只表達得出自己開沒開，表達不出 Agent 會自己執行、也表達不出
 * Agent 開著時 MCP 是被忽略的——那件事只有這裡明講。
 */
export function ModeHint({ mode, maxAgentSteps, mcpToolCount }: ModeHintProps) {
  const { t } = useLocale();

  // 顯示規則跟 Agent 狀態列一致：9999 是「無限」的內部值。
  const steps = maxAgentSteps >= 9999 ? "∞" : String(maxAgentSteps);
  let text: string;
  if (mode === "agent") {
    text = t.mode_hint_agent(steps);
  } else if (mode === "agent-terminal") {
    text = t.mode_hint_agent_terminal(steps);
  } else if (mode === "agent-mcp") {
    text = t.mode_hint_agent_mcp(steps, mcpToolCount);
  } else {
    text = t.mode_hint_suggest;
  }

  return (
    <div className={`aiterm-mode-hint aiterm-mode-hint--${mode}`}>
      <span aria-hidden="true">{ICONS[mode]}</span>
      <span>{text}</span>
    </div>
  );
}
