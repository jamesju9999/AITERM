import { useCallback, useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { listLiveRuns, type LiveRun } from "../../ipc/serverLive";
import { LiveViewDialog, type LiveTarget } from "./LiveViewDialog";

/** 多久問一次 server 有哪些瀏覽器在跑（毫秒）。 */
const POLL_MS = 3000;

/**
 * 共用專案頂列：server 上有 Agent 的瀏覽器在跑時出現「👁 N 個瀏覽器執行中」，按下去選一個看（管理員可以接手）。
 * 沒有在跑就什麼都不畫；查詢失敗（例如 server 較舊）也不畫。
 */
export function ServerLiveBar({ projectId, agentName, active }: { projectId: string; agentName: (id: string | null) => string; active: boolean }) {
  const { t } = useLocale();
  const [list, setList] = useState<{ projectId: string; runs: LiveRun[] }>({ projectId, runs: [] });
  const [picker, setPicker] = useState(false);
  const [watching, setWatching] = useState<LiveTarget | null>(null);

  const load = useCallback(async () => {
    try {
      setList({ projectId, runs: await listLiveRuns(projectId) });
    } catch {
      // 查不到就保留上一次的清單，等下一輪。
    }
  }, [projectId]);

  useEffect(() => {
    if (!active) return;
    // 第一次查詢排到下一個 tick（effect 裡不同步呼叫會設定 state 的函式）。
    const first = setTimeout(() => void load(), 0);
    const id = setInterval(() => void load(), POLL_MS);
    return () => {
      clearTimeout(first);
      clearInterval(id);
    };
  }, [active, load]);

  const runs = list.projectId === projectId ? list.runs : [];
  if (runs.length === 0 && !watching) return null;

  return (
    <>
      {runs.length > 0 && (
        <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" data-testid="ac-live-bar" onClick={() => setPicker(true)}>
          👁 {t.ac_live_bar(runs.length)}
        </button>
      )}
      {picker && !watching && (
        <div className="task-dialog-backdrop">
          <div className="task-dialog ac-db-dialog" role="dialog" aria-label={t.ac_live_runs_title}>
            <h3 className="task-dialog-title">{t.ac_live_runs_title}</h3>
            <p className="ac-dialog-desc">{t.ac_live_runs_desc}</p>
            <div className="ac-db-conns">
              {runs.map((r) => (
                <div key={`${r.card_id}-${r.system_id}`} className="ac-db-option ac-ss-row" data-testid="ac-live-run">
                  <span>
                    {r.system_name} <span className="ac-db-meta">· {agentName(r.agent_id)}{r.manual ? ` · ${t.ac_live_manual_tag}` : ""}</span>
                  </span>
                  <button
                    type="button"
                    className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                    onClick={() => setWatching({ kind: "agent", systemId: r.system_id, systemName: r.system_name, cardId: r.card_id })}
                    aria-label={`${t.ac_live_view} ${r.system_name}`}
                  >
                    {t.ac_live_view}
                  </button>
                </div>
              ))}
              {runs.length === 0 && <p className="task-field-hint">{t.ac_live_runs_none}</p>}
            </div>
            <div className="task-dialog-actions">
              <span className="ac-spacer" />
              <button className="aiterm-btn aiterm-btn--secondary" onClick={() => setPicker(false)}>
                {t.ac_live_close}
              </button>
            </div>
          </div>
        </div>
      )}
      {watching && (
        <LiveViewDialog
          target={watching}
          onClose={() => {
            setWatching(null);
            void load();
          }}
        />
      )}
    </>
  );
}
