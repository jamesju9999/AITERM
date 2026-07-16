import { useState, useEffect, useCallback, useRef } from "react";
import { loopSessionList, loopSessionDelete, loopSessionClearAll, type LoopSessionSummary } from "../../ipc/loopSession";
import { useLocale } from '../../contexts/LocaleContext';

interface SessionPickerProps {
  onResume: (sessionId: string) => void;
  isRunning: boolean;
}

function formatDate(iso: string): string {
  const d = new Date(iso);
  return `${d.getMonth() + 1}/${d.getDate()} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

export function SessionPicker({ onResume, isRunning }: SessionPickerProps) {
  const { t } = useLocale();

  const statusLabel = (status: string): string => {
    const map: Record<string, string> = {
      running: t.ls_status_running,
      paused: t.ls_status_paused,
      completed: t.ls_status_completed,
      failed: t.ls_status_failed,
    };
    return map[status] ?? status;
  };

  const [sessions, setSessions] = useState<LoopSessionSummary[]>([]);
  const [open, setOpen] = useState(false);
  const [confirmClear, setConfirmClear] = useState(false);

  const refresh = useCallback(() => {
    loopSessionList().then(setSessions).catch(() => setSessions([]));
  }, []);

  useEffect(() => { refresh(); }, [refresh]);

  // Refresh list when loop finishes (isRunning: true → false)
  const prevIsRunning = useRef(isRunning);
  useEffect(() => {
    if (prevIsRunning.current && !isRunning) {
      refresh();
    }
    prevIsRunning.current = isRunning;
  }, [isRunning, refresh]);

  const handleDelete = useCallback(async (e: React.MouseEvent, id: string) => {
    e.stopPropagation();
    await loopSessionDelete(id).catch(() => {});
    refresh();
  }, [refresh]);

  const handleClearAll = useCallback(async () => {
    await loopSessionClearAll().catch(() => {});
    setSessions([]);
    setConfirmClear(false);
    setOpen(false);
  }, []);

  return (
    <div className="ls-session-picker">
      <button
        type="button"
        className="ls-session-toggle"
        onClick={() => { setOpen(o => !o); setConfirmClear(false); }}
        disabled={isRunning || sessions.length === 0}
      >
        {t.ls_sessions_title(sessions.length)}
        <span className="ls-session-chevron">{open ? "▲" : "▼"}</span>
      </button>

      {open && sessions.length === 0 && (
        <div className="ls-session-empty">{t.ls_session_empty}</div>
      )}

      {open && sessions.length > 0 && (
        <div className="ls-session-list">
          {sessions.map(s => (
            <div key={s.id} className={`ls-session-item status-${s.status}`}>
              <div className="ls-session-info">
                <span className="ls-session-status">{statusLabel(s.status)}</span>
                <span className="ls-session-goal">{s.goal.slice(0, 60)}{s.goal.length > 60 ? "..." : ""}</span>
                <span className="ls-session-meta">{t.ls_session_iteration(s.iteration)}・{formatDate(s.updated_at)}</span>
              </div>
              <div className="ls-session-actions">
                {(s.status === "paused" || s.status === "running") && (
                  <button
                    type="button"
                    className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
                    onClick={() => { setOpen(false); onResume(s.id); }}
                    disabled={isRunning}
                  >
                    {t.ls_session_resume}
                  </button>
                )}
                <button
                  type="button"
                  className="ls-session-delete-btn aiterm-btn aiterm-btn--ghost"
                  onClick={e => handleDelete(e, s.id)}
                  title={t.ls_session_delete}
                >
                  ×
                </button>
              </div>
            </div>
          ))}

          <div className="ls-session-footer">
            {confirmClear ? (
              <div className="ls-clear-confirm">
                <span>{t.ls_session_clear_confirm(sessions.length)}</span>
                <button type="button" className="aiterm-btn aiterm-btn--danger-solid aiterm-btn--sm" onClick={handleClearAll}>{t.ls_session_confirm}</button>
                <button type="button" className="ls-clear-confirm-no" onClick={() => setConfirmClear(false)}>{t.cancel}</button>
              </div>
            ) : (
              <button
                type="button"
                className="ls-clear-all-btn aiterm-btn aiterm-btn--secondary"
                onClick={() => setConfirmClear(true)}
                disabled={isRunning}
              >
                {t.ls_session_clear_all}
              </button>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
