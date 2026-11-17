import { useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import type { Recipe } from "../../ipc/systems";

interface Props {
  recipes: Recipe[];
  onDelete: (r: Recipe) => void;
  /** 帶入參數值重播；回傳結果文字。 */
  onRun: (r: Recipe, values: Record<string, string>) => Promise<string>;
}

/** 這個系統已存的配方：名稱、說明、參數、步數、成功／失敗次數，可展開看步驟與刪除。 */
export function RecipeList({ recipes, onDelete, onRun }: Props) {
  const { t } = useLocale();
  const [open, setOpen] = useState<number | null>(null);
  const [runFor, setRunFor] = useState<number | null>(null);
  const [values, setValues] = useState<Record<string, string>>({});
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null);

  const startRun = (r: Recipe) => {
    const next = runFor === r.id ? null : r.id;
    setRunFor(next);
    setValues({});
    setResult(null);
  };

  const run = async (r: Recipe) => {
    setRunning(true);
    setResult(null);
    try {
      setResult({ ok: true, text: await onRun(r, values) });
    } catch (e) {
      setResult({ ok: false, text: typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e) });
    } finally {
      setRunning(false);
    }
  };
  return (
    <section className="systems__audit">
      {recipes.length === 0 ? (
        <p className="systems__muted">{t.sys_recipes_empty}</p>
      ) : (
        <ul>
          {recipes.map((r) => (
            <li key={r.id} data-testid="recipe-row" className="systems__recipe">
              <div className="systems__head">
                <strong>{r.name}</strong>
                <span className="systems__badge">{t.sys_recipe_steps(r.steps.length)}</span>
                <span className="systems__muted">{t.sys_recipe_stats(r.ok_count, r.fail_count)}</span>
              </div>
              {r.description && <div className="systems__muted">{r.description}</div>}
              {r.params.length > 0 && (
                <div className="systems__muted">
                  {t.sys_recipe_params}：{r.params.map((p) => p.name).join("、")}
                </div>
              )}
              <div className="systems__actions">
                <button type="button" className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" data-testid="recipe-run" aria-pressed={runFor === r.id} onClick={() => startRun(r)}>
                  {t.sys_recipe_run}
                </button>
                <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" aria-pressed={open === r.id} onClick={() => setOpen(open === r.id ? null : r.id)}>
                  {t.sys_recipe_show_steps}
                </button>
                <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => onDelete(r)}>
                  {t.sys_delete}
                </button>
              </div>
              {runFor === r.id && (
                <form
                  className="systems__form systems__runform"
                  onSubmit={(e) => {
                    e.preventDefault();
                    if (!running) void run(r);
                  }}
                >
                  {r.params.length === 0 ? (
                    <p className="systems__muted">{t.sys_recipe_run_none}</p>
                  ) : (
                    r.params.map((p) => (
                      <label key={p.name}>
                        {p.name}
                        <input value={values[p.name] ?? ""} placeholder={p.example} disabled={running} onChange={(e) => setValues((v) => ({ ...v, [p.name]: e.target.value }))} />
                      </label>
                    ))
                  )}
                  <div className="systems__actions">
                    <button type="submit" className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" disabled={running || r.params.some((p) => !(values[p.name] ?? "").trim())}>
                      {running ? t.sys_recipe_running : t.sys_recipe_run_go}
                    </button>
                  </div>
                  {result && (
                    <p className={result.ok ? "systems__preview" : "systems__error"} role="status" data-testid="recipe-run-result">
                      {result.text}
                    </p>
                  )}
                </form>
              )}
              {open === r.id && (
                <ol data-testid="recipe-steps">
                  {r.steps.map((s, i) => (
                    <li key={i}>
                      <code>{s.tool}</code> {s.intent}
                    </li>
                  ))}
                </ol>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
