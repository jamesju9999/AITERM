import type { TaskWithAttachments } from "../../ipc/tasks";

export interface LabelGroup {
  label: string;
  cards: TaskWithAttachments[];
}

/**
 * 把一欄卡片切成「未分類」跟「依 label 分組」兩塊。群組內卡片順序沿用
 * 輸入順序，不重新排序；群組彼此的順序依該 label 在輸入陣列中第一次
 * 出現的 `created_at`，由舊到新——順序穩定，不會因為卡片增減而跳動。
 */
export function groupByLabel(cards: TaskWithAttachments[]): {
  ungrouped: TaskWithAttachments[];
  groups: LabelGroup[];
} {
  const ungrouped: TaskWithAttachments[] = [];
  const byLabel = new Map<string, TaskWithAttachments[]>();
  for (const c of cards) {
    const label = c.label?.trim();
    if (!label) {
      ungrouped.push(c);
      continue;
    }
    const arr = byLabel.get(label) ?? [];
    arr.push(c);
    byLabel.set(label, arr);
  }
  const groups = [...byLabel.entries()]
    // created_at 是 SQLite `datetime('now')` 產生的固定寬度字串
    // （'YYYY-MM-DD HH:MM:SS'），字串比較跟時間先後完全一致——刻意不用
    // `Date.parse`：Tauri 在三個平台各自嵌入不同的 WebView 引擎，對
    // 「非 ISO 8601」日期字串的寬鬆解析行為並不保證一致，字串比較沒有
    // 這個跨平台風險。
    .map(([label, groupCards]) => ({
      label,
      cards: groupCards,
      firstSeen: groupCards.reduce(
        (min, c) => (c.created_at < min ? c.created_at : min),
        groupCards[0].created_at,
      ),
    }))
    .sort((a, b) => (a.firstSeen < b.firstSeen ? -1 : a.firstSeen > b.firstSeen ? 1 : 0))
    .map(({ label, cards: groupCards }) => ({ label, cards: groupCards }));
  return { ungrouped, groups };
}
