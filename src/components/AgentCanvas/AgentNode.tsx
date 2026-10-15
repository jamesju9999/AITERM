import type { MouseEvent as ReactMouseEvent } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import type { AgentRow } from "../../ipc/agents";
import type { TaskRow } from "../../ipc/tasks";
import type { AgentStatus } from "./deriveAgentStatus";
import { deriveModuleState, type ModuleState } from "./deriveModuleState";
import { formatElapsed } from "./formatElapsed";

const KIND_ICON: Record<AgentRow["kind"], string> = { coder: "</>", reviewer: "✓?", orchestrator: "⑂" };

const MODULE_CHIP: Record<ModuleState, { cls: string; icon: string }> = {
  planning: { cls: "ac-chip--queued", icon: "·" },
  queued: { cls: "ac-chip--queued", icon: "⏳" },
  coding: { cls: "ac-chip--running", icon: "▶" },
  fix_queued: { cls: "ac-chip--fix", icon: "↺" },
  awaiting_review: { cls: "ac-chip--review", icon: "⧗" },
  reviewing: { cls: "ac-chip--review", icon: "🔍" },
  passed: { cls: "ac-chip--ok", icon: "✓" },
  attention: { cls: "ac-chip--bad ac-chip--release", icon: "⚠" },
  failed: { cls: "ac-chip--bad", icon: "✗" },
  review_failed: { cls: "ac-chip--bad ac-chip--release", icon: "⚠" },
  dispatching: { cls: "ac-chip--plan", icon: "🧭" },
  dispatch_failed: { cls: "ac-chip--bad", icon: "⚠" },
};

const shortSha = (s: string | null) => (s ? s.slice(0, 7) : "?");

