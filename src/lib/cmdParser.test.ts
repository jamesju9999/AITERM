import { describe, expect, it } from "vitest";
import { parseCmdTags } from "./cmdParser";

describe("parseCmdTags", () => {
  it("returns a single text part for pure text", () => {
    const parts = parseCmdTags("just some words");
    expect(parts).toEqual([{ type: "text", content: "just some words" }]);
  });

  it("returns empty array for empty string", () => {
    expect(parseCmdTags("")).toEqual([]);
  });

  it("extracts a single single-line cmd", () => {
    const parts = parseCmdTags("試試 <cmd>ls -la</cmd> 看看");
    expect(parts).toEqual([
      { type: "text", content: "試試 " },
      { type: "cmd", content: "ls -la", multiline: false },
      { type: "text", content: " 看看" },
    ]);
  });

  it("extracts multiple cmds", () => {
    const parts = parseCmdTags("先 <cmd>cd /tmp</cmd> 再 <cmd>ls</cmd>");
    expect(parts).toHaveLength(4);
    expect(parts[1]).toEqual({ type: "cmd", content: "cd /tmp", multiline: false });
    expect(parts[3]).toEqual({ type: "cmd", content: "ls", multiline: false });
  });

  it("trims whitespace inside cmd", () => {
    const parts = parseCmdTags("<cmd>  ls   </cmd>");
    expect(parts[0]).toEqual({ type: "cmd", content: "ls", multiline: false });
  });

  it("marks multiline=true when cmd contains newlines", () => {
    const parts = parseCmdTags("<cmd>cd /tmp\nls -la</cmd>");
    expect(parts).toHaveLength(1);
    expect(parts[0]).toEqual({
      type: "cmd",
      content: "cd /tmp\nls -la",
      multiline: true,
    });
  });

  it("treats unclosed <cmd> as plain text", () => {
    const parts = parseCmdTags("oops <cmd>ls never closes");
    expect(parts).toEqual([
      { type: "text", content: "oops <cmd>ls never closes" },
    ]);
  });

  it("handles nested with non-greedy match (takes inner first pair)", () => {
    // Non-greedy regex matches the first complete pair: <cmd>a<cmd>b</cmd>
    // which yields cmd content "a<cmd>b". The trailing </cmd> becomes text.
    const parts = parseCmdTags("<cmd>a<cmd>b</cmd></cmd>");
    expect(parts[0]).toEqual({
      type: "cmd",
      content: "a<cmd>b",
      multiline: false,
    });
    expect(parts[1]).toEqual({ type: "text", content: "</cmd>" });
  });

  it("handles cmd at very start", () => {
    const parts = parseCmdTags("<cmd>ls</cmd> done");
    expect(parts[0]).toEqual({ type: "cmd", content: "ls", multiline: false });
    expect(parts[1]).toEqual({ type: "text", content: " done" });
  });

  it("handles cmd at very end", () => {
    const parts = parseCmdTags("run <cmd>ls</cmd>");
    expect(parts[0]).toEqual({ type: "text", content: "run " });
    expect(parts[1]).toEqual({ type: "cmd", content: "ls", multiline: false });
  });
});
