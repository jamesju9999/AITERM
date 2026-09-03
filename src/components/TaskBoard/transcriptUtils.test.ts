import { describe, it, expect } from "vitest";
import { stripAnsiCodes, collapseConsecutiveDuplicateLines } from "./transcriptUtils";

describe("stripAnsiCodes", () => {
  it("removes SGR color/style codes", () => {
    // \x1b[32m = green, \x1b[1m = bold, \x1b[0m = reset
    const input = "\x1b[32mhello\x1b[0m \x1b[1mworld\x1b[0m";
    expect(stripAnsiCodes(input)).toBe("hello world");
  });

  it("removes cursor-movement CSI sequences", () => {
    // \x1b[2K = erase line, \x1b[1A = cursor up 1
    const input = "line one\x1b[2K\x1b[1Aline two";
    expect(stripAnsiCodes(input)).toBe("line oneline two");
  });

  it("leaves plain text with no escape codes untouched", () => {
    expect(stripAnsiCodes("just plain text\nwith newlines")).toBe("just plain text\nwith newlines");
  });

  it("leaves an empty string untouched", () => {
    expect(stripAnsiCodes("")).toBe("");
  });
});

describe("collapseConsecutiveDuplicateLines (existing, unchanged)", () => {
  it("still collapses duplicate lines", () => {
    expect(collapseConsecutiveDuplicateLines("a\nb\nb\nb\nc")).toBe("a\nb\nc");
  });
});
