import { useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { invokeAiComplete } from "../../ipc/ai";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import type { JobKind, JobRun, NewJob, SystemConnection, SystemJob } from "../../ipc/systems";
import { ModelPickerButton } from "../ModelPickerButton";
import { buildGoalRefineMessage, GOAL_REFINE_SYSTEM, parseGoalReply } from "./goalRefine";
import { buildSpec, parseSpec, scheduleText } from "./jobSchedule";

/** 跟工作卡、專案簡介、定時器的「AI 潤飾」共用模型偏好。 */
const REFINE_PROVIDER_KEY = "aiterm_refine_provider";

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
                <div className="systems__jobhead">
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
  const [times, setTimes] = useState<string[]>(spec?.times.length ? spec.times : ["09:00"]);
  const [newTime, setNewTime] = useState("12:00");
  const [days, setDays] = useState<number[]>(spec?.days ?? [1, 2, 3, 4, 5]);
  const [every, setEvery] = useState(spec?.every_minutes || 60);
  const [error, setError] = useState<string | null>(null);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [providerId, setProviderId] = useState("");
  const [refining, setRefining] = useState(false);
  /** 潤飾前的內容；有值才顯示「還原」。 */
  const [beforeRefine, setBeforeRefine] = useState<string | null>(null);

  // 沿用其他「AI 潤飾」的慣例：記住上次選的 → is_default → 清單第一個。
  useEffect(() => {
    let alive = true;
    listProviders()
      .then((list) => {
        if (!alive) return;
        setProviders(list);
        const remembered = localStorage.getItem(REFINE_PROVIDER_KEY);
        const fallback = list.find((p) => p.is_default)?.id ?? list[0]?.id ?? "";
        setProviderId(remembered && list.some((p) => p.id === remembered) ? remembered : fallback);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  const refine = async () => {
    const draft = goal.trim();
    if (!draft) return;
    setRefining(true);
    setError(null);
    try {
      const reply = await invokeAiComplete(
        [{ role: "user", content: buildGoalRefineMessage(draft, system.name) }],
        GOAL_REFINE_SYSTEM,
        "system-job-goal-refine",
        providerId || undefined,
      );
      const refined = parseGoalReply(reply.content ?? "");
      if (!refined) {
        setError(t.board_refine_err_empty);
        return;
      }
      if (providerId) localStorage.setItem(REFINE_PROVIDER_KEY, providerId);
      setBeforeRefine(goal);
      setGoal(refined);
    } catch (e) {
      setError(typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));
    } finally {
      setRefining(false);
    }
  };

  const toggleDay = (d: number) => setDays((ds) => (ds.includes(d) ? ds.filter((x) => x !== d) : [...ds, d]));
  const addTime = () => {
    if (/^\d{2}:\d{2}$/.test(newTime) && !times.includes(newTime)) setTimes((ts) => [...ts, newTime].sort());
  };

  const built = buildSpec(kind, times.join(","), days, every);
  const needsTimes = kind !== "interval" && times.length === 0;
  const needsDays = kind === "weekly" && days.length === 0;
  const preview = needsTimes || needsDays ? null : scheduleText(kind, built, t);
  const canSave = goal.trim() !== "" && !needsTimes && !needsDays && (kind !== "interval" || every >= 5);

  async function submit() {
    // 名稱留空時用目標的前幾個字，少一個一定要填的欄位。
    const finalName = name.trim() || goal.trim().replace(/\s+/g, " ").slice(0, 16);
    try {
      await onSave({ system_id: system.id, name: finalName, goal: goal.trim(), kind, spec: built }, initial?.id);
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
        {t.sys_job_goal}
        <textarea rows={4} value={goal} placeholder={t.sys_job_goal_ph} disabled={refining} onChange={(e) => setGoal(e.target.value)} />
        <small>{t.sys_job_goal_hint}</small>
      </label>
      <div className="systems__refinebar">
        <ModelPickerButton providers={providers} selectedId={providerId} onChange={setProviderId} disabled={refining} />
        <button type="button" className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" data-testid="goal-refine" disabled={refining || !goal.trim()} onClick={() => void refine()}>
          {refining ? t.board_refine_busy : t.board_refine}
        </button>
        {beforeRefine !== null && (
          <button
            type="button"
            className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
            data-testid="goal-refine-undo"
            disabled={refining}
            onClick={() => {
              setGoal(beforeRefine);
              setBeforeRefine(null);
            }}
          >
            {t.board_refine_undo}
          </button>
        )}
      </div>
      <label>
        {t.sys_job_name}
        <input value={name} placeholder={t.sys_job_name_ph} onChange={(e) => setName(e.target.value)} />
      </label>

      <div className="systems__field-block">
        <div className="systems__field-label">{t.sys_job_kind}</div>
        <div className="systems__tabs" role="radiogroup" aria-label={t.sys_job_kind}>
          {(["daily", "weekly", "interval"] as const).map((k) => (
            <button key={k} type="button" role="radio" aria-checked={kind === k} className={kind === k ? "is-on" : ""} onClick={() => setKind(k)}>
              {k === "daily" ? t.sys_job_kind_daily : k === "weekly" ? t.sys_job_kind_weekly : t.sys_job_kind_interval}
            </button>
          ))}
        </div>
      </div>

      {kind === "weekly" && (
        <div className="systems__field-block">
          <div className="systems__field-label">{t.sys_job_days}</div>
          <div className="systems__chips">
            {[1, 2, 3, 4, 5, 6, 7].map((d) => (
              <button key={d} type="button" className={`systems__chip${days.includes(d) ? " is-on" : ""}`} aria-pressed={days.includes(d)} onClick={() => toggleDay(d)}>
                {t.sys_job_weekday(d)}
              </button>
            ))}
          </div>
        </div>
      )}

      {kind !== "interval" ? (
        <div className="systems__field-block">
          <div className="systems__field-label">{t.sys_job_times}</div>
          <div className="systems__chips">
            {times.map((x) => (
              <span key={x} className="systems__chip is-on" data-testid="job-time">
                {x}
                <button type="button" className="systems__chip-x" aria-label={t.sys_job_time_remove(x)} onClick={() => setTimes((ts) => ts.filter((y) => y !== x))}>
                  ✕
                </button>
              </span>
            ))}
            <input type="time" className="systems__time" aria-label={t.sys_job_times} value={newTime} onChange={(e) => setNewTime(e.target.value)} />
            <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={addTime}>
              {t.sys_job_add_time}
            </button>
          </div>
          {needsTimes && <small className="systems__field-error">{t.sys_job_no_times}</small>}
        </div>
      ) : (
        <div className="systems__field-block">
          <div className="systems__field-label">{t.sys_job_every}</div>
          <div className="systems__chips">
            <input type="number" min={5} className="systems__num" value={every} onChange={(e) => setEvery(Number(e.target.value))} />
            <span>{t.sys_job_unit_min}</span>
          </div>
          <small className="systems__muted">{t.sys_job_every_hint}</small>
        </div>
      )}

      {preview && (
        <p className="systems__preview" data-testid="job-preview">
          {t.sys_job_preview(preview)}
        </p>
      )}
      {(system.mode === "read_only" || system.mode === "approve_each") && (
        <p className="systems__warn" role="alert">
          ⚠ {system.mode === "read_only" ? t.sys_job_unattended_read_only : t.sys_job_unattended_approve}
        </p>
      )}
      {error && (
        <p className="systems__error" role="alert">
          {t.sys_error(error)}
        </p>
      )}
      <div className="systems__actions">
        <button type="submit" className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" disabled={!canSave}>
          {t.sys_save}
        </button>
        <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={onCancel}>
          {t.sys_cancel}
        </button>
      </div>
    </form>
  );
}
