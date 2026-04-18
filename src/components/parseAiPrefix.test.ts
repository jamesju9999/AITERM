import { describe, expect, it } from "vitest";
import { parseAiPrefix } from "./parseAiPrefix";

describe("parseAiPrefix", () => {
  it("returns the query for a valid /ai line", () => {
    expect(parseAiPrefix("/ai list files")).toBe("list files");
  });

  it("collapses multiple spaces after /ai", () => {
    expect(parseAiPrefix("/ai   hello world")).toBe("hello world");
  });

  it("returns null when /ai has no arguments", () => {
    expect(parseAiPrefix("/ai")).toBeNull();
  });

  it("returns null when /ai is followed only by whitespace", () => {
    expect(parseAiPrefix("/ai   ")).toBeNull();
  });

  it("returns null when /ai is not at the start", () => {
    expect(parseAiPrefix("  /ai list files")).toBeNull();
    expect(parseAiPrefix("echo /ai list")).toBeNull();
  });

  it("is case-sensitive — only lowercase /ai counts", () => {
    expect(parseAiPrefix("/AI list")).toBeNull();
    expect(parseAiPrefix("/Ai list")).toBeNull();
  });

  it("returns null for unrelated lines", () => {
    expect(parseAiPrefix("ls -la")).toBeNull();
    expect(parseAiPrefix("")).toBeNull();
  });
});
