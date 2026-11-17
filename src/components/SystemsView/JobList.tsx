import { useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import type { JobKind, JobRun, NewJob, SystemConnection, SystemJob } from "../../ipc/systems";
import { buildSpec, parseSpec, scheduleText } from "./jobSchedule";

interface Props {
  system: SystemConnection;
  jobs: SystemJob[];
  runs: Record<string, JobRun[]>;
  onSave: (job: NewJob, id?: string) => Promise<void>;
  onDelete: (job: SystemJob) => void;
  onToggle: (job: SystemJob) => void;
  onRunNow: (job: SystemJob) => void;
  onShowRuns: (job: SystemJob) => void;
}

/** 這個系統的定時任務：列表、新增／編輯表單、啟用、現在執行、執行記錄。 */
export function JobList({ system, jobs, runs, onSave, onDelete, onToggle, onRunNow, onShowRuns }: Props) {
  const { t } = useLocale();
  const [editing, setEditing] = useState<SystemJob | "new" | null>(null);
  const [openRuns, setOpenRuns] = useState<string | null>(null);

  const warning =
    system.mode === "read_only" ? t.sys_job_unattended_read_only : system.mode === "approve_each" ? t.sys_job_unattended_approve : null;

  const statusLabel: Record<JobRun["status"], string> = {
    running: t.sys_job_running,
    ok: t.sys_job_status_ok,
    failed: t.sys_job_status_failed,
    timeout: t.sys_job_status_timeout,
    skipped: t.sys_job_status_skipped,
  };

  return (
    <section className="systems__audit">
      <div className="systems__panelbar">
        <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => setEditing("new")}>
          {t.sys_job_add}
        </button>
      </div>
      {warning && (
        <p className="systems__warn" role="alert" data-testid="job-warning">
          ⚠ {warning}
        </p>
      )}
      {editing && (
        <JobForm
          key={editing === "new" ? "new" : editing.id}
          system={system}
          initial={editing === "new" ? undefined : editing}
          onCancel={() => setEditing(null)}
          onSave={async (job, id) => {
            await onSave(job, id);
            setEditing(null);
          }}
        />
      )}
      {jobs.length === 0 && !editing ? (
        <p className="systems__muted">{t.sys_jobs_empty}</p>
      ) : (
        <ul>
          {jobs.map((j) => {
            const list = runs[j.id] ?? [];
            const last = list[0];
            return (
              <li key={j.id} data-testid="job-row" className="systems__recipe">
                <div className="systems__head">
                  <strong>{j.name}</strong>
                  <span className="systems__badge">{scheduleText(j.kind, parseSpec(j), t)}</span>
                  {last && <span className={`systems__verdict systems__verdict--${last.status}`}>{statusLabel[last.status]}</span>}
                  {!j.enabled && <span className="systems__badge systems__badge--warn">{j.disabled_reason ? t.sys_job_disabled_reason(j.disabled_reason) : t.sys_disabled_badge}</span>}
                </div>
                <div className="systems__muted">{j.goal}</div>
                <div className="systems__actions">
                  <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => onRunNow(j)}>
                    {t.sys_job_run_now}
                  </button>
                  <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => onToggle(j)}>
                    {j.enabled ? t.sys_disable : t.sys_enable}
                  </button>
                  <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => setEditing(j)}>
                    {t.sys_edit}
                  </button>
                  <button
                    type="button"
                    className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                    aria-pressed={openRuns === j.id}
                    onClick={() => {
                      const next = openRuns === j.id ? null : j.id;
                      setOpenRuns(next);
                      if (next) onShowRuns(j);
                    }}
                  >
                    {t.sys_job_runs}
                  </button>
                  <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => onDelete(j)}>
                    {t.sys_delete}
                  </button>
                </div>
                {openRuns === j.id && (
                  <ul data-testid="job-runs">
                    {list.length === 0 ? (
                      <li className="systems__muted">{t.sys_job_runs_empty}</li>
                    ) : (
                      list.map((r) => (
                        <li key={r.id}>
                          <span className={`systems__verdict systems__verdict--${r.status}`}>{statusLabel[r.status]}</span>{" "}
                          {new Date(r.started_ms).toLocaleString()} <span className="systems__muted">{r.summary}</span>
                        </li>
                      ))
                    )}
                  </ul>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

function JobForm({ system, initial, onSave, onCancel }: { system: SystemConnection; initial?: SystemJob; onSave: (job: NewJob, id?: string) => Promise<void>; onCancel: () => void }) {
  const { t } = useLocale();
  const spec = initial ? parseSpec(initial) : undefined;
  const [name, setName] = useState(initial?.name ?? "");
  const [goal, setGoal] = useState(initial?.goal ?? "");
  const [kind, setKind] = useState<JobKind>(initial?.kind ?? "daily");
  const [times, setTimes] = useState((spec?.times ?? ["09:00"]).join(", "));
  const [days, setDays] = useState<number[]>(spec?.days ?? [1, 2, 3, 4, 5]);
  const [every, setEvery] = useState(spec?.every_minutes || 60);
  const [error, setError] = useState<string | null>(null);

  const toggleDay = (d: number) => setDays((ds) => (ds.includes(d) ? ds.filter((x) => x !== d) : [...ds, d]));

  async function submit() {
    try {
      await onSave({ system_id: system.id, name: name.trim(), goal: goal.trim(), kind, spec: buildSpec(kind, times, days, every) }, initial?.id);
    } catch (e) {
      setError(typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));
    }
  }

  return (
    <form
      className="systems__form"
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      <label>
        {t.sys_job_name}
        <input value={name} onChange={(e) => setName(e.target.value)} />
      </label>
      <label>
        {t.sys_job_goal}
        <textarea rows={3} value={goal} onChange={(e) => setGoal(e.target.value)} />
      </label>
      <label>
        {t.sys_job_kind}
        <select value={kind} onChange={(e) => setKind(e.target.value as JobKind)}>
          <option value="daily">{t.sys_job_kind_daily}</option>
          <option value="weekly">{t.sys_job_kind_weekly}</option>
          <option value="interval">{t.sys_job_kind_interval}</option>
        </select>
      </label>
      {kind === "weekly" && (
        <fieldset className="systems__rules">
          <legend>{t.sys_job_days}</legend>
          {[1, 2, 3, 4, 5, 6, 7].map((d) => (
            <label key={d}>
              <input type="checkbox" checked={days.includes(d)} onChange={() => toggleDay(d)} /> {t.sys_job_weekday(d)}
            </label>
          ))}
        </fieldset>
      )}
      {kind !== "interval" ? (
        <label>
          {t.sys_job_times}
          <input value={times} onChange={(e) => setTimes(e.target.value)} />
        </label>
      ) : (
        <label>
          {t.sys_job_every}
          <input type="number" min={5} value={every} onChange={(e) => setEvery(Number(e.target.value))} />
        </label>
      )}
      {error && (
        <p className="systems__error" role="alert">
          {t.sys_error(error)}
        </p>
      )}
      <div className="systems__actions">
        <button type="submit" className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" disabled={!name.trim() || !goal.trim()}>
          {t.sys_save}
        </button>
        <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={onCancel}>
          {t.sys_cancel}
        </button>
      </div>
    </form>
  );
}
