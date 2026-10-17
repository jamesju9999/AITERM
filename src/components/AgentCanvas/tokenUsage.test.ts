import { describe, expect, it } from "vitest";

import { formatTokens, moduleCards, sumUsage, totalTokens } from "./tokenUsage";

const u = (input: number, cache_read = 0) => ({ input, cache_write: 0, cache_read, output: 1, calls: 1 });

describe("tokenUsage", () => {
  it("formats counts compactly", () => {
    expect([999, 12_345, 1_234_567].map(formatTokens)).toEqual(["999", "12.3K", "1.23M"]);
  });

  it("sums cards that ran, and counts the ones without a transcript separately", () => {
    const cards = [
      { id: "a", dispatched_at: 1 },
      { id: "b", dispatched_at: 2 },
      { id: "never", dispatched_at: null },
      { id: "lost", dispatched_at: 3 },
    ];
    const s = sumUsage(cards, { a: u(10, 100), b: u(5), never: u(1000) });
    expect(s).toEqual({ usage: { input: 15, cache_write: 0, cache_read: 100, output: 2, calls: 2 }, cards: 2, missing: 1 });
    expect(totalTokens(s!.usage)).toBe(117);
    expect(sumUsage([{ id: "never", dispatched_at: null }], {})).toBeNull();
  });

  it("a module's cost includes its review and fix cards", () => {
    const tasks = [
      { id: "m", module_id: null },
      { id: "r", module_id: "m" },
      { id: "f", module_id: "m" },
      { id: "other", module_id: "x" },
    ];
    expect(moduleCards("m", tasks).map((c) => c.id)).toEqual(["m", "r", "f"]);
  });
});
