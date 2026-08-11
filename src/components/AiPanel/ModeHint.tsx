import { useLocale } from "../../contexts/LocaleContext";

/** 三種模式的差別是「誰按下執行鍵」，見各自的說明文案。 */
export type PanelMode = "suggest" | "agent" | "mcp";

interface ModeHintProps {
  mode: PanelMode;
  /** 設定裡的 max_agent_steps；0（無限）在上層已存成 9999。 */
  maxAgentSteps: number;
  mcpToolCount: number;
}

const ICONS: Record<PanelMode, string> = {
  suggest: "💬",
  agent: "⚡",
  mcp: "🔧",
};

/**
 * 貼在輸入框上方的一行字，說明「現在送出會發生什麼」。
 *
 * 兩顆開關只表達得出自己開沒開，表達不出 Agent 會自己執行、也表達不出
 * Agent 開著時 MCP 是被忽略的——那件事只有這裡明講。
 */
export function ModeHint({ mode, maxAgentSteps, mcpToolCount }: ModeHintProps) {
  const { t } = useLocale();

  let text: string;
  if (mode === "agent") {
    // 顯示規則跟 Agent 狀態列一致：9999 是「無限」的內部值。
    text = t.mode_hint_agent(maxAgentSteps >= 9999 ? "∞" : String(maxAgentSteps));
  } else if (mode === "mcp") {
    text = t.mode_hint_mcp(mcpToolCount);
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
