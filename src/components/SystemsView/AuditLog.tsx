import { useLocale } from "../../contexts/LocaleContext";
import type { AuditRow } from "../../ipc/systems";

export function AuditLog({ rows }: { rows: AuditRow[] }) {
  const { t } = useLocale();
  const label: Record<AuditRow["verdict"], string> = {
    allow: t.sys_verdict_allow,
    deny: t.sys_verdict_deny,
    approved: t.sys_verdict_approved,
    rejected: t.sys_verdict_rejected,
    timeout: t.sys_verdict_timeout,
    manual: t.sys_verdict_manual,
  };
  return (
    <section className="systems__audit">
      {rows.length === 0 ? (
        <p className="systems__muted">{t.sys_audit_empty}</p>
      ) : (
        <ul>
          {rows.map((r) => (
            <li key={r.id} data-testid="audit-row">
              <span className={`systems__verdict systems__verdict--${r.verdict}`}>{label[r.verdict]}</span>
              <span>{new Date(r.ts_ms).toLocaleString()}</span>
              <span>{r.intent || r.tool}</span>
              {r.method && (
                <code>
                  {r.method} {r.url}
                </code>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
