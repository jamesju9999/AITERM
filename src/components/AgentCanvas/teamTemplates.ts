import type { AgentKind, AgentRow, NewAgentArgs } from "../../ipc/agents";

/** 範本裡的一位 Agent：角色設定與相對位置，不含程式庫（套用時再選）。 */
export interface TemplateAgent {
  kind: AgentKind;
  name: string;
  role: string;
  prompt_prefix: string;
  model: string;
  produces_spec: boolean;
  dx: number;
  dy: number;
}

export interface TeamTemplate {
  name: string;
  agents: TemplateAgent[];
}

/** 跨專案共用，存在這台機器的 localStorage（跟「我的角色」一樣）。 */
const KEY = "aiterm.agentCanvas.teamTemplates";

/** 每個專案只能有一位的角色。 */
const SINGLETONS: AgentKind[] = ["reviewer", "orchestrator", "verifier"];

export function loadTemplates(): TeamTemplate[] {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(KEY) ?? "[]");
    return Array.isArray(v) ? (v as TeamTemplate[]).filter((t) => t && typeof t.name === "string" && Array.isArray(t.agents)) : [];
  } catch {
    return [];
  }
}

/** 同名的覆蓋。 */
export function saveTemplate(tpl: TeamTemplate): TeamTemplate[] {
  const next = [...loadTemplates().filter((t) => t.name !== tpl.name), tpl];
  try {
    localStorage.setItem(KEY, JSON.stringify(next));
  } catch {
    // 存不了（私密視窗、配額）就只留在這次的畫面
  }
  return next;
}

export function deleteTemplate(name: string): TeamTemplate[] {
  const next = loadTemplates().filter((t) => t.name !== name);
  try {
    localStorage.setItem(KEY, JSON.stringify(next));
  } catch {
    // 同上
  }
  return next;
}

/** 目前桌布上的團隊 → 範本。位置換成相對於最左上那位。 */
export function snapshot(name: string, agents: AgentRow[]): TeamTemplate {
  const x0 = Math.min(...agents.map((a) => a.canvas_x));
  const y0 = Math.min(...agents.map((a) => a.canvas_y));
  return {
    name,
    agents: agents.map((a) => ({
      kind: a.kind,
      name: a.name,
      role: a.role,
      prompt_prefix: a.prompt_prefix,
      model: a.model ?? "",
      produces_spec: a.produces_spec,
      dx: a.canvas_x - x0,
      dy: a.canvas_y - y0,
    })),
  };
}

/**
 * 套用範本要建立的 Agent：全部用同一個程式庫，從 (x, y) 開始擺；專案裡已經有的
 * 單一角色（審核者、協調者、驗收者）跳過。
 */
export function toCreate(tpl: TeamTemplate, existing: AgentRow[], repoDir: string, x: number, y: number): NewAgentArgs[] {
  const have = new Set(existing.map((a) => a.kind));
  return tpl.agents
    .filter((a) => !(SINGLETONS.includes(a.kind) && have.has(a.kind)))
    .map((a) => ({
      kind: a.kind,
      name: a.name,
      role: a.role,
      prompt_prefix: a.prompt_prefix,
      project_dir: repoDir,
      canvas_x: x + a.dx,
      canvas_y: y + a.dy,
      produces_spec: a.produces_spec,
      model: a.model,
    }));
}
