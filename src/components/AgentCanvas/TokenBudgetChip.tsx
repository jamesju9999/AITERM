import { useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { formatTokens, totalTokens, usageBreakdown, type UsageSum } from "./tokenUsage";

/**
 * 頂列的 Σ token：顯示專案用了多少；點開可以設上限（用到上限後 Agent 卡先不派工）。
 */
export function TokenBudgetChip({
  sum,
  budget,
  onSave,
}: {
  sum: UsageSum | null;
  budget: number | null;
  onSave: (budget: number | null) => Promise<void>;
}) {
  const { t } = useLocale();
  const [open, setOpen] = useState(false);
  const [text, setText] = useState("");
  const used = sum ? totalTokens(sum.usage) : 0;
  const over = budget !== null && used >= budget;
  const label = budget !== null ? t.ac_tokens_of(formatTokens(used), formatTokens(budget)) : t.ac_tokens(formatTokens(used));

  const start = () => {
    setText(budget !== null ? String(budget / 1_000_000) : "");
    setOpen(!open);
  };
  const save = async (value: number | null) => {
    await onSave(value);
    setOpen(false);
  };
  const parsed = Number(text);
  const valid = text.trim() !== "" && Number.isFinite(parsed) && parsed > 0;

  return (
    <span className="ac-budget">
      <button
        className={`ac-tokens ac-tokens--btn${over ? " ac-tokens--over" : ""}`}
        data-testid="ac-tokens"
        title={[sum ? t.ac_tokens_tip(usageBreakdown(sum)) : "", t.ac_budget_hint].filter(Boolean).join("\n")}
        onClick={start}
      >
        {label}
        {over && ` · ${t.ac_budget_over}`}
      </button>
      {open && (
        <span className="ac-budget-pop" role="dialog" aria-label={t.ac_budget_title}>
          <label>
            {t.ac_budget_title}
            <input
              type="number"
              min="0"
              step="0.5"
              value={text}
              autoFocus
              aria-label={t.ac_budget_title}
              onChange={(e) => setText(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && valid) void save(Math.round(parsed * 1_000_000));
                if (e.key === "Escape") setOpen(false);
              }}
            />
            M
          </label>
          <button className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" disabled={!valid} onClick={() => void save(Math.round(parsed * 1_000_000))}>
            {t.ac_budget_save}
          </button>
          {budget !== null && (
            <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={() => void save(null)}>
              {t.ac_budget_clear}
            </button>
          )}
        </span>
      )}
    </span>
  );
}
