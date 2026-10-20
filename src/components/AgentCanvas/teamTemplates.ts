import type { AgentCli, AgentKind, AgentRow, NewAgentArgs } from "../../ipc/agents";
import type { FormationAgent } from "../../ipc/formations";
import type { Translations } from "../../lib/i18n";

/** 範本裡的一位 Agent：角色設定與相對位置，不含程式庫（套用時再選）。 */
export interface TemplateAgent {
  kind: AgentKind;
  name: string;
  role: string;
  prompt_prefix: string;
  model: string;
  /** 舊範本沒有 = Claude Code。 */
  cli?: AgentCli;
  produces_spec: boolean;
  dx: number;
  dy: number;
}

export interface TeamTemplate {
  name: string;
  agents: TemplateAgent[];
}

/** 內建範本：不能刪除，名稱與角色跟著語系。 */
export interface BuiltinTemplate extends TeamTemplate {
  id: "quick" | "bugfix" | "standard" | "spec_first" | "migration" | "inventory";
  description: string;
  builtin: true;
}

/** 範本裡的排列：工程師一欄往下排，其他角色依流程往右。 */
const COL = 320;
const ROW = 210;

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
      cli: a.cli ?? "claude",
      produces_spec: a.produces_spec,
      dx: a.canvas_x - x0,
      dy: a.canvas_y - y0,
    })),
  };
}

/**
 * 內建範本（五組常用團隊）。角色與提示詞取自內建角色（roles.ts 的 builtinRoles）；
 * 審核者、驗收者預設 sonnet，跟手動建立時一樣。有驗收者的一定有協調者：驗收是在
 * 協調者合併之後才開工，沒有協調者的話驗收者永遠等不到工作。
 */
export function builtinTemplates(t: Translations): BuiltinTemplate[] {
  const c = (role: string, prefix: string, produces_spec = false): TeamCoder => ({ role, prefix, produces_spec });
  const general = { role: t.ac_role_general_reviewer, prefix: t.ac_role_general_reviewer_desc };
  const combined = { role: t.ac_role_combined_reviewer, prefix: t.ac_role_combined_reviewer_desc };
  return [
    {
      id: "quick", builtin: true, name: t.ac_tpl_quick, description: t.ac_tpl_quick_desc,
      agents: assembleTeam(t, [c(t.ls_preset_coder, t.ls_preset_coder_desc)], general, false, false),
    },
    {
      id: "bugfix", builtin: true, name: t.ac_tpl_bugfix, description: t.ac_tpl_bugfix_desc,
      agents: assembleTeam(t, [c(t.ac_role_bugfix, t.ac_role_bugfix_desc), c(t.ls_preset_tester, t.ls_preset_tester_desc)], general, true, true),
    },
    {
      id: "standard", builtin: true, name: t.ac_tpl_standard, description: t.ac_tpl_standard_desc,
      agents: assembleTeam(t, [c(t.ac_role_frontend, t.ac_role_frontend_desc), c(t.ls_preset_coder, t.ls_preset_coder_desc)], general, true, true),
    },
    {
      id: "spec_first", builtin: true, name: t.ac_tpl_spec_first, description: t.ac_tpl_spec_first_desc,
      agents: assembleTeam(
        t,
        [c(t.ac_role_spec, t.ac_role_spec_desc, true), c(t.ls_preset_coder, t.ls_preset_coder_desc), c(t.ls_preset_coder, t.ls_preset_coder_desc)],
        general,
        true,
        true,
      ),
    },
    {
      id: "migration", builtin: true, name: t.ac_tpl_migration, description: t.ac_tpl_migration_desc,
      agents: assembleTeam(
        t,
        [c(t.ac_role_analyst, t.ac_role_analyst_desc, true), c(t.ac_role_migration, t.ac_role_migration_desc), c(t.ac_role_migration, t.ac_role_migration_desc)],
        combined,
        true,
        true,
      ),
    },
    {
      // 盤點：產出是文件不是程式，不需要驗收者；需求分析師決定盤點哪些模組，協調者照他的
      // 規格拆給研究工程師，最後把各人的文件合併起來。
      id: "inventory", builtin: true, name: t.ac_tpl_inventory, description: t.ac_tpl_inventory_desc,
      agents: assembleTeam(
        t,
        [
          c(t.ac_role_analyst, t.ac_role_analyst_desc, true),
          c(t.ls_preset_researcher, t.ac_tpl_inventory_researcher),
          c(t.ls_preset_researcher, t.ac_tpl_inventory_researcher),
          c(t.ls_preset_researcher, t.ac_tpl_inventory_researcher),
          c(t.ls_preset_docs, t.ac_tpl_inventory_synthesizer),
        ],
        { role: t.ac_tpl_inventory_reviewer_role, prefix: t.ac_tpl_inventory_reviewer },
        true,
        false,
      ),
    },
  ];
}

/** 團隊裡的一位工程師：角色名稱、提示詞、是否產出規格。 */
export interface TeamCoder {
  role: string;
  prefix: string;
  produces_spec: boolean;
}

/**
 * 依工程師清單與流程角色組出範本的 Agent（內建範本與「依專案簡介推薦」共用）：工程師一欄往下排，
 * 流程角色在工程師那一欄的中間高度依序往右；有產出規格的再改成規格欄在最左（見 specColumnFirst）。
 * 審核者、驗收者預設 sonnet，跟手動建立時一樣。
 */
