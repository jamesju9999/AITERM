import { invoke } from "@tauri-apps/api/core";

export type QuotaSeverity = "normal" | "warning" | "critical";

export interface QuotaWindow {
  label: string;
  /** 已使用百分比，0–100。 */
  used_percent: number;
  resets_at: number | null;
  severity: QuotaSeverity;
  /** Copilot 的 "142 / 300" 之類的原始語意。 */
  detail: string | null;
  is_primary: boolean;
}

export interface ProviderQuota {
  provider_id: string;
  plan: string | null;
  /** 空陣列 = 查得到但沒有配額限制，與查詢失敗不同。 */
  windows: QuotaWindow[];
  fetched_at: number;
}

export type QuotaResult =
  | { status: "ok"; quota: ProviderQuota }
  | { status: "not_applicable"; provider_id: string }
  | { status: "failed"; provider_id: string; message: string };

export function usageQuota(providerId: string, force = false): Promise<QuotaResult> {
  return invoke<QuotaResult>("usage_quota", { providerId, force });
}

export function usageQuotaAll(force = false): Promise<QuotaResult[]> {
  return invoke<QuotaResult[]>("usage_quota_all", { force });
}

const SEVERITY_RANK: Record<QuotaSeverity, number> = {
  normal: 0,
  warning: 1,
  critical: 2,
};

/**
 * 收合狀態要顯示的那個窗。
 *
 * **取最嚴重的窗**，而不是上游標記的代表窗 —— 5h 窗剛重置 0% 但 7d 窗已
 * 96% 時，顯示綠色 0% 會讓使用者誤以為還很寬裕。同嚴重度時才用 is_primary
 * 決定，再同則取第一個。
 *
 * 這條規則在 `src-tauri/src/usage/quota/mod.rs` 的 `ProviderQuota::primary_window`
 * 有一份對應的 Rust 實作。**改這裡就必須同步改那裡。**
 */
export function primaryWindow(q: ProviderQuota): QuotaWindow | null {
  let best: QuotaWindow | null = null;
  for (const w of q.windows) {
    if (best === null) { best = w; continue; }
    const d = SEVERITY_RANK[w.severity] - SEVERITY_RANK[best.severity];
    if (d > 0 || (d === 0 && w.is_primary && !best.is_primary)) best = w;
  }
  return best;
}
