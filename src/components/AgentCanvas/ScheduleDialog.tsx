import { useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import type { AgentRow } from "../../ipc/agents";
import { invokeAiComplete } from "../../ipc/ai";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { parseSpec, type NewSchedule, type ScheduleKind, type ScheduleSpec, type ScheduleView } from "../../ipc/schedules";
import { ModelPickerButton } from "../ModelPickerButton";
import { buildScheduleRefineMessage, parseScheduleReply, SCHEDULE_REFINE_SYSTEM } from "./scheduleRefine";

const MIN_INTERVAL = 5;
const MAX_TIMES = 12;
/** 跟工作卡、專案簡介的「AI 潤飾」共用模型偏好。 */
const REFINE_PROVIDER_KEY = "aiterm_refine_provider";

/** 新增／編輯一個定時器。純表單：儲存、刪除、啟停、立即執行都由呼叫端決定怎麼做。 */
export function ScheduleDialog({
  agents,
  initial,
  defaultAgentId,
  onSave,
  onDelete,
  onRunNow,
  onToggleEnabled,
  onClose,
}: {
  agents: AgentRow[];
  initial: ScheduleView | null;
  /** 新增時預先選好的角色（把定時器拖到角色方塊上時帶入）。 */
  defaultAgentId?: string;
  onSave: (s: NewSchedule) => Promise<void>;
  onDelete?: () => Promise<void>;
  onRunNow?: () => Promise<string>;
  onToggleEnabled?: (enabled: boolean) => Promise<void>;
  onClose: () => void;
}) {
  const { t } = useLocale();
  const targets = agents.filter((a) => a.kind === "coder" || a.kind === "verifier");
  const init = initial ? parseSpec(initial.schedule_json) : {};
  const [name, setName] = useState(initial?.name ?? "");
  const [agentId, setAgentId] = useState(initial?.agent_id ?? (targets.some((a) => a.id === defaultAgentId) ? defaultAgentId : undefined) ?? targets[0]?.id ?? "");
  const [instruction, setInstruction] = useState(initial?.instruction ?? "");
  const [kind, setKind] = useState<ScheduleKind>(initial?.schedule_kind ?? "daily");
  const [times, setTimes] = useState<string[]>(init.times?.length ? init.times : ["08:00"]);
  const [days, setDays] = useState<number[]>(init.days?.length ? init.days : [1, 2, 3, 4, 5]);
  const [every, setEvery] = useState<string>(String(init.every_minutes ?? 60));
  const [useWindow, setUseWindow] = useState<boolean>(!!init.window);
  const [from, setFrom] = useState(init.window?.from ?? "09:00");
  const [to, setTo] = useState(init.window?.to ?? "18:00");
  const [failuresFail, setFailuresFail] = useState(initial?.failures_count_as_failed ?? false);
  const [enabled, setEnabled] = useState(initial?.enabled ?? true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
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
    const draft = instruction.trim();
    if (!draft) return;
    setRefining(true);
    setError(null);
    try {
      const reply = await invokeAiComplete(
        [{ role: "user", content: buildScheduleRefineMessage(draft) }],
        SCHEDULE_REFINE_SYSTEM,
        "schedule-refine",
        providerId || undefined,
      );
      const refined = parseScheduleReply(reply.content ?? "");
      if (!refined) {
        setError(t.board_refine_err_empty);
        return;
      }
      if (providerId) localStorage.setItem(REFINE_PROVIDER_KEY, providerId);
      setBeforeRefine(instruction);
      setInstruction(refined);
    } catch (e) {
      fail(e);
    } finally {
      setRefining(false);
    }
  };

  const fail = (e: unknown) => setError(typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));

  /** 回傳規格，或一段說明哪裡不對的文字。 */
  const buildSpec = (): ScheduleSpec | string => {
    if (kind === "interval") {
      const n = Number(every);
      if (!Number.isFinite(n) || n < MIN_INTERVAL) return `${t.sched_every} ≥ ${MIN_INTERVAL} ${t.sched_minutes}`;
      if (useWindow && !(from < to)) return `${from} – ${to}`;
      return { every_minutes: n, window: useWindow ? { from, to } : null };
    }
    if (times.length === 0 || times.some((x) => !/^\d{2}:\d{2}$/.test(x))) return `${t.sched_times}: HH:MM`;
    if (kind === "weekly") {
      if (days.length === 0) return t.sched_days;
      return { times, days };
    }
    return { times };
  };

  const save = async () => {
    setError(null);
    if (!name.trim() || !instruction.trim() || !agentId) {
      setError(`${t.sched_name} / ${t.sched_instruction}`);
      return;
    }
    const spec = buildSpec();
    if (typeof spec === "string") {
      setError(spec);
      return;
    }
    setBusy(true);
    try {
      await onSave({
        name: name.trim(),
        agent_id: agentId,
        instruction: instruction.trim(),
        schedule_kind: kind,
        spec,
        failures_count_as_failed: failuresFail,
        canvas_x: initial?.canvas_x ?? null,
        canvas_y: initial?.canvas_y ?? null,
      });
      onClose();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };

  const setTime = (i: number, v: string) => setTimes((ts) => ts.map((x, j) => (j === i ? v : x)));

  return (
    <div className="task-dialog-backdrop">
      <div className="task-dialog ac-sched-dialog" role="dialog" aria-label={t.sched_title} data-testid="schedule-dialog">
        <h3 className="task-dialog-title">⏰ {t.sched_title}</h3>

        <label className="task-field">
          <span className="task-field-label">{t.sched_name}</span>
          <input className="task-field-input" aria-label={t.sched_name} value={name} onChange={(e) => setName(e.target.value)} />
        </label>

        <label className="task-field">
          <span className="task-field-label">{t.sched_agent}</span>
          <select className="task-field-input" aria-label={t.sched_agent} value={agentId} onChange={(e) => setAgentId(e.target.value)}>
            {targets.map((a) => (
              <option key={a.id} value={a.id}>
                {a.name}
              </option>
            ))}
          </select>
        </label>

        <div className="task-field">
          <label className="task-field-label" htmlFor="sched-instruction">
            {t.sched_instruction}
          </label>
          <div className="ac-sched-refine">
            <ModelPickerButton providers={providers} selectedId={providerId} onChange={setProviderId} disabled={refining} />
            <button
              type="button"
              className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
              data-testid="sched-refine"
              disabled={refining || busy || !instruction.trim()}
              onClick={() => void refine()}
            >
              {refining ? t.board_refine_busy : t.board_refine}
            </button>
            {beforeRefine !== null && (
              <button
                type="button"
                className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
                data-testid="sched-refine-undo"
                disabled={refining}
                onClick={() => {
                  setInstruction(beforeRefine);
                  setBeforeRefine(null);
                }}
              >
                {t.board_refine_undo}
              </button>
            )}
          </div>
          <textarea
            id="sched-instruction"
            className="task-field-input task-field-textarea"
            aria-label={t.sched_instruction}
            rows={4}
            value={instruction}
            disabled={refining}
            onChange={(e) => setInstruction(e.target.value)}
          />
          <span className="task-field-hint">{t.sched_instruction_hint}</span>
        </div>

        <label className="task-field">
          <span className="task-field-label">{t.sched_mode}</span>
          <select className="task-field-input" aria-label={t.sched_mode} value={kind} onChange={(e) => setKind(e.target.value as ScheduleKind)}>
            <option value="daily">{t.sched_mode_daily}</option>
            <option value="weekly">{t.sched_mode_weekly}</option>
            <option value="interval">{t.sched_mode_interval}</option>
          </select>
        </label>

        {kind === "weekly" && (
          <div className="task-field">
            <span className="task-field-label">{t.sched_days}</span>
            <div className="ac-sched-days">
              {t.sched_weekdays.map((label, i) => (
                <label key={label}>
                  <input
                    type="checkbox"
                    checked={days.includes(i + 1)}
                    onChange={(e) => setDays((d) => (e.target.checked ? [...d, i + 1].sort() : d.filter((x) => x !== i + 1)))}
                  />
                  {label}
                </label>
              ))}
            </div>
          </div>
        )}

        {kind !== "interval" && (
          <div className="task-field">
            <span className="task-field-label">{t.sched_times}</span>
            {times.map((tm, i) => (
              <div key={i} className="ac-sched-time">
                <input type="time" className="task-field-input" aria-label={`${t.sched_times} ${i + 1}`} value={tm} onChange={(e) => setTime(i, e.target.value)} />
                {times.length > 1 && (
                  <button type="button" className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={() => setTimes((ts) => ts.filter((_, j) => j !== i))}>
                    ✕
                  </button>
                )}
              </div>
            ))}
            {times.length < MAX_TIMES && (
              <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm ac-sched-addtime" onClick={() => setTimes((ts) => [...ts, "12:00"])}>
                {t.sched_add_time}
              </button>
            )}
          </div>
        )}

        {kind === "interval" && (
          <div className="task-field">
            <span className="task-field-label">{t.sched_every}</span>
            <div className="ac-sched-time">
              <input type="number" className="task-field-input" min={MIN_INTERVAL} aria-label={t.sched_minutes} value={every} onChange={(e) => setEvery(e.target.value)} />
              <span>{t.sched_minutes}</span>
            </div>
            <label className="ac-sched-check">
              <input type="checkbox" checked={useWindow} onChange={(e) => setUseWindow(e.target.checked)} /> {t.sched_window}
            </label>
            {useWindow && (
              <div className="ac-sched-time">
                <input type="time" className="task-field-input" aria-label="from" value={from} onChange={(e) => setFrom(e.target.value)} />
                <span>–</span>
                <input type="time" className="task-field-input" aria-label="to" value={to} onChange={(e) => setTo(e.target.value)} />
              </div>
            )}
          </div>
        )}

        <label className="ac-sched-check">
          <input type="checkbox" checked={failuresFail} onChange={(e) => setFailuresFail(e.target.checked)} /> {t.sched_failures_fail}
        </label>

        {initial && onToggleEnabled && (
          <label className="ac-sched-check">
            <input
              type="checkbox"
              checked={enabled}
              onChange={(e) => {
                const v = e.target.checked;
                setEnabled(v);
                onToggleEnabled(v).catch((err) => {
                  setEnabled(!v);
                  fail(err);
                });
              }}
            />{" "}
            {t.sched_enabled}
          </label>
        )}

        {initial && (
          <div className="task-field">
            <span className="task-field-label">{t.sched_runs}</span>
            {initial.runs.length === 0 ? (
              <span className="task-field-hint">{t.sched_no_runs}</span>
            ) : (
              <ul className="ac-sched-runs">
                {initial.runs.map((r, i) => (
                  <li key={i}>
                    {new Date(r.fired_at * 1000).toLocaleString()} —{" "}
                    {r.result === "success" ? t.sched_run_success : r.result === "failed" ? t.sched_run_failed : t.sched_run_skipped}
                  </li>
                ))}
              </ul>
            )}
          </div>
        )}

        {error && <div className="task-dialog-error">{error}</div>}
        {note && <div className="task-field-hint">{note}</div>}

        <div className="task-dialog-actions">
          {initial && onDelete && (
            <button
              type="button"
              className="aiterm-btn aiterm-btn--danger-solid"
              disabled={busy}
              onClick={() => void onDelete().then(onClose).catch(fail)}
            >
              {t.sched_delete}
            </button>
          )}
          {initial && onRunNow && (
            <button
              type="button"
              className="aiterm-btn aiterm-btn--secondary"
              disabled={busy}
              onClick={() =>
                void onRunNow()
                  .then((id) => setNote(id ? t.sched_run_now_started : t.sched_run_now_skipped))
                  .catch(fail)
              }
            >
              {t.sched_run_now}
            </button>
          )}
          <button type="button" className="aiterm-btn aiterm-btn--secondary" onClick={onClose}>
            {t.common_cancel}
          </button>
          <button type="button" className="aiterm-btn aiterm-btn--primary" disabled={busy} onClick={() => void save()}>
            {t.sched_save}
          </button>
        </div>
      </div>
    </div>
  );
}