export function assembleTeam(
  t: Translations,
  coders: TeamCoder[],
  reviewer: { role: string; prefix: string },
  withOrch: boolean,
  withVerifier: boolean,
): TemplateAgent[] {
  const mid = ((coders.length - 1) * ROW) / 2;
  const out: TemplateAgent[] = coders.map((c, i) => ({
    kind: "coder", name: `${t.ac_coder}${i + 1}`, role: c.role, prompt_prefix: c.prefix, model: "", produces_spec: c.produces_spec, dx: 0, dy: i * ROW,
  }));
  out.push({ kind: "reviewer", name: t.ac_reviewer, role: reviewer.role, prompt_prefix: reviewer.prefix, model: "sonnet", produces_spec: false, dx: COL, dy: mid });
  if (withOrch) {
    out.push({ kind: "orchestrator", name: t.ac_orchestrator, role: t.ac_role_integrator, prompt_prefix: t.ac_role_integrator_desc, model: "", produces_spec: false, dx: COL * 2, dy: mid });
  }
  if (withVerifier) {
    out.push({ kind: "verifier", name: t.ac_verifier, role: t.ac_role_verifier, prompt_prefix: t.ac_role_verifier_desc, model: "sonnet", produces_spec: false, dx: COL * 3, dy: mid });
  }
  return specColumnFirst(out);
}

/**
 * 跟桌布的「自動排列」一致：產出規格的工程師在最左一欄（派工線從他出發），其他工程師
 * 在下一欄，流程角色在其他工程師那一欄的中間高度、依序往右。沒有產出規格的不動。
 */
function specColumnFirst(agents: TemplateAgent[]): TemplateAgent[] {
  const specs = agents.filter((a) => a.kind === "coder" && a.produces_spec);
  if (specs.length === 0) return agents;
  const others = agents.filter((a) => a.kind === "coder" && !a.produces_spec);
  const mid = ((Math.max(others.length, 1) - 1) * ROW) / 2;
  const specTop = mid - ((specs.length - 1) * ROW) / 2;
  return agents.map((a) => {
    if (a.kind !== "coder") return { ...a, dx: a.dx + COL, dy: mid };
    if (a.produces_spec) return { ...a, dx: 0, dy: specTop + specs.indexOf(a) * ROW };
    return { ...a, dx: COL, dy: others.indexOf(a) * ROW };
  });
}

/** 名稱已經有人用時往後編號：「工程師1」→「工程師2」…（結尾不是數字的就加上「2」「3」…）。 */
function uniqueName(name: string, taken: Set<string>): string {
  if (!taken.has(name)) return name;
  const m = /^(.*?)(\d+)$/.exec(name);
  const base = m ? m[1] : name;
  let n = m ? Number(m[2]) + 1 : 2;
  while (taken.has(`${base}${n}`)) n++;
  return `${base}${n}`;
}

/**
 * 套用範本要建立的 Agent：全部用同一個程式庫，從 (x, y) 開始擺；專案裡已經有的
 * 單一角色（審核者、協調者、驗收者）跳過。
 */
export function toCreate(tpl: TeamTemplate, existing: AgentRow[], repoDir: string, x: number, y: number): NewAgentArgs[] {
  const have = new Set(existing.map((a) => a.kind));
  // 名稱不能跟桌布上已有的、或這次已經排好的撞名（第二次套用同一個範本時最常見）。
  const taken = new Set(existing.map((a) => a.name));
  return tpl.agents
    .filter((a) => !(SINGLETONS.includes(a.kind) && have.has(a.kind)))
    .map((a) => {
      const name = uniqueName(a.name, taken);
      taken.add(name);
      return { ...a, name };
    })
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
      cli: a.cli ?? "claude",
    }));
}

/** 範本 → 新編制的 Agent：從 (x, y) 開始擺。新編制是空桌布，單一角色不必跳過、名稱不會撞。 */
export function toFormationAgents(tpl: TeamTemplate, x: number, y: number): FormationAgent[] {
  return tpl.agents.map((a) => ({
    kind: a.kind,
    name: a.name,
    role: a.role,
    prompt_prefix: a.prompt_prefix,
    model: a.model,
    cli: a.cli ?? "claude",
    produces_spec: a.produces_spec,
    dx: x + a.dx,
    dy: y + a.dy,
  }));
}

/** 範本組成的一種角色：同一個角色的人合併成一筆（研究工程師 ×3）。 */
export interface CompositionPart {
  kind: AgentKind;
  label: string;
  count: number;
  produces_spec: boolean;
}

/**
 * 範本的組成，依工作流程排序：產出規格的工程師 → 其他工程師 → 審核者 → 協調者 → 驗收者
 * （跟桌布的自動排列同一個順序）；同一個順序裡照範本出現的先後。
 */
export function composition(tpl: TeamTemplate): CompositionPart[] {
  const rank = (a: { kind: AgentKind; produces_spec: boolean }) =>
    a.kind === "coder" ? (a.produces_spec ? 0 : 1) : { reviewer: 2, orchestrator: 3, verifier: 4 }[a.kind];
  const parts: CompositionPart[] = [];
  for (const a of tpl.agents) {
    const label = a.role || a.name;
    const same = parts.find((p) => p.kind === a.kind && p.label === label && p.produces_spec === a.produces_spec);
    if (same) same.count++;
    else parts.push({ kind: a.kind, label, count: 1, produces_spec: a.produces_spec });
  }
  return parts.map((p, i) => ({ p, i })).sort((x, y) => rank(x.p) - rank(y.p) || x.i - y.i).map(({ p }) => p);
}
