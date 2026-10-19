import type { AgentKind } from "../../ipc/agents";
import { translations, type Translations } from "../../lib/i18n";
import type { RoleIconName } from "./RoleIcon";

/** 角色預設：名稱寫進 Agent 的 role，prefix 是派工時加在每張卡片前面的提示詞。 */
export interface RolePreset {
  name: string;
  /** 線條圖示（RoleIcon），跟種類圖示同一套風格。 */
  icon: RoleIconName | AgentKind;
  prefix: string;
}

/**
 * 內建角色。Coder 的清單沿用 Loop Studio 的預設角色（同一組 i18n 字串，
 * 改一邊兩邊都跟著變），再加上 Agent 協作特有的幾個（規格撰寫師、C/S 移植…）。
 * Reviewer／Orchestrator 的提示詞有工作流程上的硬性要求（不改檔、判定規則、
 * 不 push），所以不共用 Loop Studio 的描述。
 */
export function builtinRoles(kind: AgentKind, t: Translations): RolePreset[] {
  if (kind === "reviewer") {
    return [
      { name: t.ac_role_general_reviewer, icon: "check", prefix: t.ac_role_general_reviewer_desc },
      { name: t.ac_role_combined_reviewer, icon: "layers", prefix: t.ac_role_combined_reviewer_desc },
      { name: t.ls_preset_reviewer, icon: "reviewer", prefix: t.ac_role_code_reviewer_desc },
      { name: t.ls_preset_security, icon: "shield", prefix: t.ac_role_security_reviewer_desc },
    ];
  }
  if (kind === "verifier") {
    return [{ name: t.ac_role_verifier, icon: "verifier", prefix: t.ac_role_verifier_desc }];
  }
  if (kind === "orchestrator") {
    return [{ name: t.ac_role_integrator, icon: "orchestrator", prefix: t.ac_role_integrator_desc }];
  }
  return [
    { name: t.ls_preset_coder, icon: "coder", prefix: t.ls_preset_coder_desc },
    { name: t.ac_role_spec, icon: "spec", prefix: t.ac_role_spec_desc },
    { name: t.ac_role_migration, icon: "migrate", prefix: t.ac_role_migration_desc },
    { name: t.ls_preset_tester, icon: "test", prefix: t.ls_preset_tester_desc },
    { name: t.ac_role_bugfix, icon: "bug", prefix: t.ac_role_bugfix_desc },
    { name: t.ac_role_frontend, icon: "palette", prefix: t.ac_role_frontend_desc },
    { name: t.ls_preset_refactorer, icon: "wrench", prefix: t.ls_preset_refactorer_desc },
    { name: t.ls_preset_devops, icon: "gear", prefix: t.ls_preset_devops_desc },
    { name: t.ls_preset_docs, icon: "doc", prefix: t.ls_preset_docs_desc },
    { name: t.ls_preset_researcher, icon: "chart", prefix: t.ls_preset_researcher_desc },
    { name: t.ls_preset_architect, icon: "landmark", prefix: t.ls_preset_architect_desc },
  ];
}

/**
 * 工程師卡片標題的圖示：角色名稱對得上內建角色就用它的圖示，否則 null（沿用種類圖示）。
 * role 存的是挑選當下語系的名稱，所以兩個語系都比對，切換語系後才不會掉回預設圖示。
 * 只做工程師：其他種類的流程角色固定，用種類圖示辨識比較清楚。
 */
export function roleIcon(kind: AgentKind, role: string): RoleIconName | AgentKind | null {
  if (kind !== "coder") return null;
  const name = role.trim();
  if (!name) return null;
  for (const t of Object.values(translations) as Translations[]) {
    const hit = builtinRoles(kind, t).find((r) => r.name === name);
    if (hit) return hit.icon;
  }
  return null;
}
