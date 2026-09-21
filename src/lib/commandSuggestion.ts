/**
 * 行內建議：從歷史（舊 → 新）找最新一筆「以目前輸入開頭、且比它長」的單行指令，
 * 回傳要顯示的剩餘部分；沒有就回 null。區分大小寫。
 * 只處理文字本身的規則；游標位置、選單是否開啟等 UI 條件由呼叫端負責。
 */
export function findSuggestion(history: readonly string[], value: string): string | null {
  if (!value || value.includes("\n")) return null;
  for (let i = history.length - 1; i >= 0; i--) {
    const h = history[i];
    if (h.length > value.length && !h.includes("\n") && h.startsWith(value)) {
      return h.slice(value.length);
    }
  }
  return null;
}
