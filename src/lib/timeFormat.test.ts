import { describe, it, expect } from "vitest";
import { formatDuration } from "./timeFormat";

describe("formatDuration", () => {
  it("shows seconds when under 60s", () => {
    expect(formatDuration(0, 7000)).toBe("7s");
    expect(formatDuration(0, 59000)).toBe("59s");
  });
  it("shows minutes without remainder when exact", () => {
    expect(formatDuration(0, 60000)).toBe("1分");
    expect(formatDuration(0, 120000)).toBe("2分");
  });
  it("shows minutes and seconds when there is a remainder", () => {
    expect(formatDuration(0, 90000)).toBe("1分30s");
    expect(formatDuration(0, 125000)).toBe("2分5s");
  });
  it("rounds to nearest second", () => {
    expect(formatDuration(0, 7400)).toBe("7s");
    expect(formatDuration(0, 7500)).toBe("8s");
  });
  it("handles non-zero startMs", () => {
    expect(formatDuration(1000, 8000)).toBe("7s");
  });
});
