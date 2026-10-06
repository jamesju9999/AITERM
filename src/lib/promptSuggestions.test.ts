import { describe, expect, it } from "vitest";
import { parseSuggestions, buildSuggestionRequest, MAX_SUGGESTIONS, MAX_PROMPT_CHARS } from "./promptSuggestions";

describe("parseSuggestions", () => {
  it("parses a plain JSON array", () => {
    const r = parseSuggestions('[{"title":"補測試","prompt":"幫剛才的修改補單元測試"}]');
    expect(r).toEqual([{ title: "補測試", prompt: "幫剛才的修改補單元測試" }]);
  });

  it("tolerates code fences and surrounding prose", () => {
    const raw = '好的，建議如下：\n```json\n[{"title":"A","prompt":"do a"},{"title":"B","prompt":"do b"}]\n```\n希望有幫助';
    expect(parseSuggestions(raw).map((s) => s.title)).toEqual(["A", "B"]);
  });

  it("returns [] for non-JSON or non-array replies", () => {
    expect(parseSuggestions("沒有 JSON")).toEqual([]);
    expect(parseSuggestions('{"title":"A","prompt":"b"}')).toEqual([]);
    expect(parseSuggestions("")).toEqual([]);
    expect(parseSuggestions(null)).toEqual([]);
  });

  it("drops entries with empty or non-string fields", () => {
    const raw = JSON.stringify([
      { title: "ok", prompt: "fine" },
      { title: "", prompt: "no title" },
      { title: "no prompt", prompt: "   " },
      { title: 1, prompt: "x" },
      null,
    ]);
    expect(parseSuggestions(raw)).toEqual([{ title: "ok", prompt: "fine" }]);
  });

  it("dedupes by prompt and caps the count", () => {
    const items = Array.from({ length: 9 }, (_, i) => ({ title: `t${i}`, prompt: `p${i}` }));
    items.push({ title: "dup", prompt: "p0" });
    const r = parseSuggestions(JSON.stringify(items));
    expect(r).toHaveLength(MAX_SUGGESTIONS);
    expect(new Set(r.map((s) => s.prompt)).size).toBe(MAX_SUGGESTIONS);
  });

  it("truncates over-long prompts", () => {
    const long = "x".repeat(MAX_PROMPT_CHARS + 50);
    const [s] = parseSuggestions(JSON.stringify([{ title: "t", prompt: long }]));
    expect(s.prompt.length).toBe(MAX_PROMPT_CHARS);
  });
});

describe("buildSuggestionRequest", () => {
  it("embeds the screen text and the language directive", () => {
    const msg = buildSuggestionRequest("SCREEN-TEXT", "請使用繁體中文");
    expect(msg).toContain("SCREEN-TEXT");
    expect(msg).toContain("請使用繁體中文");
    expect(msg).toContain("JSON");
  });

  it("keeps only the tail of very long screens", () => {
    const screen = "A".repeat(20000) + "TAIL";
    const msg = buildSuggestionRequest(screen, "");
    expect(msg).toContain("TAIL");
    expect(msg.length).toBeLessThan(12000);
  });
});
