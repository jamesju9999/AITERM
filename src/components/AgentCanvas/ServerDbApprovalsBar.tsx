import { useCallback, useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { useIntakeRemote } from "../../hooks/useIntakeRemote";
import { decideServerDb, listServerDbApprovals, type ServerDbQuery } from "../../ipc/serverDb";
import { decideServerSys, listServerSysApprovals, type ServerSysApproval } from "../../ipc/serverSystems";

/** 多久問一次 server 有沒有新的待核准寫入（毫秒）。 */
const POLL_MS = 3000;

const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));

/**
 * 共用專案頂列：有 Agent 的資料庫寫入或系統（網頁）寫入請求在等管理員核准時出現「🗄 N 筆待核准」，按下去看完整內容。
 * 管理員可以核准或拒絕；成員看得到數量與內容但不能決定。沒有待核准時什麼都不畫。
 */
export function ServerDbApprovalsBar({ projectId, agentName, active }: { projectId: string; agentName: (id: string | null) => string; active: boolean }) {
  const { t } = useLocale();
  const isAdmin = useIntakeRemote().role === "admin";
  const [list, setList] = useState<{ projectId: string; db: ServerDbQuery[]; sys: ServerSysApproval[] }>({ projectId, db: [], sys: [] });
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    // 兩種待核准各自查：其中一種失敗（例如 server 較舊）不影響另一種；連不上時保留上一次的清單，等下一輪。
    const [db, sys] = await Promise.allSettled([listServerDbApprovals(projectId), listServerSysApprovals(projectId)]);
    setList((cur) => {
      const same = cur.projectId === projectId;
      return {
        projectId,
        db: db.status === "fulfilled" ? db.value : same ? cur.db : [],
        sys: sys.status === "fulfilled" ? sys.value : same ? cur.sys : [],
      };
    });
  }, [projectId]);

  useEffect(() => {
    if (!active) return;
    void load();
    const timer = setInterval(() => void load(), POLL_MS);
    return () => clearInterval(timer);
  }, [active, load]);

  const dbRows = list.projectId === projectId ? list.db : [];
  const sysRows = list.projectId === projectId ? list.sys : [];
  const total = dbRows.length + sysRows.length;
  if (total === 0) return null;

  const decide = async (kind: "db" | "sys", id: number | string, approve: boolean) => {
    setBusy(true);
    setError(null);
    try {
      if (kind === "db") await decideServerDb(id as number, approve);
      else await decideServerSys(id as string, approve);
      await load();
    } catch (e) {
      setError(errText(e));
      await load();
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <button type="button" className="aiterm-btn aiterm-btn--primary aiterm-btn--sm ac-db-approvals-btn" data-testid="ac-db-approvals" onClick={() => setOpen(true)}>
        🗄 {t.ac_sdb_pending(total)}
      </button>
      {open && (
        <div className="task-dialog-backdrop">
          <div className="task-dialog ac-db-dialog" role="dialog" aria-label={t.ac_sdb_approvals_title} data-testid="ac-db-approvals-dialog">
            <h3 className="task-dialog-title">{t.ac_sdb_approvals_title}</h3>
            <p className="ac-dialog-desc">{isAdmin ? t.ac_sdb_approvals_desc : t.ac_sdb_approvals_readonly}</p>
            <div className="ac-db-conns">
              {dbRows.map((q) => (
                <div key={`db-${q.id}`} className="ac-db-approval" data-testid="ac-db-approval">
                  <div className="ac-db-meta">
                    {agentName(q.agent_id)} · {q.connection_name}
                  </div>
                  <pre className="ac-db-approval-sql">{q.sql}</pre>
                  {isAdmin && (
                    <div className="ac-db-approval-actions">
                      <button className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" disabled={busy} onClick={() => void decide("db", q.id, true)}>
                        {t.ac_sdb_approve}
                      </button>
                      <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" disabled={busy} onClick={() => void decide("db", q.id, false)}>
                        {t.ac_sdb_deny}
                      </button>
                    </div>
                  )}
                </div>
              ))}
              {sysRows.map((q) => (
                <div key={`sys-${q.id}`} className="ac-db-approval" data-testid="ac-sys-approval">
                  <div className="ac-db-meta">
                    {agentName(q.agent_id)} · {q.system_name}
                  </div>
                  <div>{q.intent}</div>
                  {q.declared_read && <div className="ac-bad-text">{t.ac_ssys_approval_read_warn}</div>}
                  <pre className="ac-db-approval-sql">{`${q.method} ${q.url}`}</pre>
                  {isAdmin && (
                    <div className="ac-db-approval-actions">
                      <button className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" disabled={busy} onClick={() => void decide("sys", q.id, true)}>
                        {t.ac_sdb_approve}
                      </button>
                      <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" disabled={busy} onClick={() => void decide("sys", q.id, false)}>
                        {t.ac_sdb_deny}
                      </button>
                    </div>
                  )}
                </div>
              ))}
            </div>
            {error && (
              <p className="ac-bad-text" role="alert">
                {error}
              </p>
            )}
            <div className="task-dialog-actions">
              <button className="aiterm-btn aiterm-btn--secondary" onClick={() => setOpen(false)}>
                {t.ac_done_close}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
