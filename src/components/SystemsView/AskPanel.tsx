import { useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import type { AdhocReport, SystemConnection } from "../../ipc/systems";
import { GoalRefineBar } from "./GoalRefineBar";

interface Param {
  name: string;
  example: string;
  use: boolean;
}

interface Props {
  system: SystemConnection;
  onRun: (goal: string) => Promise<AdhocReport>;
  onSaveRecipe: (name: string, description: string, params: { name: string; example: string }[]) => Promise<string>;
  onDiscard: () => Promise<void>;
}

const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));

/** 「請 AI 操作」：打一句話讓 AI 在這個系統上做事；成功後可以把剛才的步驟存成操作流程。 */
export function AskPanel({ system, onRun, onSaveRecipe, onDiscard }: Props) {
  const { t } = useLocale();
  const [goal, setGoal] = useState("");
  const [refining, setRefining] = useState(false);
  const [running, setRunning] = useState(false);
  const [report, setReport] = useState<AdhocReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  // 存操作流程的表單
  const [name, setName] = useState("");
  const [desc, setDesc] = useState("");
  const [params, setParams] = useState<Param[]>([]);

  const toolLabel = (tool: string) => {
    const key = `sys_tool_${tool.replace(/^sys_/, "")}` as keyof typeof t;
    const v = t[key];
    return typeof v === "string" ? v : tool;
  };

  async function start() {
    setRunning(true);
    setError(null);
    setSaved(null);
    setReport(null);
    try {
      const r = await onRun(goal.trim());
      setReport(r);
      setParams(r.params.map((p) => ({ ...p, use: true })));
      setName("");
      setDesc("");
    } catch (e) {
      setError(errText(e));
    } finally {
      setRunning(false);
    }
  }

  async function save() {
    setError(null);
    try {
      const chosen = params.filter((p) => p.use && p.name.trim()).map((p) => ({ name: p.name.trim(), example: p.example }));
      await onSaveRecipe(name.trim(), desc.trim(), chosen);
      setSaved(name.trim());
      setReport(null);
    } catch (e) {
      setError(errText(e));
    }
  }

  async function skip() {
    setError(null);
    try {
      await onDiscard();
      setReport(null);
    } catch (e) {
      setError(errText(e));
    }
  }

  const setParam = (i: number, patch: Partial<Param>) => setParams((ps) => ps.map((p, j) => (j === i ? { ...p, ...patch } : p)));

  return (
    <section className="systems__audit systems__ask">
      <form
        className="systems__askform"
        onSubmit={(e) => {
          e.preventDefault();
          if (goal.trim() && !running) void start();
        }}
      >
        <label className="systems__asklabel">
          {t.sys_ask_goal}
          <textarea rows={3} value={goal} placeholder={t.sys_ask_goal_ph} disabled={running || refining} onChange={(e) => setGoal(e.target.value)} />
        </label>
        <p className="systems__muted">{t.sys_ask_hint}</p>
        <div className="systems__askbar">
          <GoalRefineBar systemName={system.name} goal={goal} onGoal={setGoal} onBusy={setRefining} disabled={running} onError={setError} />
          <button type="submit" className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" disabled={running || refining || !goal.trim()}>
            {t.sys_ask_send}
          </button>
        </div>
      </form>

      {running && (
        <p className="systems__hint" role="status" data-testid="ask-running">
          {t.sys_ask_running}
        </p>
      )}
      {error && (
        <p className="systems__error" role="alert">
          {t.sys_error(error)}
        </p>
      )}
      {saved && (
        <p className="systems__preview" role="status" data-testid="ask-saved">
          {t.sys_save_done(saved)}
        </p>
      )}

      {report && (
        <div className="systems__result" data-testid="ask-result">
          <div className="systems__jobhead">
            <span className={`systems__verdict systems__verdict--${report.status === "ok" ? "ok" : "failed"}`}>
              {report.status === "ok" ? t.sys_ask_ok : t.sys_ask_failed}
            </span>
          </div>
          <div className="systems__subtitle">{t.sys_ask_report}</div>
          <p className="systems__report">{report.summary}</p>

          <div className="systems__subtitle">{t.sys_ask_steps}</div>
          {report.steps.length === 0 ? (
            <p className="systems__muted">{t.sys_ask_no_steps}</p>
          ) : (
            <ol className="systems__steps" data-testid="ask-steps">
              {report.steps.map((s, i) => (
                <li key={i}>
                  <span className={`systems__risk systems__risk--${s.write ? "warn" : "safe"}`}>{s.write ? t.sys_step_write : t.sys_step_read}</span>
                  <strong>{toolLabel(s.tool)}</strong>
                  {s.detail && <span className="systems__muted">{s.detail}</span>}
                </li>
              ))}
            </ol>
          )}

          {report.can_save && (
            <form
              className="systems__form systems__saveform"
              data-testid="ask-save"
              onSubmit={(e) => {
                e.preventDefault();
                if (name.trim()) void save();
              }}
            >
              <div className="systems__subtitle">{t.sys_save_title}</div>
              <p className="systems__muted">{t.sys_save_hint}</p>
              <label>
                {t.sys_save_name}
                <input value={name} placeholder={t.sys_save_name_ph} onChange={(e) => setName(e.target.value)} />
              </label>
              <label>
                {t.sys_save_desc}
                <input value={desc} onChange={(e) => setDesc(e.target.value)} />
              </label>
              <div className="systems__field-block">
                <div className="systems__field-label">{t.sys_save_params}</div>
                {params.length === 0 ? (
                  <p className="systems__muted">{t.sys_save_no_params}</p>
                ) : (
                  <div className="systems__params">
                    <p className="systems__muted">{t.sys_save_params_hint}</p>
                    {params.map((p, i) => (
                      <div key={i} className="systems__param" data-testid="ask-param">
                        <input type="checkbox" aria-label={p.name} checked={p.use} onChange={(e) => setParam(i, { use: e.target.checked })} />
                        <input aria-label={`${t.sys_save_params} ${i + 1}`} value={p.name} disabled={!p.use} onChange={(e) => setParam(i, { name: e.target.value })} />
                        <span className="systems__muted">{t.sys_save_param_example(p.example)}</span>
                      </div>
                    ))}
                  </div>
                )}
              </div>
              <div className="systems__actions">
                <button type="submit" className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" disabled={!name.trim()}>
                  {t.sys_save_btn}
                </button>
                <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => void skip()}>
                  {t.sys_save_skip}
                </button>
              </div>
            </form>
          )}
        </div>
      )}
    </section>
  );
}
