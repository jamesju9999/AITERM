import { useState } from "react";

import { useLocale } from "../../../contexts/LocaleContext";
import { STEP_IDS, type StepId } from "./guideSteps";
import "./guide.css";

/** 上手清單：浮在桌布右上角，做完一步自己打勾、亮出下一步怎麼做。 */
export function GuideChecklist({ done, onHide }: { done: readonly StepId[]; onHide: () => void }) {
  const { t } = useLocale();
  const [collapsed, setCollapsed] = useState(false);
  const label: Record<StepId, [string, string]> = {
    coder: [t.ac_check_coder, t.ac_check_coder_how],
    module: [t.ac_check_module, t.ac_check_module_how],
    assign: [t.ac_check_assign, t.ac_check_assign_how],
    reviewer: [t.ac_check_reviewer, t.ac_check_reviewer_how],
    orchestrator: [t.ac_check_orchestrator, t.ac_check_orchestrator_how],
    verifier: [t.ac_check_verifier, t.ac_check_verifier_how],
    merged: [t.ac_check_merged, t.ac_check_merged_how],
    verified: [t.ac_check_verified, t.ac_check_verified_how],
  };
  const count = STEP_IDS.filter((s) => done.includes(s)).length;
  const current = STEP_IDS.find((s) => !done.includes(s));

  if (collapsed) {
    return (
      <button
        className="ac-steps ac-steps--pill"
        data-testid="ac-checklist"
        aria-label={t.ac_check_expand}
        title={t.ac_check_expand}
        onClick={() => setCollapsed(false)}
      >
        {t.ac_check_title} {t.ac_check_count(count, STEP_IDS.length)}
      </button>
    );
  }
  return (
    <div className="ac-steps" data-testid="ac-checklist" role="region" aria-label={t.ac_check_title}>
      <div className="ac-steps-head">
        <b>{t.ac_check_title}</b>
        <span className="ac-steps-count">{t.ac_check_count(count, STEP_IDS.length)}</span>
        <span className="ac-spacer" />
        <button className="ac-iconbtn" aria-label={t.ac_check_collapse} title={t.ac_check_collapse} onClick={() => setCollapsed(true)}>
          –
        </button>
        <button className="ac-iconbtn" aria-label={t.ac_check_hide} title={t.ac_check_hide} onClick={onHide}>
          ×
        </button>
      </div>
      <ol className="ac-steps-list">
        {STEP_IDS.map((s) => {
          const state = done.includes(s) ? "done" : s === current ? "now" : "todo";
          return (
            <li key={s} className={`ac-steps-item ac-steps-item--${state}`} data-step={s} data-state={state}>
              <span className="ac-steps-mark" aria-hidden>
                {state === "done" ? "✓" : state === "now" ? "→" : "○"}
              </span>
              <span>
                {label[s][0]}
                {state === "now" && <small>{label[s][1]}</small>}
              </span>
            </li>
          );
        })}
      </ol>
      {!current && (
        <div className="ac-steps-done">
          <span>{t.ac_check_all_done}</span>
          <button className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" onClick={onHide}>
            {t.ac_check_finish}
          </button>
        </div>
      )}
    </div>
  );
}
