import type { AgentMission } from "../../hooks/useAgentMission";
import type { AgentPhase } from "../AgentStatusBar";
import type { PreviewState } from "../../lib/agentLoop";
import { CommandPreview } from "../CommandPreview";
import { WarpInput } from "../WarpInput";
import type { Translations } from "../../lib/i18n";
import "./AgentPanel.css";

interface Props {
  mission: AgentMission | null;
  phase: AgentPhase | null;
  streamText: string;
  preview: PreviewState;
  onSubmitGoal: (goal: string) => void;
  onStop: () => void;
  onConfirmPreview: () => void;
  onCancelPreview: () => void;
  onClose: () => void;
  disabled: boolean;
  t: Translations;
}

export function AgentPanel({
  mission, phase, streamText, preview,
  onSubmitGoal, onStop, onConfirmPreview, onCancelPreview, onClose, disabled, t,
}: Props) {
  const active = mission?.active ?? false;

  return (
    <div className="aiterm-remote-agent-panel" role="dialog" aria-label={t.remote_agent_panel_title}>
      <div className="aiterm-remote-agent-panel__header">
        <span className="aiterm-remote-agent-panel__title">{t.remote_agent_panel_title}</span>
        <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={onClose}>
          ✕
        </button>
      </div>

      <div className="aiterm-remote-agent-panel__body">
        {phase?.phase === "asking" && (
          <div className="aiterm-remote-agent-panel__status">◐ {t.term_agent_status_asking}</div>
        )}
        {phase?.phase === "running" && (
          <div className="aiterm-remote-agent-panel__status">
            ▶ {t.term_agent_status_running(phase.command)}
          </div>
        )}
        {phase?.phase === "web" && (
          <div className="aiterm-remote-agent-panel__status">
            {phase.webKind === "search"
              ? t.term_agent_status_web_search(phase.query)
              : t.term_agent_status_web_fetch(phase.query)}
          </div>
        )}
        {streamText && phase?.phase === "asking" && (
          <pre className="aiterm-remote-agent-panel__stream">{streamText}</pre>
        )}

        {phase?.phase === "done" && (
          <div className="aiterm-remote-agent-panel__done">✅ {t.remote_agent_done}</div>
        )}
        {phase?.phase === "failed" && (
          <div className="aiterm-remote-agent-panel__failed">⚠ {phase.reason}</div>
        )}
      </div>

      {preview.visible && (
        <CommandPreview
          command={preview.command}
          explanation={preview.explanation}
          riskLevel={preview.riskLevel}
          onConfirm={onConfirmPreview}
          onCancel={onCancelPreview}
        />
      )}

      <div className="aiterm-remote-agent-panel__footer">
        {active ? (
          <button className="aiterm-btn aiterm-btn--danger" onClick={onStop}>
            <span aria-hidden="true">■ </span>
            {t.remote_agent_stop}
          </button>
        ) : (
          <WarpInput
            onSubmit={onSubmitGoal}
            disabled={disabled}
            placeholder={t.remote_agent_goal_placeholder}
          />
        )}
      </div>
    </div>
  );
}
