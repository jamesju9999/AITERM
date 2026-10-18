import { useCallback, useEffect, useState } from "react";
import {
  usageQuotaAll, usageSummary, cacheHitRate,
  type QuotaResult, type QuotaWindow, type UsageRange, type UsageSummaryEntry,
} from "../../ipc/usage";
import { useLocale } from "../../contexts/LocaleContext";
import "./UsagePage.css";

type T = ReturnType<typeof useLocale>["t"];

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

/** 窗長標籤（後端給 `5h`／`7d`／`30d`；Copilot 給 `premium`）寫成人話，其他照原樣。 */
function windowLabel(label: string, t: T): string {
  const m = /^(\d+)([hd])$/.exec(label);
  if (m) return m[2] === "h" ? t.usage_window_hours(m[1]) : t.usage_window_days(m[1]);
  if (label === "premium") return t.usage_window_premium;
  return label;
}

/** 方案代碼（例如 Copilot 的 `yearly_subscriber_quota`）寫成人話，認不得的照原樣。 */
function planLabel(plan: string, t: T): string {
  const p = plan.toLowerCase();
  if (p === "free") return t.usage_plan_free;
  if (p.startsWith("yearly")) return t.usage_plan_yearly;
  if (p.startsWith("monthly")) return t.usage_plan_monthly;
  return plan;
}

function resetsIn(resetsAt: number | null, t: T): string | null {
  if (resetsAt === null) return null;
  const secs = resetsAt - Math.floor(Date.now() / 1000);
  if (secs <= 0) return null;
  if (secs < 3600) return t.usage_resets_minutes(Math.ceil(secs / 60));
  if (secs < 86400) return t.usage_resets_hours(Math.round(secs / 3600));
  return t.usage_resets_days(Math.round(secs / 86400));
}

/** 一個額度窗：左邊窗長、中間長條、右邊用量與重置時間。 */
function QuotaRow({ w, t }: { w: QuotaWindow; t: T }) {
  const pct = Math.round(w.used_percent);
  // Copilot 的 detail 是「剩餘/總數」，比百分比精確，有就用它。
  const amount = w.detail ? t.usage_quota_remaining(w.detail.replace("/", " / ")) : t.usage_quota_used(pct);
  const reset = resetsIn(w.resets_at, t);
  return (
    <div className="usage-quota-row" data-testid="quota-window">
      <span className="usage-quota-row-label">{windowLabel(w.label, t)}</span>
      <div className="usage-quota-bar" role="progressbar" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100}>
        {/* 長條跟著 severity 變色，否則超標的窗會畫成一樣的顏色。 */}
        <div className={`usage-quota-bar-fill usage-quota-bar-fill--${w.severity}`}
             style={{ width: `${Math.min(100, Math.max(0, w.used_percent))}%` }} />
      </div>
      <span className={`usage-quota-row-amount usage-quota-row-amount--${w.severity}`}>{amount}</span>
      <span className="usage-quota-row-reset">{reset ?? ""}</span>
    </div>
  );
}

export function UsagePage() {
  const { t } = useLocale();
  const [quotas, setQuotas] = useState<QuotaResult[] | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  const loadQuotas = useCallback(
    (force: boolean) => usageQuotaAll(force).then(setQuotas).catch(() => setQuotas([])),
    [],
  );

  useEffect(() => { loadQuotas(false); }, [loadQuotas]);

  const refresh = () => {
    setRefreshing(true);
    loadQuotas(true).finally(() => setRefreshing(false));
  };

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

  const shown = (quotas ?? []).filter((r) => r.status !== "not_applicable");

  return (
    <div className="usage-page">
      <section className="usage-section">
        <div className="usage-section-head">
          <div>
            <h2>{t.usage_quota_title}</h2>
            <p className="usage-desc">{t.usage_quota_desc}</p>
          </div>
          <button className="usage-refresh" data-testid="quota-refresh" disabled={refreshing}
                  onClick={refresh}>
            {t.usage_refresh}
          </button>
        </div>
        {quotas !== null && shown.length === 0 && <div className="usage-empty">{t.usage_quota_empty}</div>}
        <div className="usage-quota-list">
          {shown.map((r) => {
            if (r.status === "failed") {
              return (
                <div key={r.provider_id} className="usage-quota-card usage-quota-card--failed"
                     data-testid={`quota-failed-${r.provider_id}`}>
                  <div className="usage-quota-card-head">
                    <span className="usage-quota-provider">{r.provider_id}</span>
                    <span className="usage-quota-status">{t.usage_quota_failed_hint}</span>
                  </div>
                  <p className="usage-quota-error" title={r.message}>{r.message}</p>
                </div>
              );
            }
            if (r.status !== "ok") return null;
            return (
              <div key={r.quota.provider_id} className="usage-quota-card">
                <div className="usage-quota-card-head">
                  <span className="usage-quota-provider">{r.quota.provider_id}</span>
                  {r.quota.plan && <span className="usage-quota-plan">{planLabel(r.quota.plan, t)}</span>}
                </div>
                {r.quota.windows.map((w) => <QuotaRow key={w.label} w={w} t={t} />)}
              </div>
            );
          })}
        </div>
      </section>

      <section className="usage-section">
        <div className="usage-section-head">
          <div>
            <h3>{t.usage_local_section_title}</h3>
            <p className="usage-desc">{t.usage_local_desc}</p>
          </div>
          <div className="usage-range" role="group" aria-label={t.usage_local_section_title}>
            {LOCAL_RANGES.map((r) => (
              <button
                key={r.key}
                className={r.key === localRange ? "active" : ""}
                aria-pressed={r.key === localRange}
                onClick={() => setLocalRange(r.key)}
              >
                {t[r.labelKey]}
              </button>
            ))}
          </div>
        </div>

        {localError && <div className="usage-local-error">{localError}</div>}

        {localRows !== null && localRows.length === 0 && (
          <div className="usage-empty" data-testid="usage-empty">{t.usage_local_empty}</div>
        )}

        {localRows !== null && localRows.length > 0 && (
          <table className="usage-local-table">
            <thead>
              <tr>
                <th>{t.usage_local_provider}</th>
                <th>{t.usage_local_model}</th>
                <th className="num">{t.usage_local_requests}</th>
                <th className="num">{t.usage_local_input}</th>
                <th className="num">{t.usage_local_output}</th>
                <th className="num">{t.usage_local_cache_hit}</th>
                <th className="num">{t.usage_local_cost}</th>
              </tr>
            </thead>
            <tbody>
              {localRows.map((e) => {
                const hit = cacheHitRate(e);
                return (
                  <tr key={`${e.provider_id}-${e.model}`}>
                    <td>{e.provider_id}</td>
                    <td className="usage-model">{e.model}</td>
                    <td className="num">{e.requests}</td>
                    <td className="num">{formatTokens(e.prompt_tokens)}</td>
                    <td className="num">{formatTokens(e.completion_tokens)}</td>
                    <td className="num">{hit === null ? "—" : `${Math.round(hit * 100)}%`}</td>
                    <td className="num" data-testid={`cost-${e.provider_id}-${e.model}`}>
                      {e.estimated_cost_usd === null ? "—" : `$${e.estimated_cost_usd.toFixed(4)}`}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </section>
    </div>
  );
}
