import { describe, expect, it } from "vitest";
import { isClaudeCommand } from "./claudeCommand";

describe("isClaudeCommand", () => {
  it("認得單獨的 claude", () => {
    expect(isClaudeCommand("claude")).toBe(true);
  });

  it("認得帶參數的 claude", () => {
    expect(isClaudeCommand("claude --resume")).toBe(true);
  });

  it("認得完整路徑", () => {
    expect(isClaudeCommand("/usr/local/bin/claude")).toBe(true);
    expect(isClaudeCommand("C:\\Users\\me\\bin\\claude --continue")).toBe(true);
  });

  it("忽略前後空白", () => {
    expect(isClaudeCommand("  claude  ")).toBe(true);
  });

  it("不認別的指令", () => {
    expect(isClaudeCommand("claude-foo")).toBe(false);
    expect(isClaudeCommand("myclaude")).toBe(false);
  });

  it("claude 只是參數時不算", () => {
    expect(isClaudeCommand("echo claude")).toBe(false);
    expect(isClaudeCommand("which claude")).toBe(false);
  });

  it("空字串不算", () => {
    expect(isClaudeCommand("")).toBe(false);
    expect(isClaudeCommand("   ")).toBe(false);
  });
});
