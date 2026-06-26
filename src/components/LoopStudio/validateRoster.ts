import type { OrchestratorAgent } from "../../hooks/useOrchestratorLoop";

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
): ValidationIssue[] {
  const issues: ValidationIssue[] = [];

  // ── Errors (block start) ──────────────────────────────────────────────────

  if (agents.length === 0) {
    issues.push({ level: "error", message: "請至少新增一個 Sub-agent" });
    return issues; // no point checking further
  }

  if (!orchestratorProvider) {
    issues.push({ level: "error", message: "請選擇 Orchestrator 的 AI Provider" });
  }

  if (!verifierProvider) {
    issues.push({ level: "error", message: "請選擇 Verifier 的 AI Provider" });
  }

  agents.forEach((a, i) => {
    if (!a.name.trim()) {
      issues.push({ level: "error", message: `第 ${i + 1} 個 Agent 尚未命名` });
    }
    if (!a.roleDescription.trim()) {
      issues.push({ level: "error", message: `Agent「${a.name || `#${i + 1}`}」缺少角色描述，Orchestrator 不知道何時呼叫它` });
    }
    if (a.tools.length === 0) {
      issues.push({ level: "error", message: `Agent「${a.name || `#${i + 1}`}」沒有啟用任何工具，無法執行任何動作` });
    }
  });

  // ── Warnings (informational) ──────────────────────────────────────────────

  const allTools = new Set(agents.flatMap(a => a.tools));

  if (WRITE_KEYWORDS.test(goal) && !allTools.has("write_file")) {
    issues.push({
      level: "warning",
      message: "目標包含寫入/建立/修改關鍵字，但沒有任何 Agent 啟用「寫檔」工具",
    });
  }

  if (EXEC_KEYWORDS.test(goal) && !allTools.has("execute_command")) {
    issues.push({
      level: "warning",
      message: "目標包含執行/測試/建置關鍵字，但沒有任何 Agent 啟用「執行指令」工具",
    });
  }

  if (READ_KEYWORDS.test(goal) && !allTools.has("read_file")) {
    issues.push({
      level: "warning",
      message: "目標包含讀取/分析關鍵字，但沒有任何 Agent 啟用「讀檔」工具",
    });
  }

  if (agents.length === 1) {
    issues.push({
      level: "warning",
      message: "只有一個 Agent，Orchestrator 無法分工。建議依任務類型加入 2–3 個角色不同的 Agent",
    });
  }

  const hasDuplicateRoles = agents.some((a, i) =>
    agents.slice(i + 1).some(b =>
      a.tools.sort().join() === b.tools.sort().join() &&
      a.roleDescription.trim() === b.roleDescription.trim()
    )
  );
  if (hasDuplicateRoles) {
    issues.push({
      level: "warning",
      message: "有兩個 Agent 的角色描述與工具完全相同，建議調整其中一個的定位",
    });
  }

  if (orchestratorProvider && orchestratorProvider === verifierProvider) {
    issues.push({
      level: "warning",
      message: "Orchestrator 和 Verifier 使用相同 Provider，可能影響評估客觀性",
    });
  }

  return issues;
}
