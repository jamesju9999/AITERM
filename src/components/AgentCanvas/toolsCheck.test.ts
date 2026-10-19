import { describe, expect, it } from "vitest";

import type { AgentRow, ToolsCheck } from "../../ipc/agents";
import { installCommand, missingTools } from "./toolsCheck";

const all: ToolsCheck = { platform: "macos", claude_command: "claude", claude: "/c", codex: "/x", git: "/g" };
const agent = (cli?: "claude" | "codex") => ({ cli }) as AgentRow;

describe("missingTools", () => {
  it("都裝了就沒有缺件", () => {
    expect(missingTools(all, [agent()])).toEqual([]);
  });

  it("git 一律需要；還沒有 Agent 時預設要 Claude Code", () => {
    expect(missingTools({ ...all, git: null, claude: null, codex: null }, [])).toEqual(["git", "claude"]);
  });

  it("只列這個專案用得到的 AI 工具（舊資料沒有 cli = claude）", () => {
    const none = { ...all, claude: null, codex: null };
    expect(missingTools(none, [agent()])).toEqual(["claude"]);
    expect(missingTools(none, [agent("codex")])).toEqual(["codex"]);
    expect(missingTools(none, [agent("codex"), agent("claude")])).toEqual(["claude", "codex"]);
  });
});

describe("installCommand", () => {
  it("依平台給官方安裝指令", () => {
    expect(installCommand("claude", "macos")).toContain("claude.ai/install.sh");
    expect(installCommand("claude", "windows")).toContain("claude.ai/install.ps1");
    expect(installCommand("git", "macos")).toBe("xcode-select --install");
    expect(installCommand("git", "windows")).toContain("winget");
    expect(installCommand("codex", "linux")).toBe("npm install -g @openai/codex");
  });
});
