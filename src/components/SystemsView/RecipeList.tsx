import { useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import type { Recipe } from "../../ipc/systems";

interface Props {
  recipes: Recipe[];
  onDelete: (r: Recipe) => void;
}

/** 這個系統已存的配方：名稱、說明、參數、步數、成功／失敗次數，可展開看步驟與刪除。 */
export function RecipeList({ recipes, onDelete }: Props) {
  const { t } = useLocale();
  const [open, setOpen] = useState<number | null>(null);
  return (
    <section className="systems__audit">
      <h3>{t.sys_recipes_title}</h3>
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
                <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" aria-pressed={open === r.id} onClick={() => setOpen(open === r.id ? null : r.id)}>
                  {t.sys_recipe_show_steps}
                </button>
                <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => onDelete(r)}>
                  {t.sys_delete}
                </button>
              </div>
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
