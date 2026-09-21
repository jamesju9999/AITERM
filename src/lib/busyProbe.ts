/** 分頁「正在做什麼」。與 i18n 的 `win_close_reason_*` 一一對應。 */
export type BusyReason = "command" | "agent" | "task" | "loop" | "streaming";

/** 分頁註冊的探針：忙碌回傳原因，閒置回傳 null。只讀狀態，不可彈框、不可有副作用。 */
export type BusyProbe = () => BusyReason | null;

export interface BusyTab {
  tabId: string;
  title: string;
  reason: BusyReason;
}

/**
 * 逐一詢問所有探針，回傳忙碌的分頁（保持註冊順序）。
 * 單一探針丟例外時當作閒置——寧可漏報一個分頁，也不能讓「關視窗」本身失效。
 */
export function collectBusyTabs(
  probes: ReadonlyMap<string, BusyProbe>,
  titleOf: (tabId: string) => string | undefined,
): BusyTab[] {
  const busy: BusyTab[] = [];
  for (const [tabId, probe] of probes) {
    let reason: BusyReason | null;
    try {
      reason = probe();
    } catch (e) {
      console.warn(`busy probe of tab ${tabId} threw; treating it as idle:`, e);
      continue;
    }
    if (reason) busy.push({ tabId, title: titleOf(tabId) ?? tabId, reason });
  }
  return busy;
}
