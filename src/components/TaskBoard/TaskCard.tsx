import { useState } from "react";
import { confirm } from "@tauri-apps/plugin-dialog";

import { useLocale } from "../../contexts/LocaleContext";
import { cloneTask, deleteTask, markTaskDone, stopTask, type TaskWithAttachments } from "../../ipc/tasks";

export function TaskCard({
  card,
  onEdit,
  onViewTranscript,
  onChanged,
}: {
  card: TaskWithAttachments;
  onEdit: () => void;
  onViewTranscript: () => void;
  onChanged: () => void;
}) {
  const { t } = useLocale();
  const [busy, setBusy] = useState(false);

  const run = async (fn: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await fn();
      onChanged();
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
    void run(() => deleteTask(card.id, closeTab));
  };

  const openTab = () => {
    if (!card.tab_id) return;
    window.dispatchEvent(new CustomEvent("aiterm:focus-tab", { detail: { tabId: card.tab_id } }));
  };

  const outcomeLabel =
    card.outcome === "success"
      ? t.board_outcome_success
      : card.outcome === "failed"
        ? t.board_outcome_failed
        : card.outcome === "cancelled"
          ? t.board_outcome_cancelled
          : null;

  return (
    <div className="task-card">
      <div className="task-card-title">{card.title}</div>
      <div className="task-card-meta">{card.project_dir}</div>
      {!card.parallel_ok && <div className="task-card-meta">⚑ {t.board_card_solo_hint}</div>}
      {card.interactive && (
        <div className="task-badge task-badge--interactive">👤 {t.board_badge_interactive}</div>
      )}

      {card.status === "running" && <div className="task-card-meta">{t.board_running_hint}</div>}

      {card.status === "done" && card.outcome && (
        <div className={`task-badge task-badge--${card.outcome}`}>{outcomeLabel}</div>
      )}
      {card.status === "done" && card.error_message && (
        <div className="task-card-meta">{card.error_message}</div>
      )}

      <div className="task-card-actions">
        {card.status === "planning" && (
          <>
            <button disabled={busy} onClick={onEdit}>{t.board_edit_card}</button>
            <button disabled={busy} onClick={() => void remove()}>{t.board_delete}</button>
          </>
        )}
        {card.status === "running" && (
          <>
            <button disabled={busy} onClick={() => void run(() => stopTask(card.id))}>
              {t.board_action_stop}
            </button>
            {card.interactive && (
              <button disabled={busy} onClick={() => void run(() => markTaskDone(card.id))}>
                {t.board_action_mark_done}
              </button>
            )}
            {card.tab_id && <button onClick={openTab}>{t.board_action_open_tab}</button>}
          </>
        )}
        {card.status === "done" && (
          <>
            {card.transcript_path && (
              <button onClick={onViewTranscript}>{t.board_action_transcript}</button>
            )}
            <button disabled={busy} onClick={() => void run(() => cloneTask(card.id))}>
              {t.board_action_requeue}
            </button>
            <button disabled={busy} onClick={() => void remove()}>{t.board_delete}</button>
          </>
        )}
      </div>
    </div>
  );
}
