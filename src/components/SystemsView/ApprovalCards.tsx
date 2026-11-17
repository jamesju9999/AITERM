import { useLocale } from "../../contexts/LocaleContext";
import type { PendingApproval } from "../../ipc/systems";

interface Props {
  approvals: PendingApproval[];
  nameOf: (systemId: string) => string;
  onDecide: (id: string, approve: boolean) => void;
  /** 拒絕並停用該系統。 */
  onRejectAndDisable: (a: PendingApproval) => void;
}

/** 等使用者核准的寫入請求：顯示 AI 宣告的意圖與實際的請求。 */
export function ApprovalCards({ approvals, nameOf, onDecide, onRejectAndDisable }: Props) {
  const { t } = useLocale();
  if (approvals.length === 0) return null;
  return (
    <section className="systems__approvals" aria-label={t.sys_approvals_title}>
      <h3>{t.sys_approvals_title}</h3>
      {approvals.map((a) => (
        <div key={a.id} className="systems__approval" data-testid="approval-card">
          <div className="systems__approval-system">{nameOf(a.system_id)}</div>
          <div>
            {t.sys_ai_wants}
            <strong>{a.intent || "—"}</strong>
          </div>
          <code className="systems__request">
            {a.method} {a.url}
          </code>
          {a.declared_read && (
            <p className="systems__warn" role="alert" data-testid="declared-read-warning">
              ⚠ {t.sys_declared_read_warn}
            </p>
          )}
          <div className="systems__actions">
            <button type="button" className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" onClick={() => onDecide(a.id, true)}>
              {t.sys_approve}
            </button>
            <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => onDecide(a.id, false)}>
              {t.sys_reject}
            </button>
            <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => onRejectAndDisable(a)}>
              {t.sys_reject_disable}
            </button>
          </div>
        </div>
      ))}
    </section>
  );
}
