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
});
