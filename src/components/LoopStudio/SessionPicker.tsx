import { useState, useEffect, useCallback, useRef } from "react";
import { loopSessionList, loopSessionDelete, loopSessionClearAll, type LoopSessionSummary } from "../../ipc/loopSession";

interface SessionPickerProps {
  onResume: (sessionId: string) => void;
  isRunning: boolean;
}

const STATUS_LABEL: Record<string, string> = {
  running: "執行中",
  paused: "已暫停",
  completed: "已完成",
  failed: "失敗",
};

function formatDate(iso: string): string {
  const d = new Date(iso);
  return `${d.getMonth() + 1}/${d.getDate()} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

export function SessionPicker({ onResume, isRunning }: SessionPickerProps) {
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
        📋 過去的 Sessions {sessions.length > 0 ? `(${sessions.length})` : ""}
        <span className="ls-session-chevron">{open ? "▲" : "▼"}</span>
      </button>

      {open && sessions.length === 0 && (
        <div className="ls-session-empty">尚無過去的 Sessions</div>
      )}

      {open && sessions.length > 0 && (
        <div className="ls-session-list">
          {sessions.map(s => (
            <div key={s.id} className={`ls-session-item status-${s.status}`}>
              <div className="ls-session-info">
                <span className="ls-session-status">{STATUS_LABEL[s.status] ?? s.status}</span>
                <span className="ls-session-goal">{s.goal.slice(0, 60)}{s.goal.length > 60 ? "..." : ""}</span>
                <span className="ls-session-meta">第 {s.iteration} 輪・{formatDate(s.updated_at)}</span>
              </div>
              <div className="ls-session-actions">
                {(s.status === "paused" || s.status === "running") && (
                  <button
                    type="button"
                    className="ls-session-resume-btn"
                    onClick={() => { setOpen(false); onResume(s.id); }}
                    disabled={isRunning}
                  >
                    ▶ 繼續
                  </button>
                )}
                <button
                  type="button"
                  className="ls-session-delete-btn"
                  onClick={e => handleDelete(e, s.id)}
                  title="刪除"
                >
                  ×
                </button>
              </div>
            </div>
          ))}

          <div className="ls-session-footer">
            {confirmClear ? (
              <div className="ls-clear-confirm">
                <span>確定要清除全部 {sessions.length} 筆記錄？</span>
                <button type="button" className="ls-clear-confirm-yes" onClick={handleClearAll}>確定</button>
                <button type="button" className="ls-clear-confirm-no" onClick={() => setConfirmClear(false)}>取消</button>
              </div>
            ) : (
              <button
                type="button"
                className="ls-clear-all-btn"
                onClick={() => setConfirmClear(true)}
                disabled={isRunning}
              >
                🗑 清除全部
              </button>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
