import { useEffect, useState } from "react";
import { usageQuota, primaryWindow, type QuotaResult, type QuotaWindow } from "../ipc/usage";

/** 背景輪詢間隔。配額變動以分鐘計，5 分鐘足夠又不吵。 */
const POLL_MS = 5 * 60 * 1000;

/**
 * 追蹤單一 provider 的配額，供常駐徽章使用。
 *
 * 回傳代表窗（最嚴重的那個）；`null` 代表沒有配額概念、查詢失敗或尚未載入 ——
 * 這三種情況 UI 都是不顯示徽章，所以不需要再細分。
 *
 * 抓取時機：掛載查一次、每 5 分鐘輪詢、視窗回到前景時補查。**只查傳入的這一個
 * provider**，不是全部 —— 一次打三個上游端點只在使用者主動展開清單時才划算。
 *
 * 這個 hook 有三個消費端（終端機標題列、Ask AI 面板、ModelPickerButton），
 * 抽出來是為了避免三份各自漂移的輪詢邏輯。
 */
export function useProviderQuota(providerId: string | null | undefined): QuotaWindow | null {
  // 連同來源 id 一起存，這樣「換了 provider」與「沒有 provider」都能直接推導出
  // null，不必在 effect 裡 setState —— 那會觸發 react-hooks/set-state-in-effect，
  // 而且會讓畫面閃過一格屬於舊 provider 的數字。
  const [state, setState] = useState<{ id: string; window: QuotaWindow | null } | null>(null);

  useEffect(() => {
    if (!providerId) return;
    let cancelled = false;
    const load = () => {
      usageQuota(providerId, false)
        .then((r: QuotaResult) => {
          if (cancelled) return;
          setState({ id: providerId, window: r.status === "ok" ? primaryWindow(r.quota) : null });
        })
        .catch(() => { if (!cancelled) setState({ id: providerId, window: null }); });
    };
    load();
    // 背景放著不動的視窗不該一直打上游。
    const timer = setInterval(() => { if (!document.hidden) load(); }, POLL_MS);
    // 使用者離開一段時間再回來時，最關心的就是「現在還剩多少」。只靠 interval
    // 會讓他盯著最多 5 分鐘前的舊數字。後端有 60 秒快取，這裡補查很便宜。
    const onVisible = () => { if (!document.hidden) load(); };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      cancelled = true;
      clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [providerId]);

  // 只認屬於目前這個 provider 的快照。
  return providerId && state?.id === providerId ? state.window : null;
}
