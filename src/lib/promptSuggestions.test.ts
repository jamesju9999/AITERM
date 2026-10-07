import { describe, expect, it } from "vitest";
import { parseSuggestions, buildSuggestionRequest, buildGoalPolishRequest, cleanPolishedGoal, buildPromptAssistRequest, cleanGeneratedText, MAX_ASSIST_PROMPT_CHARS, MAX_ASSIST_REQUEST_CHARS, MAX_ASSIST_SCREEN_CHARS, MAX_SUGGESTIONS, MAX_PROMPT_CHARS, MAX_GOAL_CHARS } from "./promptSuggestions";

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

describe("buildSuggestionRequest with a goal", () => {
  it("states the goal and asks every suggestion to move toward it", () => {
    const msg = buildSuggestionRequest("SCREEN", "", "將舊程式的 Client-Server 架構轉換為網頁平台架構");
    expect(msg).toContain("將舊程式的 Client-Server 架構轉換為網頁平台架構");
    expect(msg).toContain("大目標");
    expect(msg).toContain("SCREEN");
  });

  it("is identical to the goal-less request when the goal is empty or blank", () => {
    const base = buildSuggestionRequest("SCREEN", "LANG");
    expect(buildSuggestionRequest("SCREEN", "LANG", "")).toBe(base);
    expect(buildSuggestionRequest("SCREEN", "LANG", "   \n ")).toBe(base);
    expect(buildSuggestionRequest("SCREEN", "LANG", undefined)).toBe(base);
    expect(base).not.toContain("大目標");
  });

  it("truncates an over-long goal so it cannot swamp the request", () => {
    const msg = buildSuggestionRequest("SCREEN", "", "目".repeat(MAX_GOAL_CHARS + 300));
    expect(msg).toContain("目".repeat(MAX_GOAL_CHARS));
    expect(msg).not.toContain("目".repeat(MAX_GOAL_CHARS + 1));
  });
});

describe("buildGoalPolishRequest", () => {
  it("carries the draft and the language, and forbids inventing new requirements", () => {
    const msg = buildGoalPolishRequest("轉成網頁版", "請使用繁體中文");
    expect(msg).toContain("轉成網頁版");
    expect(msg).toContain("請使用繁體中文");
    expect(msg).toContain("不要");
  });

  it("truncates an over-long draft", () => {
    const msg = buildGoalPolishRequest("目".repeat(MAX_GOAL_CHARS + 300), "");
    expect(msg).toContain("目".repeat(MAX_GOAL_CHARS));
    expect(msg).not.toContain("目".repeat(MAX_GOAL_CHARS + 1));
  });
});

describe("cleanPolishedGoal", () => {
  it("returns plain text trimmed", () => {
    expect(cleanPolishedGoal("  將舊系統改成網頁版  ")).toBe("將舊系統改成網頁版");
  });

  it("strips a code fence and wrapping quotes the model likes to add", () => {
    expect(cleanPolishedGoal("```\n將舊系統改成網頁版\n```")).toBe("將舊系統改成網頁版");
    expect(cleanPolishedGoal("```text\n目標內容\n```")).toBe("目標內容");
    expect(cleanPolishedGoal('"目標內容"')).toBe("目標內容");
    expect(cleanPolishedGoal("“目標內容”")).toBe("目標內容");
  });

  it("keeps quotes that are part of the text", () => {
    expect(cleanPolishedGoal('把 "舊系統" 轉成網頁版')).toBe('把 "舊系統" 轉成網頁版');
  });

  it("returns an empty string for null or blank", () => {
    expect(cleanPolishedGoal(null)).toBe("");
    expect(cleanPolishedGoal("  \n")).toBe("");
  });

  it("truncates to the goal limit", () => {
    expect(cleanPolishedGoal("目".repeat(MAX_GOAL_CHARS + 50)).length).toBe(MAX_GOAL_CHARS);
  });
});

describe("buildSuggestionRequest with screen history", () => {
  it("puts the older screens before the current one and says why they are there", () => {
    const msg = buildSuggestionRequest("CURRENT-SCREEN", "", undefined, "OLDER-SCREEN");
    expect(msg).toContain("OLDER-SCREEN");
    expect(msg).toContain("較早的畫面");
    expect(msg.indexOf("OLDER-SCREEN")).toBeLessThan(msg.indexOf("CURRENT-SCREEN"));
    expect(msg).toContain("已完成的事不要再建議");
  });

  it("is byte-identical to the request without history when the history is empty", () => {
    const base = buildSuggestionRequest("SCREEN", "LANG", "目標");
    expect(buildSuggestionRequest("SCREEN", "LANG", "目標", "")).toBe(base);
    expect(buildSuggestionRequest("SCREEN", "LANG", "目標", undefined)).toBe(base);
    expect(base).not.toContain("較早的畫面");
  });

  it("works together with a goal", () => {
    const msg = buildSuggestionRequest("CUR", "", "把舊系統轉成網頁版", "OLD");
    expect(msg).toContain("把舊系統轉成網頁版");
    expect(msg).toContain("OLD");
  });
});

