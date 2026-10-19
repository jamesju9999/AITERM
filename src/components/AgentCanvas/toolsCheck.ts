import type { AgentRow, ToolsCheck } from "../../ipc/agents";

export type Tool = "git" | "claude" | "codex";

/**
 * 這個專案缺哪些工具。git 一律需要；AI 工具只列有 Agent 在用的——還沒有 Agent 時
 * 預設要 Claude Code（新建的 Agent 預設用它）。順序固定：git、claude、codex。
 */
export function missingTools(check: ToolsCheck, agents: Pick<AgentRow, "cli">[]): Tool[] {
  const uses = (cli: "claude" | "codex") => agents.some((a) => (a.cli ?? "claude") === cli);
  const needed: Tool[] = ["git"];
  if (agents.length === 0 || uses("claude")) needed.push("claude");
  if (uses("codex")) needed.push("codex");
  return needed.filter((t) => !check[t]);
}

/** 官方安裝指令（見 spec 的表格）。 */
export function installCommand(tool: Tool, platform: ToolsCheck["platform"]): string {
  switch (tool) {
    case "claude":
      return platform === "windows" ? "irm https://claude.ai/install.ps1 | iex" : "curl -fsSL https://claude.ai/install.sh | bash";
    case "codex":
      return "npm install -g @openai/codex";
    case "git":
      return platform === "macos" ? "xcode-select --install" : platform === "windows" ? "winget install --id Git.Git -e" : "sudo apt install git";
  }
}
