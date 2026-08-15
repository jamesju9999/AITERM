import { useLocale } from "../contexts/LocaleContext";
import "./AgentStatusBar.css";

export type AgentPhase =
  | { phase: "asking"; step: number; maxSteps: number }
  | { phase: "running"; step: number; maxSteps: number; command: string }
  | { phase: "web"; step: number; maxSteps: number; query: string; webKind: "search" | "fetch" }
  | { phase: "done"; steps: number }
  | { phase: "failed"; reason: string };

interface AgentStatusBarProps {
  status: AgentPhase;
  onDismiss: () => void;
  /** 本次 mission 累計 token；0 或未提供時不顯示。 */
  missionTokens?: number;
}

export function AgentStatusBar({ status, onDismiss, missionTokens }: AgentStatusBarProps) {
  const { t } = useLocale();

  let icon: string;
  let text: string;
  switch (status.phase) {
    case "asking":
      icon = "◐";
      text = t.term_agent_status_asking;
      break;
    case "running":
      icon = "▶";
      text = t.term_agent_status_running(status.command);
      break;
    case "web":
      icon = status.webKind === "search" ? "🔍" : "📄";
      text = status.webKind === "search"
        ? t.term_agent_status_web_search(status.query)
        : t.term_agent_status_web_fetch(status.query);
      break;
    case "done":
      icon = "✅";
      text = t.term_agent_status_done(status.steps);
      break;
    case "failed":
      icon = "⚠";
      text = t.term_agent_status_failed(status.reason);
      break;
  }

  const showStep = status.phase === "asking" || status.phase === "running" || status.phase === "web";
  const dismissible = status.phase === "done" || status.phase === "failed";
  const pulsing = status.phase === "asking" || status.phase === "running" || status.phase === "web";

  return (
    <div
      className={`aiterm-agent-status aiterm-agent-status--${status.phase}`}
      role="status"
      aria-live="polite"
    >
      <span
        className={`aiterm-agent-status__icon${pulsing ? " aiterm-agent-status__icon--pulse" : ""}`}
        aria-hidden="true"
      >
        {icon}
      </span>
      <span className="aiterm-agent-status__text">{text}</span>
      {showStep && (
        <span className="aiterm-agent-status__step">
          {t.term_agent_status_step(status.step, status.maxSteps)}
        </span>
      )}
      {missionTokens ? (
        <span className="agent-status-tokens" data-testid="mission-tokens">
          {missionTokens >= 1000
            ? `${(missionTokens / 1000).toFixed(1)}k`
            : String(missionTokens)}
        </span>
      ) : null}
      {dismissible && (
        <button
          type="button"
          className="aiterm-agent-status__dismiss"
          onClick={onDismiss}
          aria-label={t.term_agent_status_dismiss}
        >
          ✕
        </button>
      )}
    </div>
  );
}