describe("buildSuggestionRequest with milestones", () => {
  it("puts the milestone section after the goal and before the history and the current screen", () => {
    const msg = buildSuggestionRequest("CURRENT", "", "我的目標", "OLDER", "MILESTONE-SECTION");
    expect(msg).toContain("MILESTONE-SECTION");
    expect(msg.indexOf("我的目標")).toBeLessThan(msg.indexOf("MILESTONE-SECTION"));
    expect(msg.indexOf("MILESTONE-SECTION")).toBeLessThan(msg.indexOf("OLDER"));
    expect(msg.indexOf("OLDER")).toBeLessThan(msg.indexOf("CURRENT"));
  });

  it("is byte-identical to the request without milestones when the section is empty", () => {
    const base = buildSuggestionRequest("SCREEN", "LANG", "目標", "OLD");
    expect(buildSuggestionRequest("SCREEN", "LANG", "目標", "OLD", "")).toBe(base);
    expect(buildSuggestionRequest("SCREEN", "LANG", "目標", "OLD", undefined)).toBe(base);
  });
});

describe("buildPromptAssistRequest", () => {
  const base = { request: "幫我把登入改成 REST", languageDirective: "請使用繁體中文" };

  it("carries the user's rough request and the language, and forbids inventing requirements", () => {
    const msg = buildPromptAssistRequest(base);
    expect(msg).toContain("幫我把登入改成 REST");
    expect(msg).toContain("請使用繁體中文");
    expect(msg).toContain("不要新增");
    expect(msg).toContain("〈");
  });

  it("includes the goal, the milestone section and the screen only when given, in that order", () => {
    const msg = buildPromptAssistRequest({ ...base, goal: "網頁化", milestones: "里程碑一節", screen: "SCREEN-TEXT" });
    expect(msg).toContain("網頁化");
    expect(msg).toContain("里程碑一節");
    expect(msg).toContain("SCREEN-TEXT");
    expect(msg.indexOf("網頁化")).toBeLessThan(msg.indexOf("里程碑一節"));
    expect(msg.indexOf("里程碑一節")).toBeLessThan(msg.indexOf("SCREEN-TEXT"));
    expect(msg.indexOf("SCREEN-TEXT")).toBeLessThan(msg.indexOf("幫我把登入改成 REST"));
  });

  it("leaves out the optional sections when empty", () => {
    const msg = buildPromptAssistRequest({ ...base, goal: "  ", milestones: "", screen: "" });
    expect(msg).not.toContain("大目標");
    expect(msg).not.toContain("終端機畫面");
  });

  it("truncates an over-long request and keeps only the tail of a long screen", () => {
    const msg = buildPromptAssistRequest({
      ...base,
      request: "需".repeat(MAX_ASSIST_REQUEST_CHARS + 50),
      screen: "A".repeat(MAX_ASSIST_SCREEN_CHARS + 500) + "TAIL",
    });
    expect(msg).toContain("需".repeat(MAX_ASSIST_REQUEST_CHARS));
    expect(msg).not.toContain("需".repeat(MAX_ASSIST_REQUEST_CHARS + 1));
    expect(msg).toContain("TAIL");
    expect(msg).not.toContain("A".repeat(MAX_ASSIST_SCREEN_CHARS + 1));
  });
});

describe("cleanGeneratedText", () => {
  it("strips fences and wrapping quotes, trims, and applies the given limit", () => {
    expect(cleanGeneratedText("```\n請幫我重構\n```", 100)).toBe("請幫我重構");
    expect(cleanGeneratedText("「請幫我重構」", 100)).toBe("請幫我重構");
    expect(cleanGeneratedText("  abcdef  ", 3)).toBe("abc");
    expect(cleanGeneratedText(null, 10)).toBe("");
  });

  it("keeps a multi-line prompt intact", () => {
    expect(cleanGeneratedText("第一行\n第二行", 100)).toBe("第一行\n第二行");
  });

  it("the assistant limit is larger than the goal limit", () => {
    expect(MAX_ASSIST_PROMPT_CHARS).toBeGreaterThan(MAX_GOAL_CHARS);
  });
});
