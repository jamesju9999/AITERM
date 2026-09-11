import { describe, expect, it } from "vitest";
import { parseAnsiToRenderedLines } from "./ansiBlockParser";

describe("parseAnsiToRenderedLines", () => {
  it("splits plain multi-line text into one RenderedLine per line", async () => {
    const lines = await parseAnsiToRenderedLines("hello\r\nworld\r\n", 80);
    expect(lines.length).toBeGreaterThanOrEqual(2);
    expect(lines[0].spans.map((s) => s.text).join("")).toBe("hello");
    expect(lines[1].spans.map((s) => s.text).join("")).toBe("world");
  });

  it("captures ANSI foreground color as a styled span", async () => {
    const lines = await parseAnsiToRenderedLines("\x1b[32mgreen\x1b[0m plain\r\n", 80);
    const spans = lines[0].spans;
    const greenSpan = spans.find((s) => s.text === "green");
    expect(greenSpan?.fg).toBe("#0dbc79");
    const plainSpan = spans.find((s) => s.text.includes("plain"));
    expect(plainSpan?.fg).toBeUndefined();
  });

  it("captures bold attribute", async () => {
    const lines = await parseAnsiToRenderedLines("\x1b[1mbold text\x1b[0m\r\n", 80);
    const boldSpan = lines[0].spans.find((s) => s.text === "bold text");
    expect(boldSpan?.bold).toBe(true);
  });

  it("trims trailing unstyled blank content from each line", async () => {
    const lines = await parseAnsiToRenderedLines("hi\r\n", 80);
    const totalText = lines[0].spans.map((s) => s.text).join("");
    expect(totalText).toBe("hi");
  });

  it("does not insert phantom spaces after wide (CJK) characters", async () => {
    const lines = await parseAnsiToRenderedLines("你好world\r\n", 80);
    const totalText = lines[0].spans.map((s) => s.text).join("");
    expect(totalText).toBe("你好world");
  });

  it("drops an auto-wrapped continuation row that holds only padding", async () => {
    // Windows PowerShell 5.1 pads table rows to width-1 *characters*, but
    // counts 下午 as 2 cells instead of 4, so the row spills 1 cell of padding
    // onto the next line (real machine: width 135, every dir row 134 chars).
    const row = "d 下午 x".padEnd(19, " ");
    const lines = await parseAnsiToRenderedLines(`${row}\r\nnext\r\n`, 20);
    expect(lines.map((l) => l.spans.map((s) => s.text).join(""))).toEqual(["d 下午 x", "next"]);
  });

  it("keeps real blank lines and wrapped continuations that carry content", async () => {
    const lines = await parseAnsiToRenderedLines(`a\r\n\r\n${"x".repeat(25)}\r\n`, 20);
    expect(lines.map((l) => l.spans.map((s) => s.text).join(""))).toEqual(["a", "", "x".repeat(20), "x".repeat(5)]);
  });
});
