import { describe, expect, it } from "vitest";

import { findAppCaret, type CaretScanTarget } from "./terminalCaret";

/**
 * 用字串畫出畫面：`#` 是反白的格子，其他字元就是該格的內容。行數即 rows，
 * 最長的行決定 cols。
 */
function screen(lines: string[], viewportY = 0): CaretScanTarget {
  const cols = Math.max(...lines.map((l) => [...l].length));
  return {
    rows: lines.length,
    cols,
    buffer: {
      active: {
        viewportY,
        getLine: (y: number) => {
          const line = lines[y - viewportY];
          if (line === undefined) return undefined;
          const chars = [...line];
          return {
            getCell: (x: number) => ({
              isInverse: () => (chars[x] === "#" ? 1 : 0),
            }),
          };
        },
      },
    },
  };
}

describe("findAppCaret", () => {
  it("找到單獨一格反白", () => {
    expect(findAppCaret(screen([".....", "..#..", "....."]))).toEqual({ x: 2, y: 1 });
  });

  it("畫面上什麼都沒有時回傳 null", () => {
    expect(findAppCaret(screen(["....", "....", "...."]))).toBeNull();
  });

  it("由下往上找，取最靠近底部的那個", () => {
    expect(findAppCaret(screen(["#....", ".....", "...#."]))).toEqual({ x: 3, y: 2 });
  });

  it("全形字佔的兩格仍算 caret", () => {
    expect(findAppCaret(screen([".....", ".##.."]))).toEqual({ x: 1, y: 1 });
  });

  // 反白也用來標示選單選取列，那會是一長串——認成 caret 的話組字框會跳到選單上。
  it("略過長串反白（選單選取列那種）", () => {
    expect(findAppCaret(screen(["......", "######", "..#..."]))).toEqual({ x: 2, y: 2 });
  });

  it("同一列裡跳過長串後仍找得到後面的 caret", () => {
    expect(findAppCaret(screen(["####.#."]))).toEqual({ x: 5, y: 0 });
  });

  // caret 在已輸入文字的尾端；左邊那些反白是別的東西。取最左邊會讓 caret 卡在
  // 行首，新的組字內容就從頭疊上去把已輸入的字蓋掉。
  it("同一列有多段短反白時取最右邊那一段", () => {
    expect(findAppCaret(screen(["..#....#.."]))).toEqual({ x: 7, y: 0 });
  });

  it("最右邊那段是長串時，仍取左邊合格的那一段", () => {
    expect(findAppCaret(screen(["..#..#####"]))).toEqual({ x: 2, y: 0 });
  });

  // alt buffer 的 viewportY 是 0，但一般緩衝區捲動後不是——座標要以可視區為準。
  it("回傳的是可視區內的相對列號，不是緩衝區絕對列號", () => {
    expect(findAppCaret(screen([".....", "..#.."], 120))).toEqual({ x: 2, y: 1 });
  });
});
