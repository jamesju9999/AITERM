import { confirm } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useRef, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import {
  onSystemsDenied,
  onSystemsJobDisabled,
  onSystemsJobFinished,
  onSystemsNeedApproval,
  onSystemsResolved,
  onSystemsState,
  systemsAudit,
  systemsDecide,
  systemsHandback,
  systemsJobDelete,
  systemsJobRunNow,
  systemsJobRuns,
  systemsJobs,
  systemsJobSave,
  systemsJobSetEnabled,
  systemsList,
  systemsMcpConfig,
  systemsOpenLogin,
  systemsPending,
  systemsRecipeDelete,
  systemsRecipes,
  systemsRemove,
  systemsSave,
  systemsStatus,
  systemsStop,
  systemsTakeover,
  type AuditRow,
  type JobRun,
  type PendingApproval,
  type Recipe,
  type SystemConnection,
  type SystemJob,
  type SystemStatus,
} from "../../ipc/systems";
import { unlistenOnCleanup } from "../../lib/eventSubscription";
import { restoreWebviewFocus } from "../../lib/webviewFocus";
import { LinkIcon } from "../Icons";
import { ApprovalCards } from "./ApprovalCards";
import { AuditLog } from "./AuditLog";
import { ConnectionForm } from "./ConnectionForm";
import { JobList } from "./JobList";
import { RecipeList } from "./RecipeList";
import "./index.css";

type PanelKind = "audit" | "recipes" | "jobs";

const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));

