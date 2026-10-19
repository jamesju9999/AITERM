import { useEffect } from "react";

import { useLocale } from "../../../contexts/LocaleContext";
import "./guide.css";

/** 說明面板：流程、角色、狀態、常見問題，以及重看導覽／打開清單／提示開關。 */
export function GuideHelp({
  hintsOn,
  onToggleHints,
  onReplayTour,
  onShowChecklist,
  onResetProgress,
  onClose,
}: {
  hintsOn: boolean;
  onToggleHints: (on: boolean) => void;
  onReplayTour: () => void;
  onShowChecklist: () => void;
  /** 重新學習：清空上手清單的進度並打開它。 */
  onResetProgress: () => void;
  onClose: () => void;
}) {
  const { t } = useLocale();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const pairs = (rows: readonly (readonly [string, string])[]) => (
    <dl className="ac-help-dl">
      {rows.map(([k, v]) => (
        <div key={k}>
          <dt>{k}</dt>
          <dd>{v}</dd>
        </div>
      ))}
    </dl>
  );

  return (
    <div className="task-dialog-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="task-dialog ac-help" role="dialog" aria-label={t.ac_help_title}>
        <h3 className="task-dialog-title">{t.ac_help_title}</h3>
        <div className="ac-help-actions">
          <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={onReplayTour}>
            {t.ac_help_replay}
          </button>
          <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={onShowChecklist}>
            {t.ac_help_checklist}
          </button>
          <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={onResetProgress}>
            {t.ac_check_reset}
          </button>
          <label className="ac-help-toggle">
            <input type="checkbox" checked={hintsOn} onChange={(e) => onToggleHints(e.target.checked)} />
            {t.ac_help_hints}
          </label>
        </div>
        <div className="ac-help-body">
          <section>
            <h4>{t.ac_help_flow_title}</h4>
            <ol className="ac-help-flow">
              {t.ac_help_flow.map((s) => (
                <li key={s}>{s}</li>
              ))}
            </ol>
          </section>
          <section>
            <h4>{t.ac_help_roles_title}</h4>
            {pairs(t.ac_help_roles)}
          </section>
          <section>
            <h4>{t.ac_help_status_title}</h4>
            {pairs(t.ac_help_status)}
          </section>
          <section>
            <h4>{t.ac_help_faq_title}</h4>
            {pairs(t.ac_help_faq)}
          </section>
        </div>
        <div className="task-dialog-actions">
          <button className="aiterm-btn aiterm-btn--secondary" onClick={onClose}>
            {t.ac_help_close}
          </button>
        </div>
      </div>
    </div>
  );
}
