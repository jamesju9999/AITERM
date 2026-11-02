import { Terminal } from "@xterm/xterm";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { clearedStartRow, markerClearedByScreenErase, outputStartRow, snapshotRowsAbove } from "./liveStartRow";

let term: Terminal;
const write = (data: string) => new Promise<void>((resolve) => term.write(data, resolve));

beforeEach(() => {
  term = new Terminal({ cols: 40, rows: 12, allowProposedApi: true });
});
afterEach(() => term.dispose());

/** 上面有兩列舊輸出，游標停在第 3 列（指令輸出的第一列）。 */
async function setup() {
  await write("PS> ls\r\nold output\r\n");
  const marker = term.registerMarker(0)!;
  const snap = snapshotRowsAbove(term.buffer.active, marker.line);
  return { marker, snap };
}

describe("outputStartRow", () => {
  it("指令沒動過標記列上方：從標記列開始，舊輸出不會混進來", async () => {
    const { marker, snap } = await setup();
    await write("new line 1\r\nnew line 2");
    expect(outputStartRow(term.buffer.active, snap, marker.line)).toBe(marker.line);
  });

  it("沒清畫面、游標往上移回去改寫（Ink 類 TUI 的重畫方式）：從被改寫的最上面那一列開始", async () => {
    const { marker, snap } = await setup();
    await write("line A\r\nline B\r\nline C");
    // 游標往上跑到標記列上方，把第 0 列蓋掉
    await write("\x1b[H\x1b[2KWELCOME BOX");
    expect(marker.isDisposed).toBe(false);
    expect(outputStartRow(term.buffer.active, snap, marker.line)).toBe(0);
  });

  it("上方只有空白被改寫（沒有新內容）：不撐出空白，維持標記列", async () => {
    const { marker, snap } = await setup();
    await write("\x1b[H\x1b[2K");
    expect(outputStartRow(term.buffer.active, snap, marker.line)).toBe(marker.line);
  });

  it("標記列的號碼變了（緩衝區位移）就不比對，退回標記列", async () => {
    const { snap } = await setup();
    expect(outputStartRow(term.buffer.active, snap, snap.markerLine + 5)).toBe(snap.markerLine + 5);
  });

  it("沒有快照或標記被修剪掉（-1）：行為跟以前一樣", async () => {
    await setup();
    expect(outputStartRow(term.buffer.active, null, 7)).toBe(7);
    expect(outputStartRow(term.buffer.active, null, -1)).toBe(0);
  });
});

describe("清畫面（ESC[2J）會讓標記失效", () => {
  it("xterm 實測：標記被丟掉、line 變 -1，而且判得出是清畫面而不是被修剪", async () => {
    const { marker } = await setup();
    await write("\x1b[2J\x1b[H");
    expect(marker.isDisposed).toBe(true);
    expect(marker.line).toBe(-1);
    expect(markerClearedByScreenErase(term.buffer.active, marker.isDisposed, 1000, term.rows)).toBe(true);
    // 緩衝區已經長到 scrollback 上限：才是被修剪掉
    expect(markerClearedByScreenErase(term.buffer.active, true, 0, 0)).toBe(false);
    expect(markerClearedByScreenErase(term.buffer.active, false, 1000, term.rows)).toBe(false);
  });

  it("清畫面後從最上方重畫：整個介面都讀得到", async () => {
    await setup();
    await write("\x1b[2J\x1b[H");
    await write("WELCOME BOX\r\nchat history\r\n> input");
    const buf = term.buffer.active;
    expect(clearedStartRow(buf, buf.baseY + buf.cursorY)).toBe(0);
  });

  it("清畫面後先空幾列才畫：略過上面的空白列", async () => {
    await setup();
    await write("\x1b[2J\x1b[4;1H");
    await write("UI top\r\nUI bottom");
    const buf = term.buffer.active;
    expect(clearedStartRow(buf, buf.baseY + buf.cursorY)).toBe(3);
  });

  it("清了畫面還沒畫東西：從游標列開始，不撐出空白", async () => {
    await setup();
    await write("\x1b[2J\x1b[H");
    const buf = term.buffer.active;
    expect(clearedStartRow(buf, buf.baseY + buf.cursorY)).toBe(0);
  });
});