export function SystemsView() {
  const { t } = useLocale();
  const [systems, setSystems] = useState<SystemConnection[]>([]);
  const [status, setStatus] = useState<Record<string, SystemStatus>>({});
  const [approvals, setApprovals] = useState<PendingApproval[]>([]);
  const [editing, setEditing] = useState<SystemConnection | "new" | null>(null);
  // 每個系統卡片底下一次只展開一個面板（操作記錄／配方／定時任務）。
  const [panel, setPanel] = useState<{ id: string; kind: PanelKind } | null>(null);
  const [audit, setAudit] = useState<AuditRow[]>([]);
  const [recipes, setRecipes] = useState<Recipe[]>([]);
  const [jobs, setJobs] = useState<SystemJob[]>([]);
  const [jobRuns, setJobRuns] = useState<Record<string, JobRun[]>>({});
  const [notice, setNotice] = useState<string | null>(null);
  const [mcpConfig, setMcpConfig] = useState("");
  const [error, setError] = useState<string | null>(null);
  const aliveRef = useRef(true);
  const auditForRef = useRef<string | null>(null);
  const jobsForRef = useRef<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const list = await systemsList();
      const [pend, statuses] = await Promise.all([
        systemsPending(),
        Promise.all(list.map(async (s) => [s.id, await systemsStatus(s.id)] as const)),
      ]);
      if (!aliveRef.current) return;
      setSystems(list);
      setApprovals(pend);
      setStatus(Object.fromEntries(statuses));
      // 第一次儲存連線之後才會有 token，設定要跟著更新。
      void systemsMcpConfig().then((c) => aliveRef.current && setMcpConfig(c)).catch(() => {});
    } catch (e) {
      if (aliveRef.current) setError(errText(e));
    }
  }, []);

  const loadRecipes = useCallback(async (id: string) => {
    try {
      const rows = await systemsRecipes(id);
      if (aliveRef.current) setRecipes(rows);
    } catch (e) {
      if (aliveRef.current) setError(errText(e));
    }
  }, []);

  const loadJobs = useCallback(async (id: string) => {
    try {
      const list = await systemsJobs(id);
      const entries = await Promise.all(list.map(async (j) => [j.id, await systemsJobRuns(j.id)] as const));
      if (aliveRef.current && jobsForRef.current === id) {
        setJobs(list);
        setJobRuns(Object.fromEntries(entries));
      }
    } catch (e) {
      if (aliveRef.current) setError(errText(e));
    }
  }, []);

  const loadAudit = useCallback(async (id: string) => {
    try {
      const rows = await systemsAudit(id, 100);
      if (aliveRef.current && auditForRef.current === id) setAudit(rows);
    } catch (e) {
      if (aliveRef.current) setError(errText(e));
    }
  }, []);

  // 先訂閱事件、再取初始狀態，事件才不會在兩者之間漏掉。
  useEffect(() => {
    aliveRef.current = true;
    const offs = [
      unlistenOnCleanup(
        onSystemsNeedApproval((a) => {
          if (aliveRef.current) setApprovals((prev) => (prev.some((p) => p.id === a.id) ? prev : [...prev, a]));
        }),
        "systems://need-approval",
      ),
      unlistenOnCleanup(
        onSystemsResolved((id) => {
          if (aliveRef.current) setApprovals((prev) => prev.filter((p) => p.id !== id));
        }),
        "systems://resolved",
      ),
      unlistenOnCleanup(onSystemsState(() => void refresh()), "systems://state"),
      unlistenOnCleanup(
        onSystemsJobFinished(() => {
          if (jobsForRef.current) void loadJobs(jobsForRef.current);
        }),
        "systems://job-finished",
      ),
      unlistenOnCleanup(
        onSystemsJobDisabled((name) => {
          if (aliveRef.current) setNotice(t.sys_job_disabled_toast(name));
          if (jobsForRef.current) void loadJobs(jobsForRef.current);
        }),
        "systems://job-disabled",
      ),
      unlistenOnCleanup(
        onSystemsDenied(() => {
          if (auditForRef.current) void loadAudit(auditForRef.current);
        }),
        "systems://denied",
      ),
    ];
    queueMicrotask(() => void refresh());
    return () => {
      aliveRef.current = false;
      offs.forEach((off) => off());
    };
  }, [refresh, loadAudit, loadJobs, t]);

  const run = async (fn: () => Promise<unknown>) => {
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(errText(e));
    }
    await refresh();
  };

  const nameOf = (id: string) => systems.find((s) => s.id === id)?.name ?? id;

  const statusLabel = (s: SystemConnection) => {
    const st = status[s.id];
    if (!s.enabled) return t.sys_disabled_badge;
    if (!st?.running) return t.sys_status_idle;
    if (st.needs_login) return t.sys_status_needs_login;
    if (st.manual) return t.sys_status_manual;
    return t.sys_status_running;
  };

  const badgeKind = (s: SystemConnection) => {
    const st = status[s.id];
    if (!s.enabled) return "off";
    if (st?.needs_login) return "warn";
    if (st?.manual) return "info";
    return st?.running ? "on" : "idle";
  };

  const openPanel = (id: string, kind: PanelKind) => {
    const closing = panel?.id === id && panel.kind === kind;
    const next = closing ? null : { id, kind };
    setPanel(next);
    auditForRef.current = next?.kind === "audit" ? id : null;
    jobsForRef.current = next?.kind === "jobs" ? id : null;
    setAudit([]);
    setRecipes([]);
    setJobs([]);
    setJobRuns({});
    if (!next) return;
    if (kind === "audit") void loadAudit(id);
    else if (kind === "recipes") void loadRecipes(id);
    else void loadJobs(id);
  };

  const remove = async (s: SystemConnection) => {
    const ok = await confirm(t.sys_delete_confirm(s.name), { kind: "warning" });
    await restoreWebviewFocus();
    if (ok) await run(() => systemsRemove(s.id));
  };

  return (
    <div className="systems">
      <div className="systems__bar">
        <h2 className="systems__title">
          <LinkIcon size={18} /> {t.systems_tab}
        </h2>
        <button type="button" className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" onClick={() => setEditing("new")}>
          {t.sys_add}
        </button>
      </div>
      {error && (
        <p className="systems__error" role="alert">
          {t.sys_error(error)}
        </p>
      )}
      {notice && (
        <p className="systems__warn" role="status" data-testid="job-notice">
          {notice}
        </p>
      )}

      <ApprovalCards
        approvals={approvals}
        nameOf={nameOf}
        onDecide={(id, approve) => {
          setApprovals((prev) => prev.filter((p) => p.id !== id));
          void run(() => systemsDecide(id, approve));
        }}
        onRejectAndDisable={(a) => {
          const conn = systems.find((s) => s.id === a.system_id);
          setApprovals((prev) => prev.filter((p) => p.id !== a.id));
          void run(async () => {
            await systemsDecide(a.id, false);
            if (conn) await systemsSave({ ...conn, enabled: false });
          });
        }}
      />

      {editing && (
        <ConnectionForm
          initial={editing === "new" ? undefined : editing}
          onCancel={() => setEditing(null)}
          onSave={async (c) => {
            await systemsSave(c);
            setEditing(null);
            await refresh();
          }}
        />
      )}

      {systems.length === 0 && !editing ? (
        <div className="systems__empty">
          <p>{t.sys_empty}</p>
          <button type="button" className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" onClick={() => setEditing("new")}>
            {t.sys_add}
          </button>
        </div>
      ) : (
        <ul className="systems__list">
          {systems.map((s) => {
            const st = status[s.id];
            return (
              <li key={s.id} className="systems__item" data-testid="system-item">
                <div className="systems__head">
                  <div className="systems__titleblock">
                    <div className="systems__name">
                      <strong>{s.name}</strong>
                      <span className={`systems__badge systems__badge--${badgeKind(s)}`} data-testid="system-status">
                        {statusLabel(s)}
                      </span>
                      {st && st.pending > 0 && <span className="systems__badge systems__badge--warn">{t.sys_pending_n(st.pending)}</span>}
                    </div>
                    <div className="systems__url">
                      {s.start_url}
                      <span className="systems__dot">·</span>
                      {{ read_only: t.sys_mode_read_only, approve_each: t.sys_mode_approve_each, rules: t.sys_mode_rules, autonomous: t.sys_mode_autonomous }[s.mode]}
                    </div>
                  </div>
                  <div className="systems__manage">
                    <button type="button" className="systems__link" onClick={() => void run(() => systemsSave({ ...s, enabled: !s.enabled }))}>
                      {s.enabled ? t.sys_disable : t.sys_enable}
                    </button>
                    <button type="button" className="systems__link" onClick={() => setEditing(s)}>
                      {t.sys_edit}
                    </button>
                    <button type="button" className="systems__link systems__link--danger" onClick={() => void remove(s)}>
                      {t.sys_delete}
                    </button>
                  </div>
                </div>
                <div className="systems__actions">
                  <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" disabled={!s.enabled} onClick={() => void run(() => systemsOpenLogin(s.id))}>
                    {t.sys_open_login}
                  </button>
                  <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" disabled={!st?.running} onClick={() => void run(() => systemsStop(s.id))}>
                    {t.sys_stop}
                  </button>
                  {st?.manual ? (
                    <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => void run(() => systemsHandback(s.id))}>
                      {t.sys_handback}
                    </button>
                  ) : (
                    <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" disabled={!st?.running} onClick={() => void run(() => systemsTakeover(s.id))}>
                      {t.sys_takeover}
                    </button>
                  )}
                </div>
                <div className="systems__tabs" role="group">
                  {(["audit", "recipes", "jobs"] as const).map((kind) => (
                    <button
                      key={kind}
                      type="button"
                      className={panel?.id === s.id && panel.kind === kind ? "is-on" : ""}
                      aria-pressed={panel?.id === s.id && panel.kind === kind}
                      onClick={() => openPanel(s.id, kind)}
                    >
                      {kind === "audit" ? t.sys_audit_title : kind === "recipes" ? t.sys_recipes_title : t.sys_jobs_title}
                    </button>
                  ))}
                </div>
                {panel?.id === s.id && panel.kind === "audit" && <AuditLog rows={audit} />}
                {panel?.id === s.id && panel.kind === "jobs" && (
                  <JobList
                    system={s}
                    jobs={jobs}
                    runs={jobRuns}
                    onSave={async (job, id) => {
                      await systemsJobSave(job, id);
                      await loadJobs(s.id);
                    }}
                    onDelete={async (j) => {
                      const ok = await confirm(t.sys_delete_confirm(j.name), { kind: "warning" });
                      await restoreWebviewFocus();
                      if (ok) {
                        await run(() => systemsJobDelete(j.id));
                        await loadJobs(s.id);
                      }
                    }}
                    onToggle={(j) => void run(() => systemsJobSetEnabled(j.id, !j.enabled)).then(() => loadJobs(s.id))}
                    onRunNow={(j) => void run(() => systemsJobRunNow(j.id)).then(() => loadJobs(s.id))}
                    onShowRuns={() => void loadJobs(s.id)}
                  />
                )}
                {panel?.id === s.id && panel.kind === "recipes" && (
                  <RecipeList
                    recipes={recipes}
                    onDelete={async (r) => {
                      const ok = await confirm(t.sys_recipe_delete_confirm(r.name), { kind: "warning" });
                      await restoreWebviewFocus();
                      if (ok) {
                        await run(() => systemsRecipeDelete(r.id));
                        await loadRecipes(s.id);
                      }
                    }}
                  />
                )}
              </li>
            );
          })}
        </ul>
      )}

      <details className="systems__note">
        <summary>{t.sys_limits_title}</summary>
        <p>{t.sys_limits_body}</p>
        {mcpConfig && (
          <>
            <div className="systems__subtitle">{t.sys_mcp_title}</div>
            <p className="systems__muted">{t.sys_mcp_hint}</p>
            <pre className="systems__pre">{mcpConfig}</pre>
          </>
        )}
      </details>
    </div>
  );
}
