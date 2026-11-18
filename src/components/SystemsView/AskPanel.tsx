import { useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import type { AdhocReport, SystemConnection } from "../../ipc/systems";
import { GoalRefineBar } from "./GoalRefineBar";
import { ModelSelect } from "./ModelSelect";
import { modelName } from "./utils";

interface Props {
  system: SystemConnection;
  onRun: (goal: string, model: string | null) => Promise<AdhocReport>;
}

const goalsKey = (systemId: string) => `aiterm.systems.goals.${systemId}`;

function loadGoals(systemId: string): string[] {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(goalsKey(systemId)) ?? "[]");
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}

const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));

/** 「請 AI 操作」：打一句話讓 AI 在這個系統上做事；結果直接顯示在下方。 */
export function AskPanel({ system, onRun }: Props) {
  const { t } = useLocale();
  const [goal, setGoal] = useState("");
  const [savedGoals, setSavedGoals] = useState<string[]>(() => loadGoals(system.id));
  const [model, setModel] = useState<string | null>(null);
  const [refining, setRefining] = useState(false);
  const [running, setRunning] = useState(false);
  const [report, setReport] = useState<AdhocReport | null>(null);
  const [error, setError] = useState<string | null>(null);

  const toolLabel = (tool: string) => {
    const key = `sys_tool_${tool.replace(/^sys_/, "")}` as keyof typeof t;
    const v = t[key];
    return typeof v === "string" ? v : tool;
  };

  function persistGoals(next: string[]) {
    setSavedGoals(next);
    try {
      localStorage.setItem(goalsKey(system.id), JSON.stringify(next));
    } catch {
      /* 儲存空間不可用時只留在畫面上 */
    }
  }

  async function start() {
    setRunning(true);
    setError(null);
    setReport(null);
    try {
      const r = await onRun(goal.trim(), model);
      setReport(r);
    } catch (e) {
      setError(errText(e));
    } finally {
      setRunning(false);
    }
  }

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
        {savedGoals.length > 0 && (
          <div className="systems__chips" aria-label={t.sys_goal_saved} data-testid="saved-goals">
            {savedGoals.map((g) => (
              <span key={g} className={`systems__chip${goal === g ? " is-on" : ""}`}>
                <button type="button" className="systems__chip-x" title={g} disabled={running} onClick={() => setGoal(g)}>
                  {g.replace(/\s+/g, " ").slice(0, 24)}
                </button>
                <button type="button" className="systems__chip-x" aria-label={t.sys_goal_remove} title={t.sys_goal_remove} onClick={() => persistGoals(savedGoals.filter((x) => x !== g))}>
                  ✕
                </button>
              </span>
            ))}
          </div>
        )}
        <p className="systems__muted">{t.sys_ask_hint}</p>
        <label className="systems__asklabel systems__askmodel">
          {t.sys_model}
          <ModelSelect value={model} onChange={setModel} disabled={running} inheritLabel={t.sys_model_inherit_system(modelName(t, system.model))} />
        </label>
        <div className="systems__askbar">
          <button
            type="button"
            className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
            disabled={running || !goal.trim() || savedGoals.includes(goal.trim())}
            onClick={() => persistGoals([...savedGoals, goal.trim()])}
          >
            {t.sys_goal_save}
          </button>
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

        </div>
      )}
    </section>
  );
}
