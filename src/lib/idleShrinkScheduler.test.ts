import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { IdleShrinkScheduler, IDLE_SHRINK_SETTLE_MS } from "./idleShrinkScheduler";

describe("IdleShrinkScheduler", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(0);
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("安靜滿一段時間後才觸發收縮——讓遲來的自訂提示字元（如 oh-my-posh）來得及先印出來", () => {
    const onSettle = vi.fn();
    const scheduler = new IdleShrinkScheduler(() => 0, onSettle);
    scheduler.arm();

    vi.advanceTimersByTime(IDLE_SHRINK_SETTLE_MS - 1);
    expect(onSettle).not.toHaveBeenCalled();

    vi.advanceTimersByTime(1);
    expect(onSettle).toHaveBeenCalledTimes(1);
  });

  it("安靜期間如果有新輸出進來，原本排定的收縮會延後到新輸出後再滿一段安靜期", () => {
    let lastOutputAt = 0;
    const onSettle = vi.fn();
    const scheduler = new IdleShrinkScheduler(() => lastOutputAt, onSettle);
    scheduler.arm();

    vi.advanceTimersByTime(IDLE_SHRINK_SETTLE_MS - 100);
    lastOutputAt = Date.now(); // 遲來的提示字元剛印出來
    vi.advanceTimersByTime(100);
    expect(onSettle).not.toHaveBeenCalled();

    vi.advanceTimersByTime(IDLE_SHRINK_SETTLE_MS - 100);
    expect(onSettle).toHaveBeenCalledTimes(1);
  });

  it("cancel() 取消排定中的收縮", () => {
    const onSettle = vi.fn();
    const scheduler = new IdleShrinkScheduler(() => 0, onSettle);
    scheduler.arm();
    scheduler.cancel();

    vi.advanceTimersByTime(IDLE_SHRINK_SETTLE_MS * 2);
    expect(onSettle).not.toHaveBeenCalled();
  });

  it("重複呼叫 arm() 會重新排定，不會疊加出多次觸發", () => {
    const onSettle = vi.fn();
    const scheduler = new IdleShrinkScheduler(() => 0, onSettle);
    scheduler.arm();
    vi.advanceTimersByTime(200);
    scheduler.arm();
    vi.advanceTimersByTime(200);
    scheduler.arm();

    vi.advanceTimersByTime(IDLE_SHRINK_SETTLE_MS);
    expect(onSettle).toHaveBeenCalledTimes(1);
  });
});
