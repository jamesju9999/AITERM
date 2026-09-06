import { useCallback, useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { listArchivedTasks, unarchiveTask, type TaskWithAttachments } from "../../ipc/tasks";

/** 封存時間。跟報告歷史清單一樣交給平台決定日期慣例。 */
function formatArchivedAt(at: number | null): string {
  if (!at) return "";
  return new Date(at * 1000).toLocaleString(undefined, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/**
 * 封存起來的工作。
 *
 * 封存不是刪除——卡片、對話記錄、AI 摘要、附件全部原封不動地留在同一個
 * `tasks.db` 裡，只是不再出現在四欄上。這個視窗就是那道保證：看得到、
 * 讀得到、放得回去。
 */
export function ArchiveDialog({
  projectId,
  onClose,
  onRestored,
  onViewTranscript,
}: {
  projectId: string;
  onClose: () => void;
  onRestored: () => void;
  onViewTranscript: (taskId: string) => void;
}) {
  const { t } = useLocale();
  const [rows, setRows] = useState<TaskWithAttachments[]>([]);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    setRows(await listArchivedTasks(projectId));
  }, [projectId]);

  // 掛載時抓一次。用帶 cleanup 的 inline 寫法而不是 `void refresh()`：
  // 後者會被 react-hooks/set-state-in-effect 判定為「effect 唯一的內容
  // 就是呼叫一個以 setState 結尾的函式」而報錯（同 ReportDialog）。
  useEffect(() => {
    let alive = true;
    void listArchivedTasks(projectId).then((list) => {
      if (alive) setRows(list);
    });
    return () => {
      alive = false;
    };
  }, [projectId]);

  const restore = async (id: string) => {
    setBusy(true);
    try {
      await unarchiveTask(projectId, id);
      await refresh();
      // 看板要跟著把卡片放回「已完成」欄。
      onRestored();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="task-dialog-backdrop" onClick={onClose}>
      <div className="archive-dialog" onClick={(e) => e.stopPropagation()}>
        <div className="report-head">
          <h3>
            {t.board_archive_title}
            {rows.length > 0 && (
              <span className="archive-count">{t.board_archive_count(rows.length)}</span>
            )}
          </h3>
          <button className="tb-btn tb-btn--ghost" onClick={onClose}>
            {t.report_close}
          </button>
        </div>

        {rows.length === 0 ? (
          <div className="archive-empty" data-testid="archive-empty">
            {t.board_archive_empty}
          </div>
        ) : (
          <ul className="archive-list">
            {rows.map((r) => (
              <li key={r.id} className="archive-row">
                <div className="archive-row-info">
                  <div className="archive-row-title">{r.title}</div>
                  <div className="archive-row-meta">
                    <span className="task-card-meta-icon">📁</span>
                    <span className="task-card-meta-text">{r.project_dir}</span>
                  </div>
                  <div className="archive-row-meta">
                    {t.board_archived_at} {formatArchivedAt(r.archived_at)}
                  </div>
                </div>
                <div className="archive-row-actions">
                  {r.transcript_path && (
                    <button
                      className="tb-btn tb-btn--ghost"
                      onClick={() => onViewTranscript(r.id)}
                    >
                      {t.board_action_transcript}
                    </button>
                  )}
                  <button
                    className="tb-btn tb-btn--ghost"
                    data-testid={`archive-restore-${r.id}`}
                    disabled={busy}
                    onClick={() => void restore(r.id)}
                  >
                    {t.board_archive_restore}
                  </button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
