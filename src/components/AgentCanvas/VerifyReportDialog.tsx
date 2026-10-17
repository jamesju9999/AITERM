import { useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { verifyReport, type VerifyItem, type VerifyReportView } from "../../ipc/agents";

const STATUS_ICON: Record<VerifyItem["status"], string> = { pass: "✓", fail: "✗", manual: "?" };

/**
 * 驗收報告：每條驗收條件的結果、重現步驟、證據截圖。驗收要你決定時可以在這裡放行。
 */
export function VerifyReportDialog({
  projectId,
  taskId,
  canAccept,
  onAccept,
  onClose,
}: {
  projectId: string;
  taskId: string;
  /** 驗收卡判定是 attention（要你決定）時顯示「放行」。 */
  canAccept: boolean;
  onAccept: () => void;
  onClose: () => void;
}) {
  const { t } = useLocale();
  const [view, setView] = useState<VerifyReportView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [zoom, setZoom] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    verifyReport(projectId, taskId)
      .then((v) => alive && setView(v))
      .catch((e) => alive && setError(String(e)));
    return () => {
      alive = false;
    };
  }, [projectId, taskId]);

  const items = view?.report?.items ?? [];
  const count = (s: VerifyItem["status"]) => items.filter((i) => i.status === s).length;
  const label = { pass: t.ac_verify_pass_item, fail: t.ac_verify_fail_item, manual: t.ac_verify_manual_item };
  // 沒通過的放最前面，再來是要你確認的。
  const order = { fail: 0, manual: 1, pass: 2 };
  const sorted = [...items].sort((a, b) => order[a.status] - order[b.status]);

  return (
    <div className="task-dialog-backdrop">
      <div className="task-dialog" role="dialog" aria-label={t.ac_verify_title}>
        <h3 className="task-dialog-title">{t.ac_verify_title}</h3>
        {error && <p className="ac-bad-text">{error}</p>}
        {view && !view.report && <p className="ac-dialog-desc">{t.ac_verify_none}</p>}
        {view?.report && (
          <>
            <p className="ac-dialog-desc" data-testid="verify-summary">
              {t.ac_verify_summary(count("pass"), count("fail"), count("manual"))}
              {view.report.project_type ? ` · ${view.report.project_type}` : ""}
            </p>
            <div className="ac-verify-list">
              {sorted.map((item, i) => (
                <div key={i} className={`ac-verify-item ac-verify-item--${item.status}`} data-testid={`verify-item-${item.status}`}>
                  <div className="ac-verify-head">
                    <span aria-label={label[item.status]} title={label[item.status]}>
                      {STATUS_ICON[item.status]}
                    </span>
                    <span>{item.criterion}</span>
                  </div>
                  {item.steps && (
                    <div className="ac-verify-field">
                      <b>{t.ac_verify_steps}</b>
                      {item.steps}
                    </div>
                  )}
                  {item.expected && (
                    <div className="ac-verify-field">
                      <b>{t.ac_verify_expected}</b>
                      {item.expected}
                    </div>
                  )}
                  {item.actual && (
                    <div className="ac-verify-field">
                      <b>{t.ac_verify_actual}</b>
                      {item.actual}
                    </div>
                  )}
                  {item.notes && (
                    <div className="ac-verify-field">
                      <b>{t.ac_verify_notes}</b>
                      {item.notes}
                    </div>
                  )}
                  {item.evidence.some((e) => !view.images[e]) && (
                    <div className="ac-verify-field">
                      <b>{t.ac_verify_evidence}</b>
                      {item.evidence.filter((e) => !view.images[e]).join("\n")}
                    </div>
                  )}
                  {item.evidence.some((e) => view.images[e]) && (
                    <div className="ac-verify-shots">
                      {item.evidence
                        .filter((e) => view.images[e])
                        .map((e) => (
                          <button key={e} className="ac-verify-shot" title={e} onClick={() => setZoom(view.images[e])}>
                            <img src={view.images[e]} alt={e} />
                          </button>
                        ))}
                    </div>
                  )}
                </div>
              ))}
            </div>
            {view.report.notes && <p className="ac-verify-field">{view.report.notes}</p>}
          </>
        )}
        <div className="task-dialog-actions">
          {canAccept && (
            <button className="aiterm-btn aiterm-btn--primary" onClick={onAccept}>
              {t.ac_verify_accept}
            </button>
          )}
          <button className="aiterm-btn aiterm-btn--secondary" onClick={onClose}>
            {t.ac_verify_close}
          </button>
        </div>
      </div>
      {zoom && (
        <button className="ac-verify-zoom" onClick={() => setZoom(null)} aria-label={t.ac_verify_close}>
          <img src={zoom} alt="" />
        </button>
      )}
    </div>
  );
}