export function AgentNode({
  agent,
  tasks,
  agentName,
  coderIds,
  hasReviewer,
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
  onRelease,
}: {
  agent: AgentRow;
  /** 這個專案所有未封存的卡片（節點自己挑出跟它有關的）。 */
  tasks: TaskRow[];
  agentName: (id: string | null) => string;
  /** 專案裡所有 Coder 的 id——Orchestrator 用它找出全部模組。 */
  coderIds: Set<string>;
  hasReviewer: boolean;
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
  onRelease: (moduleId: string) => void;
}) {
  const { t } = useLocale();
  const mine = tasks.filter((c) => c.agent_id === agent.id);
  const running = mine.find((c) => c.status === "running");
  const queued = mine.filter((c) => c.status === "queued");
  const failed = agent.paused_task_id ? mine.find((c) => c.id === agent.paused_task_id) : undefined;
  const byId = (id: string | null) => tasks.find((c) => c.id === id);
  const statusLabel = {
    attention: t.ac_status_attention,
    coding: t.ac_status_coding,
    reviewing: t.ac_status_reviewing,
    merging: t.ac_status_merging,
    waiting: t.ac_status_waiting,
    done: t.ac_status_done,
    idle: t.ac_status_idle,
  }[status];
  const msLabel: Record<ModuleState, string> = {
    planning: t.ac_ms_planning,
    queued: t.ac_ms_queued,
    coding: t.ac_ms_coding,
    fix_queued: t.ac_ms_fix_queued,
    awaiting_review: t.ac_ms_awaiting_review,
    reviewing: t.ac_ms_reviewing,
    passed: t.ac_ms_passed,
    attention: t.ac_ms_attention,
    failed: t.ac_ms_failed,
    review_failed: t.ac_ms_review_failed,
    dispatching: t.ac_ms_dispatching,
    dispatch_failed: t.ac_ms_dispatch_failed,
  };
  const canMerge = agent.kind !== "reviewer" && !!agent.branch && (status === "idle" || status === "done");

  // ── Coder：每個模組一顆 chip，狀態沿著審查鏈推導 ─────────────────────────
  const modules = agent.kind === "coder" ? mine.filter((c) => c.card_kind === "work") : [];
  const moduleStates = modules.map((m) => ({ m, s: deriveModuleState(m, tasks, hasReviewer) }));
  const rank = (s: ModuleState) =>
    ({
      coding: 0, attention: 1, review_failed: 1, failed: 1, dispatch_failed: 1, fix_queued: 2,
      reviewing: 3, awaiting_review: 3, dispatching: 3, queued: 4, planning: 4, passed: 5,
    })[s];
  moduleStates.sort((a, b) => rank(a.s) - rank(b.s) || a.m.sort_order - b.m.sort_order);
  const passedCount = moduleStates.filter((x) => x.s === "passed").length;

  // ── Orchestrator：所有 Coder 的模組 ───────────────────────────────────────
  const allModules = tasks.filter((c) => c.card_kind === "work" && c.agent_id && coderIds.has(c.agent_id));
  const allStates = agent.kind === "orchestrator" ? allModules.map((m) => deriveModuleState(m, tasks, hasReviewer)) : [];
  const allPassed = allStates.filter((s) => s === "passed").length;
  const allAttention = allStates.filter((s) => s === "attention" || s === "review_failed").length;
  const lastMerge = agent.kind === "orchestrator"
    ? [...mine].filter((c) => c.card_kind === "merge").sort((a, b) => (b.finished_at ?? 0) - (a.finished_at ?? 0))[0]
    : undefined;

  const runningLine = () => {
    if (!running) return null;
    let title = running.title;
    let snip: string | null = null;
    if (agent.kind === "reviewer") {
      const module = byId(running.module_id);
      title = t.ac_reviewing_module(module?.title ?? running.title, agentName(module?.agent_id ?? null));
      snip = `git diff ${shortSha(running.start_sha)}..${shortSha(running.end_sha)}`;
    } else if (agent.kind === "orchestrator") {
      title = running.card_kind === "plan"
        ? t.ac_splitting_spec(byId(running.module_id)?.title ?? running.title)
        : t.ac_merge_running;
    }
    return (
      <>
        <div className="ac-now">
          <span className="ac-now-title">
            {agent.kind === "reviewer" ? "🔍" : agent.kind === "orchestrator" ? "⑂" : running.card_kind === "fix" ? "↺" : "▶"}{" "}
            <b>{title}</b>
          </span>
          {running.tab_id && (
            <button className="ac-link" onClick={() => onOpenTab(running.tab_id as string)}>
              {t.ac_open_tab}
            </button>
          )}
        </div>
        {snip && <div className="ac-snip">{snip}</div>}
        <div className="ac-now-sub">{t.ac_elapsed(formatElapsed(nowSecs - (running.dispatched_at ?? nowSecs)))}</div>
        <div className="ac-bar ac-bar--indeterminate" aria-hidden>
          <i />
        </div>
      </>
    );
  };

  const idleLine = () => {
    if (agent.kind === "coder") {
      if (status === "done") return <span><span className="ac-check">✓</span> {t.ac_all_passed}</span>;
      if (queued.length) return t.ac_waiting_slot;
      if (moduleStates.some(({ s }) => s === "awaiting_review" || s === "reviewing")) return t.ac_ms_awaiting_review;
      if (moduleStates.some(({ s }) => s === "attention" || s === "review_failed")) {
        return <span className="ac-bad-text">{t.ac_release_hint}</span>;
      }
      return t.ac_no_work;
    }
    if (agent.kind === "reviewer") return queued.length ? t.ac_review_queue(queued.length) : t.ac_waiting_handoff;
    if (lastMerge && lastMerge.outcome === "success" && allPassed === allStates.length && queued.length === 0) {
      return <span><span className="ac-check">✓</span> {t.ac_integrated}</span>;
    }
    return (
      <span>
        {t.ac_merge_waiting(allPassed, allStates.length)}
        {allAttention > 0 && <span className="ac-bad-text"> · {t.ac_attention_count(allAttention)}</span>}
      </span>
    );
  };

  return (
    <div
      className={`ac-agent ac-agent--${agent.kind} ac-agent--${status}${dropState ? ` ac-agent--drop-${dropState}` : ""}`}
      data-agent-id={agent.id}
      data-testid={`agent-${agent.id}`}
      style={{ left: agent.canvas_x, top: agent.canvas_y }}
    >
      <div className="ac-agent-head" onMouseDown={onHeaderMouseDown}>
        <span className="ac-agent-ico" aria-hidden>
          {KIND_ICON[agent.kind]}
        </span>
        <div className="ac-agent-name">
          <b>
            {agent.produces_spec && (
              <span className="ac-spec-mark" title={t.ac_spec_badge} aria-label={t.ac_spec_badge}>
                📐{" "}
              </span>
            )}
            {agent.name}
          </b>
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
          runningLine()
        ) : (
          <div className="ac-now ac-now--muted">{idleLine()}</div>
        )}

        {agent.kind === "coder" && (
          <div className="ac-chips">
            {moduleStates.length === 0 && <span className="ac-dropzone">{t.ac_drop_here}</span>}
            {moduleStates.map(({ m, s }) => {
              const { cls, icon } = MODULE_CHIP[s];
              const draggable = s === "queued";
              const releasable = s === "attention" || s === "review_failed";
              return (
                <span
                  key={m.id}
                  className={`ac-chip ${cls}${draggable ? " ac-chip--draggable" : ""}`}
                  title={`${m.title} · ${msLabel[s]}`}
                  data-testid={`chip-${m.id}`}
                  data-state={s}
                  onMouseDown={draggable ? (e) => onChipMouseDown(e, m.id) : undefined}
                  onClick={releasable ? () => onRelease(m.id) : m.status === "done" ? () => onTranscript(m.id) : undefined}
                >
                  {icon} {m.title}
                </span>
              );
            })}
          </div>
        )}
        {agent.kind === "reviewer" && queued.length > 0 && (
          <div className="ac-chips">
            {queued.map((c) => (
              <span key={c.id} className="ac-chip ac-chip--review" title={c.title}>
                ⧗ {byId(c.module_id)?.title ?? c.title}
              </span>
            ))}
          </div>
        )}

        <div className="ac-meta">
          <span>⑂ {agent.kind === "reviewer" ? "—" : (agent.branch ?? "—")}</span>
          {agent.kind === "coder" && (
            <span>
              {passedCount}/{moduleStates.length}
            </span>
          )}
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
