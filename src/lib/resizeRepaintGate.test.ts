import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { ResizeRepaintGate, RESIZE_REPAINT_PREFIX } from "./resizeRepaintGate";

const repaint = (marker: string) => `${RESIZE_REPAINT_PREFIX}${marker}`;

describe("ResizeRepaintGate", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("剛 resize 過後，連續多筆全畫面重繪只保留最後一筆——實機截圖證實 ConPTY 在這個窗口內會夾雜一筆過期畫面", () => {
    const gate = new ResizeRepaintGate();
    const write = vi.fn();
    gate.noteResize();
    gate.handleChunk(repaint("correct-1"), false, write);
    gate.handleChunk(repaint("stale"), false, write);
    gate.handleChunk(repaint("correct-2"), false, write);
    expect(write).not.toHaveBeenCalled();

    vi.runOnlyPendingTimers();

    expect(write).toHaveBeenCalledTimes(1);
    expect(write).toHaveBeenCalledWith(repaint("correct-2"));
  });

  it("沒有 resize 過的全畫面重繪 chunk，照常立即寫入不等待", () => {
    const gate = new ResizeRepaintGate();
    const write = vi.fn();
    gate.handleChunk(repaint("normal-redraw"), false, write);
    expect(write).toHaveBeenCalledTimes(1);
    expect(write).toHaveBeenCalledWith(repaint("normal-redraw"));
  });

  it("isAlternateBuffer（全螢幕 TUI）時即使剛 resize 過也不合併，直接寫入——避免吃掉 vim/htop 的畫面更新", () => {
    const gate = new ResizeRepaintGate();
    const write = vi.fn();
    gate.noteResize();
    gate.handleChunk(repaint("tui-frame"), true, write);
    expect(write).toHaveBeenCalledTimes(1);
    expect(write).toHaveBeenCalledWith(repaint("tui-frame"));
  });

  it("持有中的重繪遇到非重繪內容時立刻依序 flush，不等待計時器", () => {
    const gate = new ResizeRepaintGate();
    const write = vi.fn();
    gate.noteResize();
    gate.handleChunk(repaint("held"), false, write);
    expect(write).not.toHaveBeenCalled();

    gate.handleChunk("normal output\r\n", false, write);

    expect(write).toHaveBeenCalledTimes(2);
    expect(write).toHaveBeenNthCalledWith(1, repaint("held"));
    expect(write).toHaveBeenNthCalledWith(2, "normal output\r\n");
  });

  it("超過武裝時間窗後，全畫面重繪不再合併，直接寫入", () => {
    const gate = new ResizeRepaintGate();
    const write = vi.fn();
    gate.noteResize();
    vi.advanceTimersByTime(1000);

    gate.handleChunk(repaint("late"), false, write);

    expect(write).toHaveBeenCalledTimes(1);
    expect(write).toHaveBeenCalledWith(repaint("late"));
  });
});
