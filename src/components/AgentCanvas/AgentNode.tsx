import type { MouseEvent as ReactMouseEvent } from "react";
import { RoleIcon } from "./RoleIcon";
import { roleIcon } from "./roles";

import { useLocale } from "../../contexts/LocaleContext";
import type { AgentRow } from "../../ipc/agents";
import type { TaskRow, TokenUsage } from "../../ipc/tasks";
import type { AgentStatus } from "./deriveAgentStatus";
import { deriveModuleState, type ModuleState } from "./deriveModuleState";
import { localizedCardTitle } from "./cardTitle";
import { formatElapsed } from "./formatElapsed";
import { waitingPrerequisites } from "./prerequisites";
import { shortBranch } from "./projectSummary";
import { formatTokens, moduleCards, sumUsage, totalTokens, usageBreakdown, type UsageSum } from "./tokenUsage";


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

/** 審查／驗收卡的 review_reasons（JSON 字串陣列）。 */
const reasonsOf = (c: TaskRow): string[] => {
  try {
    const v: unknown = JSON.parse(c.review_reasons ?? "[]");
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
};

export function AgentNode({
  agent,
  tasks,
  agentName,
  coderIds,
  hasReviewer,
  hasOrchestrator,
  hasVerifier = false,
  missingTool = null,
  dbSkill = null,
  onDbSkill,
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
  onMarkDone,
  question,
  onOpenQuestion,
  usage = {},
  onVerifyReport,
  onAcceptVerify,
}: {
  agent: AgentRow;
  /** 這個專案所有未封存的卡片（節點自己挑出跟它有關的）。 */
  tasks: TaskRow[];
  agentName: (id: string | null) => string;
  /** 專案裡所有 Coder 的 id——Orchestrator 用它找出全部模組。 */
  coderIds: Set<string>;
  hasReviewer: boolean;
  /** 有協調者時，工程師的分支由它合併，工程師卡上就不放「合併分支」。 */
  hasOrchestrator: boolean;
  /** 有驗收者時，合併結果要驗收通過才更新程式庫，協調者卡上不放「合併分支」。 */
  hasVerifier?: boolean;
  /** 這個 Agent 用的 AI 工具沒裝（顯示名稱，例如「Claude Code」）；有裝或還不知道是 null。 */
  missingTool?: string | null;
  /** 資料庫技能：標籤文字，與正在跑的 SQL 開頭（沒有在查就是 null）。沒裝是 null。 */
  dbSkill?: { mode: string; label: string; active: string | null } | null;
  onDbSkill?: () => void;
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
  /** 執行中的互動卡：手動標記完成（互動卡不會被完成標記結案）。 */
  onMarkDone: (taskId: string) => void;
  /** 這個 Agent 正在等人回答的問題（ask_human）。 */
  question?: { id: number; question: string } | null;
  onOpenQuestion?: (id: number) => void;
  /** 每張卡的 token 用量（task id → 用量）。 */
  usage?: Record<string, TokenUsage>;
  /** 驗收者：打開某張驗收卡的報告。 */
  onVerifyReport?: (taskId: string) => void;
  /** 驗收者：放行沒通過的驗收。 */
  onAcceptVerify?: (taskId: string) => void;
}) {
  const { t } = useLocale();
  const icon = roleIcon(agent.kind, agent.role) ?? agent.kind;
  const mine = tasks.filter((c) => c.agent_id === agent.id);
  const running = mine.find((c) => c.status === "running");
  const queued = mine.filter((c) => c.status === "queued");
  const mineUsage = sumUsage(mine, usage);
  const tokensText = (s: UsageSum) => t.ac_tokens(formatTokens(totalTokens(s.usage)));
  const failed = agent.paused_task_id ? mine.find((c) => c.id === agent.paused_task_id) : undefined;
  const byId = (id: string | null) => tasks.find((c) => c.id === id);
  const statusLabel = {
    attention: t.ac_status_attention,
    coding: t.ac_status_coding,
    reviewing: t.ac_status_reviewing,
    // 同樣是 Orchestrator 在跑，拆解規格與合併分支是兩回事。
    merging: running?.card_kind === "plan" ? t.ac_status_splitting : t.ac_status_merging,
    verifying: t.ac_status_verifying,
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
  const allStates =
    agent.kind === "orchestrator" || agent.kind === "reviewer" || agent.kind === "verifier"
      ? allModules.map((m) => deriveModuleState(m, tasks, hasReviewer))
      : [];
  const allPassed = allStates.filter((s) => s === "passed").length;
  const allAttention = allStates.filter((s) => s === "attention" || s === "review_failed").length;
  const lastMerge = agent.kind === "orchestrator"
    ? [...mine].filter((c) => c.card_kind === "merge").sort((a, b) => (b.finished_at ?? 0) - (a.finished_at ?? 0))[0]
    : undefined;
  // 已經合回程式庫就不用再按；手動合併的結果訊息還在顯示時保留按鈕（可能要還原衝突）。
  const canMerge =
    !!agent.branch &&
    (status === "idle" || status === "done") &&
    (agent.kind === "orchestrator"
      ? (lastMerge?.verdict !== "repo_merged" && !(hasVerifier && !lastMerge?.verdict)) || mergeConflict
      : agent.kind === "coder" && !hasOrchestrator);

  // ── Verifier：最後一張驗收卡 ─────────────────────────────────────────────
  const lastVerify =
    agent.kind === "verifier"
      ? mine.filter((c) => c.card_kind === "verify" && c.status === "done").sort((a, b) => (b.finished_at ?? 0) - (a.finished_at ?? 0))[0]
      : undefined;

  const runningLine = () => {
    if (!running) return null;
    let title = localizedCardTitle(running, t);
    let snip: string | null = null;
    if (agent.kind === "reviewer") {
      const module = byId(running.module_id);
      title = t.ac_reviewing_module(module?.title ?? running.title, agentName(module?.agent_id ?? null));
      snip = `git diff ${shortSha(running.start_sha)}..${shortSha(running.end_sha)}`;
    } else if (agent.kind === "verifier") {
      title = t.ac_verify_running;
    } else if (agent.kind === "orchestrator") {
      title = running.card_kind === "plan"
        ? t.ac_splitting_spec(byId(running.module_id)?.title ?? running.title)
        : t.ac_merge_running;
    }
    return (
      <>
        <div className="ac-now">
          <span className="ac-now-title">
            {agent.kind === "reviewer"
              ? "🔍"
              : agent.kind === "orchestrator"
                ? "⑂"
                : agent.kind === "verifier"
                  ? "☑"
                  : running.card_kind === "fix"
                    ? "↺"
                    : "▶"}{" "}
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
        {question ? (
          <button className="ac-reply-wait ac-ask" onClick={() => onOpenQuestion?.(question.id)}>
            <span>{t.ac_waiting_decision(question.question)}</span>
          </button>
        ) : running.quota_wait_at ? (
          <div className="ac-quota-wait">{t.board_quota_wait_hint}</div>
        ) : running.interactive ? (
          <div className="ac-reply-wait">
            <span>{t.ac_interactive_hint}</span>
            <button className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" onClick={() => onMarkDone(running.id)}>
              {t.ac_mark_done}
            </button>
          </div>
        ) : running.reply_wait_at ? (
          <div className="ac-reply-wait" role="status">
            {t.ac_reply_wait}
          </div>
        ) : (
          <div className="ac-bar ac-bar--indeterminate" aria-hidden>
            <i />
          </div>
        )}
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
    if (agent.kind === "reviewer") {
      if (queued.length) return t.ac_review_queue(queued.length);
      // 每個模組都審過了就不是「等交件」。
      if (allStates.length > 0 && allPassed === allStates.length) {
        return (
          <span>
            <span className="ac-check">✓</span> {t.ac_review_all_done}
          </span>
        );
      }
      return t.ac_waiting_handoff;
    }
    if (agent.kind === "verifier") {
      if (queued.length) return t.ac_verify_queue;
      if (!lastVerify) return t.ac_verify_waiting;
      const report = (
        <button className="ac-link" onClick={() => onVerifyReport?.(lastVerify.id)}>
          {t.ac_verify_report}
        </button>
      );
      if (lastVerify.verdict === "pass") {
        return (
          <span>
            <span className="ac-check">✓</span> {t.ac_verify_passed} · {report}
          </span>
        );
      }
      if (lastVerify.verdict === "attention") {
        return (
          <span>
            <span className="ac-bad-text">⚠ {t.ac_verify_attention}</span> · {report}{" "}
            <button className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" onClick={() => onAcceptVerify?.(lastVerify.id)}>
              {t.ac_verify_accept}
            </button>
          </span>
        );
      }
      // 退回後模組都又通過了 = 修正做完，在等協調者重新合併；還有沒通過的才是「修正中」。
      const n = reasonsOf(lastVerify).length;
      const fixed = allStates.length > 0 && allPassed === allStates.length;
      return (
        <span>
          <span className="ac-bad-text">✗ {fixed ? t.ac_verify_fixed(n) : t.ac_verify_failed(n)}</span> · {report}
        </span>
      );
    }
    if (lastMerge && lastMerge.outcome === "success" && allPassed === allStates.length && queued.length === 0) {
      // verdict：合併卡結束後有沒有自動合回程式庫（見後端 workflow::fast_forward_repo）。
      const why = ({
        repo_dirty: t.ac_repo_why_dirty,
        repo_merge_in_progress: t.ac_repo_why_in_progress,
        repo_diverged: t.ac_repo_why_diverged,
      } as Record<string, string>)[lastMerge.verdict ?? ""];
      if (lastMerge.verdict === "repo_merged") return <span><span className="ac-check">✓</span> {t.ac_in_repo}</span>;
      if (hasVerifier && !lastMerge.verdict) return t.ac_wait_verify;
      if (why) return <span className="ac-bad-text">{t.ac_repo_pending(why)}</span>;
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
      data-guide-agent={agent.id}
      data-agent-id={agent.id}
      data-testid={`agent-${agent.id}`}
      style={{ left: agent.canvas_x, top: agent.canvas_y }}
    >
      <div className="ac-agent-head" onMouseDown={onHeaderMouseDown}>
        <span className="ac-agent-ico" aria-hidden>
          <RoleIcon name={icon} />
        </span>
        <div className="ac-agent-name">
          <b>
            {agent.produces_spec && (
              <span className="ac-spec-mark" title={t.ac_spec_badge} aria-label={t.ac_spec_badge}>
                <RoleIcon name="spec" size={12} />{" "}
              </span>
            )}
            {agent.name}
          </b>
          {/* 角色名太長時只截角色，模型永遠看得到；完整角色名在提示裡。 */}
          <span className="ac-agent-sub" title={agent.model ? `${agent.role} · ${agent.model}` : agent.role}>
            <span className="ac-agent-role">{agent.role}</span>
            {agent.cli === "codex" ? (
              <span className="ac-model ac-model--codex">{agent.model ? `Codex · ${agent.model}` : "Codex"}</span>
            ) : (
              agent.model && <span className="ac-model">{agent.model}</span>
            )}
          </span>
          {missingTool && (
            <span className="ac-tool-missing" data-testid="agent-tool-missing">
              {t.ac_tool_missing_badge(missingTool)}
            </span>
          )}
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
      {/* 技能列：自己一整列（標題那欄太窄，連線名稱會被截掉）。查詢中整列亮起來，第二行顯示 SQL 開頭。 */}
      {dbSkill && (
        <button
          type="button"
          className={`ac-skill-row${dbSkill.active ? " ac-skill-row--active" : ""}`}
          data-testid="agent-db-skill"
          title={`${t.ac_db_chip_tip}\n${dbSkill.label}（${dbSkill.mode}）`}
          onMouseDown={(e) => e.stopPropagation()}
          onClick={onDbSkill}
        >
          <span className="ac-skill-row-line">
            <span aria-hidden>🗄</span>
            {/* 權限放前面：連線多時被截掉的是名稱，不是權限。 */}
            <span className="ac-skill-row-mode">{dbSkill.mode}</span>
            <span className="ac-skill-row-label">{dbSkill.label}</span>
          </span>
          {dbSkill.active && <span className="ac-db-sql">{dbSkill.active}</span>}
        </button>
      )}
      <div className="ac-agent-body">
        {failed ? (
          <div className="ac-attention" role="alert">
            <div className="ac-attention-title">{t.ac_failed_title}</div>
            <div className="ac-attention-card">✗ {localizedCardTitle(failed, t)}</div>
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
              const { cls, icon: stateIcon } = MODULE_CHIP[s];
              const draggable = s === "queued";
              const releasable = s === "attention" || s === "review_failed";
              // 排隊中但還在等別位工程師的前置審過：排程器會先跳過它。
              const waiting = waitingPrerequisites(m, tasks, hasReviewer);
              const icon = waiting.length ? "⛓" : stateIcon;
              const label = waiting.length ? t.ac_waiting_prereqs(waiting.map((d) => d.title).join("、")) : msLabel[s];
              const mUsage = sumUsage(moduleCards(m.id, tasks), usage);
              return (
                <span
                  key={m.id}
                  className={`ac-chip ${cls}${draggable ? " ac-chip--draggable" : ""}${waiting.length ? " ac-chip--waiting" : ""}`}
                  title={`${m.title} · ${label}${mUsage?.cards ? ` · ${tokensText(mUsage)}` : ""}${m.provides ? `\n${t.ac_provides(m.provides)}` : ""}`}
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
              <span key={c.id} className="ac-chip ac-chip--review" title={localizedCardTitle(c, t)}>
                ⧗ {byId(c.module_id)?.title ?? c.title}
              </span>
            ))}
          </div>
        )}

        {(agent.kind === "coder" || agent.kind === "orchestrator" || !!mineUsage?.cards) && (
          <div className="ac-meta">
            {/* 工程師顯示進度，分支名放到提示裡；協調者的分支就是要合回程式庫的那條，留在卡片上。 */}
            {agent.kind === "coder" ? (
              <span className="ac-progress" title={agent.branch ?? undefined}>
                {t.ac_modules_progress(passedCount, moduleStates.length)}
              </span>
            ) : agent.kind === "orchestrator" ? (
              <span className="ac-branch" title={agent.branch ?? undefined}>⑂ {agent.branch ? shortBranch(agent.branch) : "—"}</span>
            ) : (
              <span />
            )}
            {/* 一張都沒有對話記錄（例如早期的卡片）就不顯示，免得只剩一個「–」。 */}
            {!!mineUsage?.cards && (
              <span className="ac-tokens" data-testid={`tokens-${agent.id}`} title={t.ac_tokens_tip(usageBreakdown(mineUsage))}>
                {tokensText(mineUsage)}
              </span>
            )}
          </div>
        )}
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
