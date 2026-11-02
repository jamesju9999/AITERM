import type { IBuffer } from "@xterm/xterm";

/**
 * 指令開始時「標記列上方」那幾列畫面的快照。
 *
 * 區塊卡的內容是從標記列（指令輸出的第一列）讀到游標所在位置。但全螢幕式的 TUI
 * （例如 Windows 上的 claude）常常先清畫面、游標回到視窗最上方，再從那裡重畫整個介面——
 * 於是介面的上半部落在標記列的**上方**，被排除在外，區塊卡只剩下半部的分隔線與狀態列。
 * 拿快照跟現在比對，就知道這個指令改寫了哪些標記列以上的列，從最上面被改寫的那一列開始讀；
 * 沒被動過的舊輸出（上一個指令留下的）維持排除，所以畫面是乾淨的。
 */
export interface RowsAboveSnapshot {
  /** 快照第一列的緩衝區絕對列號（當時視窗最上方）。 */
  baseRow: number;
  /** 拍快照當下的標記列。標記列的號碼之後變了（scrollback 被修剪、整個緩衝區位移）就不能比對。 */
  markerLine: number;
  rows: string[];
}

const rowText = (buffer: IBuffer, y: number): string => buffer.getLine(y)?.translateToString(true) ?? "";

export function snapshotRowsAbove(buffer: IBuffer, markerLine: number): RowsAboveSnapshot {
  const baseRow = Math.min(buffer.baseY, Math.max(0, markerLine));
  const rows: string[] = [];
  for (let y = baseRow; y < markerLine; y++) rows.push(rowText(buffer, y));
  return { baseRow, markerLine, rows };
}

/**
 * 區塊內容該從哪一列開始讀：標記列，或標記列上方被這個指令改寫過、而且現在有內容的最上面那一列。
 * 只被清成空白、沒再寫東西的列不算（清畫面後留下的空列不該撐出一段空白）。
 */
export function outputStartRow(buffer: IBuffer, snap: RowsAboveSnapshot | null, markerLine: number): number {
  const fallback = Math.max(0, markerLine);
  if (!snap || markerLine < 0 || snap.markerLine !== markerLine) return fallback;
  for (let i = 0; i < snap.rows.length; i++) {
    const now = rowText(buffer, snap.baseRow + i);
    if (now.trim() !== "" && now !== snap.rows[i]) return snap.baseRow + i;
  }
  return fallback;
}

/**
 * 指令清了畫面（`ESC[2J`）：xterm 會讓標記直接失效（`line` 變 -1，實測），跟「輸出比 scrollback 還長、
 * 標記被修剪掉」看起來一樣。清畫面之後，畫面上的東西都是這個指令畫的，從視窗最上方第一列有內容的開始讀，
 * 上面的空白列（被清掉、還沒重畫的）不算。
 */
export function clearedStartRow(buffer: IBuffer, cursorRow: number): number {
  let y = buffer.baseY;
  while (y < cursorRow && rowText(buffer, y).trim() === "") y++;
  return y;
}

/** 標記失效是不是因為清畫面：緩衝區還沒長到 scrollback 的上限，就不可能是被修剪掉的。 */
export function markerClearedByScreenErase(buffer: IBuffer, disposed: boolean, scrollback: number, rows: number): boolean {
  return disposed && buffer.length < scrollback + rows;
}
