import { describe, expect, it } from "vitest";
import {
  BUILTIN_AI_CLI_NAMES,
  isAiCliCommand,
  normalizeCliName,
  parseCustomNames,
} from "./aiCliCommand";

describe("isAiCliCommand", () => {
  it.each(BUILTIN_AI_CLI_NAMES)("recognises the built-in tool %s", (name) => {
    expect(isAiCliCommand(name)).toBe(true);
    expect(isAiCliCommand(`${name} --help`)).toBe(true);
  });

  it("ignores leading whitespace, path prefixes (both separators) and case", () => {
    expect(isAiCliCommand("   claude")).toBe(true);
    expect(isAiCliCommand("/usr/local/bin/claude -p hi")).toBe(true);
    expect(isAiCliCommand("C:\\Users\\me\\AppData\\Roaming\\npm\\codex")).toBe(true);
    expect(isAiCliCommand("CLAUDE")).toBe(true);
  });

  it("strips Windows executable extensions", () => {
    expect(isAiCliCommand("codex.cmd")).toBe(true);
    expect(isAiCliCommand("C:\\bin\\gemini.exe --yolo")).toBe(true);
    expect(isAiCliCommand("aider.ps1")).toBe(true);
    expect(isAiCliCommand("claude.bat")).toBe(true);
  });

  it("does not match names that merely start or end with a tool name", () => {
    expect(isAiCliCommand("claude-helper")).toBe(false);
    expect(isAiCliCommand("myclaude")).toBe(false);
    expect(isAiCliCommand("codex-cli-wrapper run")).toBe(false);
    expect(isAiCliCommand("claude.py")).toBe(false);
  });

  it("only looks at the first token, so a tool name as an argument does not count", () => {
    expect(isAiCliCommand("ls claude")).toBe(false);
    expect(isAiCliCommand("echo codex")).toBe(false);
    expect(isAiCliCommand("cat gemini.md")).toBe(false);
  });

  it("does not support env prefixes or npx wrappers (documented limitation)", () => {
    expect(isAiCliCommand("FOO=1 claude")).toBe(false);
    expect(isAiCliCommand("npx codex")).toBe(false);
  });

  it("returns false for empty input", () => {
    expect(isAiCliCommand("")).toBe(false);
    expect(isAiCliCommand("   ")).toBe(false);
  });

  it("honours custom names", () => {
    expect(isAiCliCommand("mytool run", ["mytool"])).toBe(true);
    expect(isAiCliCommand("mytool run")).toBe(false);
    expect(isAiCliCommand("claude", ["mytool"])).toBe(true);
  });
});

describe("normalizeCliName", () => {
  it("trims, lowercases and strips executable extensions", () => {
    expect(normalizeCliName("  MyTool ")).toBe("mytool");
    expect(normalizeCliName("mytool.exe")).toBe("mytool");
  });

  it("rejects values that are empty, contain spaces, or contain path separators", () => {
    expect(normalizeCliName("")).toBeNull();
    expect(normalizeCliName("   ")).toBeNull();
    expect(normalizeCliName("my tool")).toBeNull();
    expect(normalizeCliName("bin/mytool")).toBeNull();
    expect(normalizeCliName("bin\\mytool")).toBeNull();
    expect(normalizeCliName(".exe")).toBeNull();
  });
});

describe("parseCustomNames", () => {
  it("parses a JSON string array, normalising and de-duplicating", () => {
    expect(parseCustomNames('["MyTool","mytool","other.exe"]')).toEqual(["mytool", "other"]);
  });

  it("drops invalid entries and tolerates garbage", () => {
    expect(parseCustomNames('["ok", 3, null, "has space", ""]')).toEqual(["ok"]);
    expect(parseCustomNames("not json")).toEqual([]);
    expect(parseCustomNames('{"a":1}')).toEqual([]);
    expect(parseCustomNames(null)).toEqual([]);
  });

  it("does not return names that are already built in", () => {
    expect(parseCustomNames('["claude","mytool"]')).toEqual(["mytool"]);
  });
});
