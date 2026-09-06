import { useCallback, useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { listArchivedTasks, unarchiveTask, type TaskRow } from "../../ipc/tasks";
import { TranscriptDialog } from "./TranscriptDialog";

/** 一頁幾筆。 */
const PAGE_SIZE = 20;

/** 打字停下多久才真的去查。每個按鍵都打一次 IPC 是浪費，也會讓結果亂跳。 */
const SEARCH_DEBOUNCE_MS = 250;

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
}: {
  projectId: string;
  onClose: () => void;
  onRestored: () => void;
}) {
  const { t } = useLocale();
  const [rows, setRows] = useState<TaskRow[]>([]);
  const [total, setTotal] = useState(0);
  /** 輸入框當下的字。送去查詢的是下面 debounce 過的 `query`。 */
  const [term, setTerm] = useState("");
  const [query, setQuery] = useState("");
  const [page, setPage] = useState(0);
  const [busy, setBusy] = useState(false);
  /**
   * 正在看對話記錄的那張封存卡片。
   *
   * 這個視窗自己渲染 TranscriptDialog，不把 id 交給 ProjectBoard——
   * 那邊是從 `tasks` 裡找卡片，而 `tasks` 只有未封存的，封存的永遠找不到，
   * 結果是封存視窗關掉、對話記錄也沒出來。實機回報過。
   */
  const [transcriptFor, setTranscriptFor] = useState<TaskRow | null>(null);

  const refresh = useCallback(async () => {
    const p = await listArchivedTasks(projectId, query, PAGE_SIZE, page * PAGE_SIZE);
    setRows(p.rows);
    setTotal(p.total);
  }, [projectId, query, page]);

  // 打字先進 term，安靜下來才變成 query 去查。換關鍵字一定要回到第一頁：
  // 停在第 3 頁搜一個只有 5 筆結果的字，會得到一個空白的視窗。
  useEffect(() => {
    const id = setTimeout(() => {
      setQuery(term);
      setPage(0);
    }, SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(id);
  }, [term]);

  // 關鍵字或頁碼一變就重查。用帶 cleanup 的 inline 寫法而不是
  // `void refresh()`：後者會被 react-hooks/set-state-in-effect 判定為
  // 「effect 唯一的內容就是呼叫一個以 setState 結尾的函式」而報錯
  // （同 ReportDialog）。
  useEffect(() => {
    let alive = true;
    void listArchivedTasks(projectId, query, PAGE_SIZE, page * PAGE_SIZE).then((p) => {
      if (!alive) return;
      setRows(p.rows);
      setTotal(p.total);
    });
    return () => {
      alive = false;
    };
  }, [projectId, query, page]);

  const pageCount = Math.max(1, Math.ceil(total / PAGE_SIZE));

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
            {total > 0 && <span className="archive-count">{t.board_archive_count(total)}</span>}
          </h3>
          <button className="tb-btn tb-btn--ghost" onClick={onClose}>
            {t.report_close}
          </button>
        </div>

        <div className="archive-search-row">
          <input
            className="task-field-input"
            data-testid="archive-search"
            placeholder={t.board_archive_search}
            value={term}
            onChange={(e) => setTerm(e.target.value)}
          />
        </div>

        {rows.length === 0 ? (
          <div className="archive-empty" data-testid="archive-empty">
            {query ? t.board_archive_no_match : t.board_archive_empty}
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
                      data-testid={`archive-transcript-${r.id}`}
                      onClick={() => setTranscriptFor(r)}
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

        {pageCount > 1 && (
          <div className="archive-pager">
            <button
              className="tb-btn tb-btn--ghost"
              data-testid="archive-prev"
              disabled={page === 0}
              onClick={() => setPage((p) => Math.max(0, p - 1))}
            >
              {t.board_archive_prev}
            </button>
            <span className="archive-pager-label">{t.board_archive_page(page + 1, pageCount)}</span>
            <button
              className="tb-btn tb-btn--ghost"
              data-testid="archive-next"
              disabled={page + 1 >= pageCount}
              onClick={() => setPage((p) => Math.min(pageCount - 1, p + 1))}
            >
              {t.board_archive_next}
            </button>
          </div>
        )}
      </div>

      {/* 疊在封存清單上面。包一層 stopPropagation：TranscriptDialog 自己
          的底板點擊會冒泡到外層這個底板，那會把封存視窗一起關掉。 */}
      {transcriptFor && (
        <div onClick={(e) => e.stopPropagation()}>
          <TranscriptDialog
            projectId={projectId}
            taskId={transcriptFor.id}
            body={transcriptFor.body}
            onClose={() => setTranscriptFor(null)}
          />
        </div>
      )}
    </div>
  );
}
