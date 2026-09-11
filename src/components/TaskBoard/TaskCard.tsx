import { useState, type CSSProperties } from "react";
import { confirm, message } from "@tauri-apps/plugin-dialog";

import { useLocale } from "../../contexts/LocaleContext";
import { archiveTask, cloneTask, deleteTask, markTaskDone, mergeTaskWorktree, stopTask, type TaskWithAttachments } from "../../ipc/tasks";
import { hashLabelHue } from "./labelColor";

/**
 * Tauri `invoke()` reject 丟回來的不一定是字串——Rust 端序列化的錯誤物件用
 * `String(e)` 只會印出 `[object Object]`，這個 repo 踩過。這裡把三種形狀都
 * 攤成看得懂的文字。
 */
function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return JSON.stringify(e);
}

export function TaskCard({
  projectId,
  card,
  onEdit,
  onViewTranscript,
  onEditLabel,
  onChanged,
}: {
  projectId: string;
  card: TaskWithAttachments;
  onEdit: () => void;
  onViewTranscript: () => void;
  onEditLabel: () => void;
  onChanged: () => void;
}) {
  const { t } = useLocale();
  const [busy, setBusy] = useState(false);

  const run = async (fn: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await fn();
      onChanged();
    } catch (e) {
      // 後端的拒絕一定要講出來。呼叫端全部是 `void run(...)`，讓錯誤逃出去
      // 等於整個被丟掉，使用者看到的就是「按了完全沒反應」——實機上就是這樣
      // 回報「合併回原分支按了沒用」的，而後端其實有回 git 的 stderr（見
      // vcs/git.rs 的 git()）。同樣的原則 ProjectList.tsx 早就寫過了。
      await message(errorText(e), { title: card.title, kind: "error" });
    } finally {
      setBusy(false);
    }
  };

  // Not window.confirm: Tauri's webview has no real implementation of it
  // (see NotebookSidebar.tsx's own comment on the same pitfall) — must use
  // @tauri-apps/plugin-dialog's async confirm() instead, a real native
  // dialog.
  const remove = async () => {
    if (!(await confirm(t.board_delete_confirm))) return;
    const closeTab = card.tab_id ? await confirm(t.board_delete_close_tab) : false;
    await run(() => deleteTask(projectId, card.id, closeTab));
    // deleteTask's close_tab flag only kills the PTY session on the
    // backend — TerminalApp is the only place that removes a tab from its
    // own tab list (see its aiterm:close-tab listener), so without this
    // dispatch the tab visually stays open (now attached to a dead
    // session) even though the user explicitly confirmed closing it.
    if (closeTab && card.tab_id) {
      window.dispatchEvent(new CustomEvent("aiterm:close-tab", { detail: { tabId: card.tab_id } }));
    }
  };

  const openTab = () => {
    if (!card.tab_id) return;
    window.dispatchEvent(new CustomEvent("aiterm:focus-tab", { detail: { tabId: card.tab_id } }));
  };

  const markDone = async () => {
    if (!(await confirm(t.board_mark_done_confirm))) return;
    await run(() => markTaskDone(projectId, card.id));
  };

  const outcomeLabel =
    card.outcome === "success"
      ? t.board_outcome_success
      : card.outcome === "failed"
        ? t.board_outcome_failed
        : card.outcome === "cancelled"
          ? t.board_outcome_cancelled
          : null;

  const cardStatus =
    card.status === "done" ? (card.outcome ?? "done") : card.status;

  return (
    <div className="task-card" data-task-status={cardStatus}>
      <div className="task-card-top-row">
        <div className="task-card-info">
          <div className="task-card-title">{card.title}</div>
          <div className="task-card-meta">
            <span className="task-card-meta-icon">📁</span>
            <span className="task-card-meta-text">{card.project_dir}</span>
          </div>
          {!card.parallel_ok && <div className="task-card-meta"><span className="task-card-meta-icon">⚑</span>{t.board_card_solo_hint}</div>}
          {card.status === "running" && <div className="task-card-meta">{t.board_running_hint}</div>}
          {card.status === "done" && card.error_message && (
            <div className="task-card-meta">{card.error_message}</div>
          )}
        </div>
        {card.interactive && <div className="task-card-avatar">👤</div>}
      </div>

      <div className="task-card-badges">
        {card.status === "running" && (
          <span className="task-badge task-badge--running">
            <span className="task-badge-dot" />
            {t.board_col_running}
          </span>
        )}
        {card.interactive && (
          <span className="task-badge task-badge--interactive">{t.board_badge_interactive}</span>
        )}
        {card.status === "done" && card.outcome && (
          <span className={`task-badge task-badge--${card.outcome}`}>{outcomeLabel}</span>
        )}
        {card.label && (
          <span
            className="task-label-chip"
            style={{ "--label-hue": hashLabelHue(card.label) } as CSSProperties}
          >
            {card.label}
          </span>
        )}
      </div>

      <div className="task-card-divider" />

      <div className="task-card-actions">
        {card.status === "planning" && (
          <>
            <button className="tb-btn tb-btn--ghost" disabled={busy} onClick={onEdit}>{t.board_edit_card}</button>
            <button className="tb-btn tb-btn--danger-ghost" disabled={busy} onClick={() => void remove()}>{t.board_delete}</button>
          </>
        )}
        {card.status === "queued" && (
          <button className="tb-btn tb-btn--ghost" onClick={onEditLabel}>{t.board_action_edit_label}</button>
        )}
        {card.status === "running" && (
          <>
            <button className="tb-btn tb-btn--ghost" disabled={busy} onClick={() => void run(() => stopTask(projectId, card.id))}>
              {t.board_action_stop}
            </button>
            {card.interactive && (
              <button className="tb-btn tb-btn--primary" disabled={busy} onClick={() => void markDone()}>
                {t.board_action_mark_done}
              </button>
            )}
            {card.tab_id && <button className="tb-btn tb-btn--ghost" onClick={openTab}>{t.board_action_open_tab}</button>}
            <button className="tb-btn tb-btn--ghost" onClick={onEditLabel}>{t.board_action_edit_label}</button>
          </>
        )}
        {card.status === "done" && (
          <>
            {card.transcript_path && (
              <button className="tb-btn tb-btn--ghost" onClick={onViewTranscript}>{t.board_action_transcript}</button>
            )}
            <button className="tb-btn tb-btn--ghost" disabled={busy} onClick={() => void run(() => cloneTask(projectId, card.id))}>
              {t.board_action_requeue}
            </button>
            {card.worktree_branch && (
              <button className="tb-btn tb-btn--primary" disabled={busy} onClick={() => void run(() => mergeTaskWorktree(projectId, card.id))}>
                {t.board_action_merge_worktree}
              </button>
            )}
            {/* 封存不問過就直接做：資料完全保留，隨時能從封存清單放回來，
                所以它是可復原的動作，不該跟刪除一樣攔一次。 */}
            <button className="tb-btn tb-btn--ghost" disabled={busy} onClick={() => void run(() => archiveTask(projectId, card.id))}>
              {t.board_action_archive}
            </button>
            <button className="tb-btn tb-btn--ghost" onClick={onEditLabel}>{t.board_action_edit_label}</button>
            <button className="tb-btn tb-btn--danger-ghost" disabled={busy} onClick={() => void remove()}>{t.board_delete}</button>
          </>
        )}
      </div>
    </div>
  );
}
