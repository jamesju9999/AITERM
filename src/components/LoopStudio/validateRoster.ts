import type { OrchestratorAgent } from "../../hooks/useOrchestratorLoop";
import type { Translations } from "../../lib/i18n";

export interface ValidationIssue {
  level: "error" | "warning";
  message: string;
}

const WRITE_KEYWORDS = /寫|建立|建檔|產生|修改|新增|創建|create|write|generate|modify|update|edit/i;
const EXEC_KEYWORDS = /執行|跑|測試|建置|運行|編譯|run|test|build|execute|compile|install/i;
const READ_KEYWORDS = /讀|分析|查看|搜尋|掃描|read|analyze|search|scan|inspect|review/i;

export function validateRoster(
  agents: OrchestratorAgent[],
  orchestratorProvider: string,
  verifierProvider: string,
  goal: string,
  t: Translations,
): ValidationIssue[] {
  const issues: ValidationIssue[] = [];

  // ── Errors (block start) ──────────────────────────────────────────────────

  if (agents.length === 0) {
    issues.push({ level: "error", message: t.ls_validate_no_agents });
    return issues;
  }

  if (!orchestratorProvider) {
    issues.push({ level: "error", message: t.ls_validate_no_orch_provider });
  }

  if (!verifierProvider) {
    issues.push({ level: "error", message: t.ls_validate_no_verifier_provider });
  }

  agents.forEach((a, i) => {
    if (!a.name.trim()) {
      issues.push({ level: "error", message: t.ls_validate_agent_no_name(i + 1) });
    }
    if (!a.roleDescription.trim()) {
      issues.push({ level: "error", message: t.ls_validate_agent_no_desc(a.name || `#${i + 1}`) });
    }
    if (a.tools.length === 0) {
      issues.push({ level: "error", message: t.ls_validate_agent_no_tools(a.name || `#${i + 1}`) });
    }
  });

  // ── Warnings (informational) ──────────────────────────────────────────────

  const allTools = new Set(agents.flatMap(a => a.tools));

  if (WRITE_KEYWORDS.test(goal) && !allTools.has("write_file")) {
    issues.push({ level: "warning", message: t.ls_validate_no_write_tool });
  }

  if (EXEC_KEYWORDS.test(goal) && !allTools.has("execute_command")) {
    issues.push({ level: "warning", message: t.ls_validate_no_exec_tool });
  }

  if (READ_KEYWORDS.test(goal) && !allTools.has("read_file")) {
    issues.push({ level: "warning", message: t.ls_validate_no_read_tool });
  }

  if (agents.length === 1) {
    issues.push({ level: "warning", message: t.ls_validate_single_agent });
  }

  const hasDuplicateRoles = agents.some((a, i) =>
    agents.slice(i + 1).some(b =>
      a.tools.sort().join() === b.tools.sort().join() &&
      a.roleDescription.trim() === b.roleDescription.trim()
    )
  );
  if (hasDuplicateRoles) {
    issues.push({ level: "warning", message: t.ls_validate_duplicate_roles });
  }

  if (orchestratorProvider && orchestratorProvider === verifierProvider) {
    issues.push({ level: "warning", message: t.ls_validate_same_provider });
  }

  return issues;
}
