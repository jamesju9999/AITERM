/**
 * 看板卡片的建立時間：解析、顯示、依它排序。
 *
 * 後端 `tasks.created_at` 是 SQLite 的 `datetime('now')`：**UTC**、格式
 * `YYYY-MM-DD HH:MM:SS`、沒有時區標記。直接丟給 `Date.parse` 會被當成本地
 * 時間，台灣（UTC+8）會整整差八小時，所以要先補上 `Z`。
 */

export type SortMode = "default" | "created-desc" | "created-asc";

export const SORT_MODES: SortMode[] = ["default", "created-desc", "created-asc"];

const SQLITE_UTC = /^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/;

/** 解析失敗（空字串、格式不認得）回 `null`，讓呼叫端決定怎麼處理。 */
export function parseCreatedAt(s: string | null | undefined): number | null {
  if (!s) return null;
  const ms = Date.parse(SQLITE_UTC.test(s) ? `${s.replace(" ", "T")}Z` : s);
  return Number.isNaN(ms) ? null : ms;
}

const pad = (n: number) => String(n).padStart(2, "0");

/** 本地時間的 `YYYY-MM-DD HH:mm`；解析不了回 `null`（卡片就不顯示這一行）。 */
export function formatCreatedAt(s: string | null | undefined): string | null {
  const ms = parseCreatedAt(s);
  if (ms === null) return null;
  const d = new Date(ms);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/**
 * 依建立時間排序；`default` 原樣回傳（呼叫端傳進來的順序已經是該欄的
 * 預設順序：三個工作欄是手動 `sort_order`，已完成欄是完成時間）。
 *
 * 不動傳入的陣列。時間相同或解析不了的卡片保持原本的相對順序
 * （`Array.prototype.sort` 是穩定的）；解析不了的一律排在最後，不論升降冪
 * ——它們沒有可比的日期，放最前面只會把有資料的卡片擠下去。
 */
export function sortByCreated<T extends { created_at: string }>(rows: T[], mode: SortMode): T[] {
  if (mode === "default") return rows;
  const dir = mode === "created-desc" ? -1 : 1;
  return [...rows].sort((a, b) => {
    const x = parseCreatedAt(a.created_at);
    const y = parseCreatedAt(b.created_at);
    if (x === null && y === null) return 0;
    if (x === null) return 1;
    if (y === null) return -1;
    return (x - y) * dir;
  });
}

const SORT_STORAGE_KEY = "aiterm_board_sort";

/** 每欄各自的排序選擇（欄位狀態 → 模式）。壞資料、不認得的值一律當作沒存。 */
export function loadSortModes(): Record<string, SortMode> {
  try {
    const raw = JSON.parse(localStorage.getItem(SORT_STORAGE_KEY) ?? "{}");
    if (typeof raw !== "object" || raw === null) return {};
    const out: Record<string, SortMode> = {};
    for (const [k, v] of Object.entries(raw)) {
      if (SORT_MODES.includes(v as SortMode)) out[k] = v as SortMode;
    }
    return out;
  } catch {
    return {};
  }
}

export function saveSortModes(modes: Record<string, SortMode>): void {
  try {
    localStorage.setItem(SORT_STORAGE_KEY, JSON.stringify(modes));
  } catch {
    // 隱私模式或儲存空間被封鎖：排序只是顯示偏好，存不了就不存。
  }
}

const FLAT_STORAGE_KEY = "aiterm_board_flat";

/** 哪些欄在日期排序時選了「不分組」（欄位狀態 → true）。壞資料當作沒存。 */
export function loadFlatColumns(): Record<string, boolean> {
  try {
    const raw = JSON.parse(localStorage.getItem(FLAT_STORAGE_KEY) ?? "{}");
    if (typeof raw !== "object" || raw === null) return {};
    const out: Record<string, boolean> = {};
    for (const [k, v] of Object.entries(raw)) if (v === true) out[k] = true;
    return out;
  } catch {
    return {};
  }
}

export function saveFlatColumns(flat: Record<string, boolean>): void {
  try {
    localStorage.setItem(FLAT_STORAGE_KEY, JSON.stringify(flat));
  } catch {
    // 同 saveSortModes：只是顯示偏好。
  }
}
