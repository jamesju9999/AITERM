import { describe, expect, it } from "vitest";
import {
  MAX_SCREENS,
  MAX_HISTORY_CHARS,
  formatHistoryForPrompt,
  isSameScreen,
  normalizeScreen,
  pushScreen,
} from "./screenHistory";

const lines = (prefix: string, n: number) => Array.from({ length: n }, (_, i) => `${prefix}-${i}`).join("\n");

describe("normalizeScreen", () => {
  it("strips ANSI, trailing spaces, repeated lines and blank edges", () => {
    const raw = "\n\n\x1b[32mhello\x1b[0m   \nspinner\nspinner\nspinner\nworld  \n\n";
    expect(normalizeScreen(raw)).toBe("hello\nspinner\nworld");
  });

  it("returns an empty string for a blank screen", () => {
    expect(normalizeScreen("   \n \n")).toBe("");
    expect(normalizeScreen("")).toBe("");
  });
});

describe("isSameScreen", () => {
  it("treats identical screens as the same", () => {
    expect(isSameScreen("a\nb\nc", "a\nb\nc")).toBe(true);
  });

  // 門檻 70%：10 行裡有 7 行在對方畫面裡＝同一畫面；只有 6 行＝不同。
  it("is true at 70% overlap and false just below it", () => {
    const older = lines("L", 10);
    const seven = [...older.split("\n").slice(0, 7), "x-1", "x-2", "x-3"].join("\n");
    const six = [...older.split("\n").slice(0, 6), "x-1", "x-2", "x-3", "x-4"].join("\n");
    expect(isSameScreen(seven, older)).toBe(true);
    expect(isSameScreen(six, older)).toBe(false);
  });

  it("is symmetric: a partial screen is the same as the completed screen that contains it", () => {
    const partial = lines("L", 5);
    const full = lines("L", 6);
    expect(isSameScreen(partial, full)).toBe(true);
    expect(isSameScreen(full, partial)).toBe(true);
  });

  it("a small partial screen counts as the same as the big screen that contains it, both ways round", () => {
    const small = lines("L", 3);
    const big = lines("L", 12);
    expect(isSameScreen(small, big)).toBe(true);
    expect(isSameScreen(big, small)).toBe(true);
  });

  it("is false for unrelated screens", () => {
    expect(isSameScreen(lines("A", 8), lines("B", 8))).toBe(false);
  });
});

describe("pushScreen", () => {
  it("appends a new, different screen", () => {
    const h = pushScreen(pushScreen([], lines("A", 8)), lines("B", 8));
    expect(h).toHaveLength(2);
  });

  it("ignores blank screens", () => {
    expect(pushScreen([], "  \n ")).toEqual([]);
  });

  it("replaces the last screen when the new one is a redraw of it", () => {
    const first = lines("L", 10);
    const redraw = lines("L", 10) + "\nextra-line";
    const h = pushScreen(pushScreen([], first), redraw);
    expect(h).toHaveLength(1);
    expect(h[0]).toContain("extra-line");
  });

  it("does not let a tiny screen replace a big one it is contained in", () => {
    const big = lines("L", 20);
    const h = pushScreen(pushScreen([], big), "L-3");
    expect(h).toHaveLength(1);
    expect(h[0]).toBe(normalizeScreen(big));
  });

  it("drops the oldest screens beyond MAX_SCREENS", () => {
    let h: string[] = [];
    for (let i = 0; i < MAX_SCREENS + 3; i++) h = pushScreen(h, lines(`S${i}`, 6));
    expect(h).toHaveLength(MAX_SCREENS);
    expect(h[0]).toContain("S3-0");
    expect(h[h.length - 1]).toContain(`S${MAX_SCREENS + 2}-0`);
  });

  it("drops the oldest screens when the total size is over the limit, but always keeps the newest", () => {
    const big = (tag: string) => `${tag}\n` + "x".repeat(MAX_HISTORY_CHARS / 2 - 100);
    let h = pushScreen([], big("one"));
    h = pushScreen(h, big("two"));
    expect(h).toHaveLength(2);
    h = pushScreen(h, big("three"));
    expect(h.join("").length).toBeLessThanOrEqual(MAX_HISTORY_CHARS);
    expect(h[h.length - 1]).toContain("three");
    expect(h.some((s) => s.startsWith("one"))).toBe(false);
  });

  it("keeps a single screen even when it alone is over the size limit", () => {
    const h = pushScreen([], "only\n" + "x".repeat(MAX_HISTORY_CHARS + 500));
    expect(h).toHaveLength(1);
  });

  it("does not mutate the array it was given", () => {
    const h: string[] = [];
    pushScreen(h, lines("A", 5));
    expect(h).toEqual([]);
  });
});

describe("formatHistoryForPrompt", () => {
  it("returns an empty string when there is no history", () => {
    expect(formatHistoryForPrompt([], "current", 1000)).toBe("");
  });

  it("lists older screens oldest to newest", () => {
    const out = formatHistoryForPrompt([lines("A", 6), lines("B", 6)], lines("C", 6), 10_000);
    expect(out.indexOf("A-0")).toBeGreaterThanOrEqual(0);
    expect(out.indexOf("A-0")).toBeLessThan(out.indexOf("B-0"));
  });

  it("leaves out the newest history screen when it is just the current screen", () => {
    const current = lines("C", 8);
    const out = formatHistoryForPrompt([lines("A", 8), current], current, 10_000);
    expect(out).toContain("A-0");
    expect(out).not.toContain("C-0");
  });

  it("returns an empty string when the only history is the current screen", () => {
    const current = lines("C", 8);
    expect(formatHistoryForPrompt([current], current, 10_000)).toBe("");
  });

  it("keeps the newest screens and drops the oldest when over budget", () => {
    const a = lines("A", 6), b = lines("B", 6), c = lines("C", 6);
    const budget = b.length + c.length + 20;
    const out = formatHistoryForPrompt([a, b, c], lines("Z", 6), budget);
    expect(out).toContain("C-0");
    expect(out).toContain("B-0");
    expect(out).not.toContain("A-0");
  });

  it("never exceeds the budget, even when a single screen is bigger than it", () => {
    const huge = "x".repeat(5000);
    const out = formatHistoryForPrompt([huge], "other", 1000);
    expect(out.length).toBeLessThanOrEqual(1000);
    expect(out.length).toBeGreaterThan(0);
  });
});
