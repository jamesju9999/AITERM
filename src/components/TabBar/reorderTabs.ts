/**
 * 把第 `from` 個元素搬到第 `to` 個位置，回傳新陣列。
 *
 * 原地不動或索引無效時回傳原本那個陣列（同一個參考），呼叫端就能靠 identity
 * 判斷「沒有變化」而不觸發重繪。
 *
 * 獨立成一個檔案是因為 react-refresh 規定元件檔只能匯出元件。
 */
export function reorderTabs<T>(items: T[], from: number, to: number): T[] {
  if (from === to) return items;
  if (from < 0 || to < 0 || from >= items.length || to >= items.length) return items;
  const next = [...items];
  const [moved] = next.splice(from, 1);
  next.splice(to, 0, moved);
  return next;
}
