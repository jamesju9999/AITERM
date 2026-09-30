import { describe, it, expect } from "vitest";
import {
  stripAnsiCodes,
  collapseConsecutiveDuplicateLines,
  parseTranscriptTurns,
} from "./transcriptUtils";

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

  // Regression test: real TUI output (Claude Code, via xterm.js's
  // SerializeAddon) includes DEC private-mode CSI sequences — e.g.
  // \x1b[?1049h to enter the alternate screen buffer, \x1b[?2004h for
  // bracketed paste, \x1b[?1004h for focus reporting. These have a `?`
  // between `ESC [` and the digits, which the plain [0-9;]* character class
  // doesn't match, so they leaked through into saved transcripts.
  it("removes DEC private-mode CSI sequences (the '?' variant)", () => {
    const input = "before\x1b[?1049h\x1b[?2004h\x1b[?1004hafter";
    expect(stripAnsiCodes(input)).toBe("beforeafter");
  });
});

describe("collapseConsecutiveDuplicateLines (existing, unchanged)", () => {
  it("still collapses duplicate lines", () => {
    expect(collapseConsecutiveDuplicateLines("a\nb\nb\nb\nc")).toBe("a\nb\nc");
  });
});

describe("parseTranscriptTurns", () => {
  const RULE = "─".repeat(40);

  it("splits output into one turn per user prompt", () => {
    const text = ["❯ first prompt", "answer one", "more one", "❯ second prompt", "answer two"].join("\n");
    const { preamble, turns } = parseTranscriptTurns(text);
    expect(preamble).toBe("");
    expect(turns).toEqual([
      { prompt: "first prompt", output: "answer one\nmore one" },
      { prompt: "second prompt", output: "answer two" },
    ]);
  });

  it("does not treat the bare empty input-box ❯ as a prompt", () => {
    const text = ["some output", RULE, "❯ ", RULE, "status line"].join("\n");
    expect(parseTranscriptTurns(text).turns).toEqual([]);
  });

  it("does not treat unsent text inside the input box as a prompt", () => {
    const text = ["❯ real prompt", "answer", RULE, "❯ half typed", RULE, "status"].join("\n");
    const { turns } = parseTranscriptTurns(text);
    expect(turns).toHaveLength(1);
    expect(turns[0].prompt).toBe("real prompt");
    expect(turns[0].output).toContain("❯ half typed");
  });

  it("puts text before the first prompt into preamble", () => {
    const { preamble, turns } = parseTranscriptTurns("boot noise\n\n❯ hi\nhello");
    expect(preamble).toBe("boot noise");
    expect(turns).toEqual([{ prompt: "hi", output: "hello" }]);
  });

  it("returns no turns when there are no prompts", () => {
    const { turns } = parseTranscriptTurns("just\nplain output");
    expect(turns).toEqual([]);
  });
});
