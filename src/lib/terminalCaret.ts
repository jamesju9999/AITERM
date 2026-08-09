/**
 * 找出全螢幕 TUI 畫在畫面上的游標方塊。
 *
 * 為什麼需要這個：這類程式會用 ESC[?25l 關掉終端機的真游標。Windows 的 ConPTY
 * 在那之後就不再把游標擺回 caret（反正看不見），游標於是停在「最後寫到的那格」
 * ——實測 col 51、col 81 都出現過，不是固定在某一欄。xterm 的 IME 組字 UI 跟著
 * 真游標走，注音因此跑到畫面別處。
 *
 * 真游標既然不可信，只能回頭在畫面內容裡找那個看得見的方塊——實測 Windows 上
 * Claude Code 是用反白（SGR 7）畫的（診斷面板量到 caret=inverse@4,24）。
 *
 * 只認反白、不認實體方塊字元（█▌▐）：內文本來就可能出現那些字元（進度條、
 * 框線圖），認了會在 macOS 這種本來就正常的平台上誤判，反而把好的弄壞。
 */

/** 只取用得到的部分，測試才不必假造整個 xterm Terminal。 */
export interface CaretScanCell {
  isInverse(): number;
}

export interface CaretScanLine {
  getCell(x: number): CaretScanCell | undefined;
}

export interface CaretScanTarget {
  rows: number;
  cols: number;
  buffer: {
    active: {
      viewportY: number;
      getLine(y: number): CaretScanLine | undefined;
    };
  };
}

/**
 * 一整段反白最多幾格才算 caret。反白也用來標示選單選取列、diff 區塊之類，
 * 那些會是一長串；caret 則是一格（全形字兩格）。
 */
const MAX_CARET_RUN = 2;

/**
 * 由下往上找，回傳最靠近底部的那個 caret（以可視區左上角為原點）。找不到回
 * null，呼叫端自行決定退路。
 *
 * 由下往上是因為輸入框幾乎都在畫面下緣，而上方的內容區比較可能出現其他反白
 * （引用、選取、語法標示）。
 *
 * 同一列有多段反白時取**最右邊**那一段：caret 位在已輸入文字的尾端，左邊那些
 * 是別的東西。實測抓到過取最左邊的後果——輸入列上有另一段反白時 caret 卡在
 * 第 2 欄不動，新的組字內容就從行首疊上去，把已輸入的字蓋掉（使用者看到的是
 * 「重新輸入時又從頭開始」）。
 */
export function findAppCaret(term: CaretScanTarget): { x: number; y: number } | null {
  const buf = term.buffer.active;
  for (let y = term.rows - 1; y >= 0; y--) {
    const line = buf.getLine(buf.viewportY + y);
    if (!line) continue;

    let found: number | null = null;
    let x = 0;
    while (x < term.cols) {
      if (!line.getCell(x)?.isInverse()) {
        x++;
        continue;
      }
      const start = x;
      while (x < term.cols && line.getCell(x)?.isInverse()) x++;
      // 太長就不是 caret（選單選取列那種），跳過但繼續往右掃。
      if (x - start <= MAX_CARET_RUN) found = start;
    }
    if (found !== null) return { x: found, y };
  }
  return null;
}
