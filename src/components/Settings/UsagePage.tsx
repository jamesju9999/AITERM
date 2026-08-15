import { useCallback, useEffect, useState } from "react";
import {
  usageQuotaAll, usageSummary, cacheHitRate,
  type QuotaResult, type UsageRange, type UsageSummaryEntry,
} from "../../ipc/usage";
import { QuotaBadge } from "../QuotaBadge";
import { useLocale } from "../../contexts/LocaleContext";
import "./UsagePage.css";

// 第一版只做配額區塊（列出所有 provider 的訂閱配額，可手動重新整理）。
// 本地用量累計的表格是另一份計畫的範圍，不在這裡做。

type LocalRangeLabelKey = "usage_local_range_today" | "usage_local_range_7d" | "usage_local_range_30d";

const LOCAL_RANGES: { key: UsageRange; labelKey: LocalRangeLabelKey }[] = [
  { key: "today", labelKey: "usage_local_range_today" },
  { key: "days7", labelKey: "usage_local_range_7d" },
  { key: "days30", labelKey: "usage_local_range_30d" },
];

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

export function UsagePage() {
  const { t } = useLocale();
  const [quotas, setQuotas] = useState<QuotaResult[]>([]);

  const loadQuotas = useCallback((force: boolean) => {
    usageQuotaAll(force).then(setQuotas).catch(() => setQuotas([]));
  }, []);

  useEffect(() => { loadQuotas(false); }, [loadQuotas]);

  const [localRange, setLocalRange] = useState<UsageRange>("today");
  const [localRows, setLocalRows] = useState<UsageSummaryEntry[] | null>(null);
  const [localError, setLocalError] = useState<string | null>(null);

  useEffect(() => {
    // 不在 effect 裡直接 setState 清空舊資料（會觸發
    // react-hooks/set-state-in-effect）——只在非同步回呼裡更新，
    // 沿用 useProviderQuota.ts 的既有慣例。
    let cancelled = false;
    usageSummary(localRange)
      .then((r) => { if (!cancelled) { setLocalRows(r); setLocalError(null); } })
      .catch((e) => { if (!cancelled) setLocalError(String(e)); });
    return () => { cancelled = true; };
  }, [localRange]);

  return (
    <>
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
                    <QuotaBadge window={w} bar={false} />
                    <div className="usage-quota-bar">
                      {/* 長條跟著 severity 變色，否則超標的窗會畫成一樣的藍色。 */}
                      <div className={`usage-quota-bar-fill usage-quota-bar-fill--${w.severity}`}
                           style={{ width: `${Math.min(100, Math.max(0, w.used_percent))}%` }} />
                    </div>
                  </div>
                ))}
              </div>
            </div>
          );
        })}
      </section>

      <section className="usage-local-section">
        <h3>{t.usage_local_section_title}</h3>
        <div className="usage-local-range-tabs">
          {LOCAL_RANGES.map((r) => (
            <button
              key={r.key}
              className={r.key === localRange ? "active" : ""}
              onClick={() => setLocalRange(r.key)}
            >
              {t[r.labelKey]}
            </button>
          ))}
        </div>

        {localError && <div className="usage-local-error">{localError}</div>}

        {localRows !== null && localRows.length === 0 && (
          <div className="usage-local-empty" data-testid="usage-empty">{t.usage_local_empty}</div>
        )}

        {localRows !== null && localRows.length > 0 && (
          <table className="usage-local-table">
            <thead>
              <tr>
                <th>{t.usage_local_provider}</th>
                <th>{t.usage_local_model}</th>
                <th>{t.usage_local_requests}</th>
                <th>{t.usage_local_input}</th>
                <th>{t.usage_local_output}</th>
                <th>{t.usage_local_cache_hit}</th>
                <th>{t.usage_local_cost}</th>
              </tr>
            </thead>
            <tbody>
              {localRows.map((e) => {
                const hit = cacheHitRate(e);
                return (
                  <tr key={`${e.provider_id}-${e.model}`}>
                    <td>{e.provider_id}</td>
                    <td>{e.model}</td>
                    <td>{e.requests}</td>
                    <td>{formatTokens(e.prompt_tokens)}</td>
                    <td>{formatTokens(e.completion_tokens)}</td>
                    <td>{hit === null ? "—" : `${Math.round(hit * 100)}%`}</td>
                    <td data-testid={`cost-${e.provider_id}-${e.model}`}>
                      {e.estimated_cost_usd === null ? "—" : `$${e.estimated_cost_usd.toFixed(4)}`}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </section>
    </>
  );
}
