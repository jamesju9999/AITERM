import type { MouseEvent as ReactMouseEvent } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import type { AgentRow } from "../../ipc/agents";
import type { TaskRow } from "../../ipc/tasks";
import type { AgentStatus } from "./deriveAgentStatus";
import { formatElapsed } from "./formatElapsed";

const CHIP: Record<string, { cls: string; icon: string }> = {
  queued: { cls: "ac-chip--queued", icon: "⏳" },
  running: { cls: "ac-chip--running", icon: "▶" },
  success: { cls: "ac-chip--ok", icon: "✓" },
  failed: { cls: "ac-chip--bad", icon: "✗" },
  cancelled: { cls: "ac-chip--bad", icon: "✗" },
};

function chipOf(card: TaskRow) {
  if (card.status === "done") return CHIP[card.outcome ?? "failed"] ?? CHIP.failed;
  return CHIP[card.status] ?? CHIP.queued;
}

export function AgentNode({
  agent,
  cards,
  status,
  nowSecs,
  dropState,
  mergeMessage,
  mergeConflict,
  onHeaderMouseDown,
  onChipMouseDown,
  onEdit,
  onDelete,
  onRetry,
  onSkip,
  onTranscript,
  onOpenTab,
  onMerge,
  onAbortMerge,
}: {
  agent: AgentRow;
  /** 這個 Agent 名下、未封存的卡片。 */
  cards: TaskRow[];
  status: AgentStatus;
  nowSecs: number;
  dropState: "ok" | "bad" | null;
  mergeMessage: string | null;
  mergeConflict: boolean;
  onHeaderMouseDown: (e: ReactMouseEvent) => void;
  onChipMouseDown: (e: ReactMouseEvent, taskId: string) => void;
  onEdit: () => void;
  onDelete: () => void;
  onRetry: () => void;
  onSkip: () => void;
  onTranscript: (taskId: string) => void;
  onOpenTab: (tabId: string) => void;
  onMerge: () => void;
  onAbortMerge: () => void;
}) {
  const { t } = useLocale();
  const running = cards.find((c) => c.status === "running");
  const queued = cards.filter((c) => c.status === "queued");
  const failed = agent.paused_task_id ? cards.find((c) => c.id === agent.paused_task_id) : undefined;
  const statusLabel = {
    attention: t.ac_status_attention,
    coding: t.ac_status_coding,
    waiting: t.ac_status_waiting,
    done: t.ac_status_done,
    idle: t.ac_status_idle,
  }[status];
  const doneCount = cards.filter((c) => c.status === "done" && c.outcome === "success").length;
  const canMerge = !!agent.branch && (status === "idle" || status === "done");

  // 已完成的卡片排在最後；排隊的照佇列順序。
  const chips = [...cards].sort((a, b) => {
    const rank = (c: TaskRow) => (c.status === "done" ? 2 : c.status === "running" ? 0 : 1);
    return rank(a) - rank(b) || a.sort_order - b.sort_order;
  });

  return (
    <div
      className={`ac-agent ac-agent--${status}${dropState ? ` ac-agent--drop-${dropState}` : ""}`}
      data-agent-id={agent.id}
      data-testid={`agent-${agent.id}`}
      style={{ left: agent.canvas_x, top: agent.canvas_y }}
    >
      <div className="ac-agent-head" onMouseDown={onHeaderMouseDown}>
        <span className="ac-agent-ico" aria-hidden>
          {"</>"}
        </span>
        <div className="ac-agent-name">
          <b>{agent.name}</b>
          <span>{agent.role}</span>
        </div>
        <span className="ac-badge">
          {status === "done" ? "✓ " : ""}
          {statusLabel}
        </span>
        <button className="ac-iconbtn" title={t.ac_settings} aria-label={t.ac_settings} onClick={onEdit}>
          ⚙
        </button>
        <button className="ac-iconbtn" title={t.ac_delete} aria-label={t.ac_delete} onClick={onDelete}>
          ×
        </button>
      </div>
      <div className="ac-agent-body">
        {failed ? (
          <div className="ac-attention" role="alert">
            <div className="ac-attention-title">{t.ac_failed_title}</div>
            <div className="ac-attention-card">✗ {failed.title}</div>
            {failed.error_message && <div className="ac-attention-msg">{failed.error_message}</div>}
            <div className="ac-attention-actions">
              <button className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" onClick={onRetry}>
                {t.ac_retry}
              </button>
              <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={onSkip}>
                {t.ac_skip}
              </button>
              <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={() => onTranscript(failed.id)}>
                {t.ac_transcript}
              </button>
            </div>
          </div>
        ) : running ? (
          <>
            <div className="ac-now">
              <span className="ac-now-title">
                ▶ <b>{running.title}</b>
              </span>
              {running.tab_id && (
                <button className="ac-link" onClick={() => onOpenTab(running.tab_id as string)}>
                  {t.ac_open_tab}
                </button>
              )}
            </div>
            <div className="ac-now-sub">
              {t.ac_elapsed(formatElapsed(nowSecs - (running.dispatched_at ?? nowSecs)))}
            </div>
            <div className="ac-bar ac-bar--indeterminate" aria-hidden>
              <i />
            </div>
          </>
        ) : (
          <div className="ac-now ac-now--muted">
            {status === "done" ? (
              <span>
                <span className="ac-check">✓</span> {t.ac_all_passed}
              </span>
            ) : queued.length ? (
              t.ac_waiting_slot
            ) : (
              t.ac_no_work
            )}
          </div>
        )}
        <div className="ac-chips">
          {chips.length === 0 && <span className="ac-dropzone">{t.ac_drop_here}</span>}
          {chips.map((c) => {
            const { cls, icon } = chipOf(c);
            const draggable = c.status === "queued";
            return (
              <span
                key={c.id}
                className={`ac-chip ${cls}${draggable ? " ac-chip--draggable" : ""}`}
                title={c.title}
                data-testid={`chip-${c.id}`}
                onMouseDown={draggable ? (e) => onChipMouseDown(e, c.id) : undefined}
                onClick={c.status === "done" ? () => onTranscript(c.id) : undefined}
              >
                {icon} {c.title}
              </span>
            );
          })}
        </div>
        <div className="ac-meta">
          <span>⑂ {agent.branch ?? "—"}</span>
          <span>
            {doneCount}/{cards.length}
          </span>
        </div>
        {canMerge && (
          <div className="ac-merge">
            <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={onMerge}>
              {t.ac_merge}
            </button>
            {mergeConflict && (
              <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={onAbortMerge}>
                {t.ac_abort_merge}
              </button>
            )}
          </div>
        )}
        {mergeMessage && <div className={`ac-merge-msg${mergeConflict ? " ac-merge-msg--bad" : ""}`}>{mergeMessage}</div>}
      </div>
    </div>
  );
}
