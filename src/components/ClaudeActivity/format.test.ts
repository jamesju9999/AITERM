import { describe, expect, it } from "vitest";
import { formatDuration, formatTokens } from "./format";

describe("formatDuration", () => {
  it.each([
    [0, "0s"],
    [4958, "4.9s"],
    [59_000, "59s"],
    [65_000, "1m 5s"],
    [3_725_000, "1h 2m"],
  ])("%i ms → %s", (ms, text) => expect(formatDuration(ms)).toBe(text));

  it("負數（時鐘偏差）當 0", () => expect(formatDuration(-5)).toBe("0s"));
});

describe("formatTokens", () => {
  it.each([
    [0, "0"],
    [999, "999"],
    [1000, "1.0k"],
    [20484, "20.5k"],
    [1_250_000, "1.3M"],
  ])("%i → %s", (n, text) => expect(formatTokens(n)).toBe(text));
});
