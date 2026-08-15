import { useCallback, useEffect, useState } from "react";
import { usageQuotaAll, type QuotaResult } from "../../ipc/usage";
import { QuotaBadge } from "../QuotaBadge";
import { useLocale } from "../../contexts/LocaleContext";
import "./UsagePage.css";

// 第一版只做配額區塊（列出所有 provider 的訂閱配額，可手動重新整理）。
// 本地用量累計的表格是另一份計畫的範圍，不在這裡做。
export function UsagePage() {
  const { t } = useLocale();
  const [quotas, setQuotas] = useState<QuotaResult[]>([]);

  const loadQuotas = useCallback((force: boolean) => {
    usageQuotaAll(force).then(setQuotas).catch(() => setQuotas([]));
  }, []);

  useEffect(() => { loadQuotas(false); }, [loadQuotas]);

  return (
    <section className="usage-quota-section">
      <div className="usage-quota-header">
        <h3>{t.usage_quota_title}</h3>
        <button data-testid="quota-refresh" onClick={() => loadQuotas(true)}>
          {t.usage_refresh}
        </button>
      </div>
      {quotas.map((r) => {
        if (r.status === "not_applicable") return null;
        if (r.status === "failed") {
          return (
            <div key={r.provider_id} className="usage-quota-card usage-quota-card--failed"
                 data-testid={`quota-failed-${r.provider_id}`}>
              <span className="usage-quota-provider">{r.provider_id}</span>
              <span className="usage-quota-error">{t.quota_unavailable}: {r.message}</span>
            </div>
          );
        }
        return (
          <div key={r.quota.provider_id} className="usage-quota-card">
            <div className="usage-quota-card-head">
              <span className="usage-quota-provider">{r.quota.provider_id}</span>
              {r.quota.plan && <span className="usage-quota-plan">{r.quota.plan}</span>}
            </div>
            <div className="usage-quota-windows">
              {r.quota.windows.map((w) => (
                <div key={w.label} className="usage-quota-window">
                  <QuotaBadge window={w} />
                  <div className="usage-quota-bar">
                    <div className="usage-quota-bar-fill"
                         style={{ width: `${Math.min(100, w.used_percent)}%` }} />
                  </div>
                </div>
              ))}
            </div>
          </div>
        );
      })}
    </section>
  );
}
