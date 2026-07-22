import { describe, expect, it } from "vitest";
import { findNextBlockMatch, findPreviousBlockMatch, blockPlainText } from "./blockSearch";
import type { TerminalBlock } from "../hooks/useTerminalBlocks";

function makeBlock(id: string, text: string): TerminalBlock {
  return {
    id,
    command: `cmd-${id}`,
    status: "completed",
    exitCode: 0,
    startTime: 0,
    rawOutput: text,
    renderedLines: [{ spans: [{ text }] }],
  };
}

describe("blockPlainText", () => {
  it("joins all span text across all lines", () => {
    const block = makeBlock("a", "hello world");
    expect(blockPlainText(block)).toBe("hello world");
  });
});

describe("findNextBlockMatch", () => {
  it("finds a match in the first block after the given cursor", () => {
    const blocks = [makeBlock("a", "foo bar"), makeBlock("b", "no match here")];
    const match = findNextBlockMatch(blocks, "bar", null);
    expect(match?.blockId).toBe("a");
  });

  it("skips to the next block when the query isn't in the current block", () => {
    const blocks = [makeBlock("a", "foo"), makeBlock("b", "target")];
    const match = findNextBlockMatch(blocks, "target", { blockId: "a", offset: 0 });
    expect(match?.blockId).toBe("b");
  });

  it("returns null when no block contains the query", () => {
    const blocks = [makeBlock("a", "foo"), makeBlock("b", "bar")];
    expect(findNextBlockMatch(blocks, "zzz", null)).toBeNull();
  });
});

describe("findPreviousBlockMatch", () => {
  it("finds a match in the block before the given cursor", () => {
    const blocks = [makeBlock("a", "target"), makeBlock("b", "no match")];
    const match = findPreviousBlockMatch(blocks, "target", { blockId: "b", offset: 0 });
    expect(match?.blockId).toBe("a");
  });
});
