import type { AgentKind } from "../../ipc/agents";
import type { Translations } from "../../lib/i18n";

/** 角色預設：名稱寫進 Agent 的 role，prefix 是派工時加在每張卡片前面的提示詞。 */
export interface RolePreset {
  name: string;
  emoji: string;
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
      { name: t.ac_role_general_reviewer, emoji: "✅", prefix: t.ac_role_general_reviewer_desc },
      { name: t.ls_preset_reviewer, emoji: "🔍", prefix: t.ac_role_code_reviewer_desc },
      { name: t.ls_preset_security, emoji: "🛡️", prefix: t.ac_role_security_reviewer_desc },
    ];
  }
  if (kind === "orchestrator") {
    return [{ name: t.ac_role_integrator, emoji: "⑂", prefix: t.ac_role_integrator_desc }];
  }
  return [
    { name: t.ls_preset_coder, emoji: "💻", prefix: t.ls_preset_coder_desc },
    { name: t.ac_role_spec, emoji: "📐", prefix: t.ac_role_spec_desc },
    { name: t.ac_role_migration, emoji: "🔁", prefix: t.ac_role_migration_desc },
    { name: t.ls_preset_tester, emoji: "🧪", prefix: t.ls_preset_tester_desc },
    { name: t.ac_role_bugfix, emoji: "🐞", prefix: t.ac_role_bugfix_desc },
    { name: t.ac_role_frontend, emoji: "🎨", prefix: t.ac_role_frontend_desc },
    { name: t.ls_preset_refactorer, emoji: "🔧", prefix: t.ls_preset_refactorer_desc },
    { name: t.ls_preset_devops, emoji: "⚙️", prefix: t.ls_preset_devops_desc },
    { name: t.ls_preset_docs, emoji: "📝", prefix: t.ls_preset_docs_desc },
    { name: t.ls_preset_researcher, emoji: "📊", prefix: t.ls_preset_researcher_desc },
    { name: t.ls_preset_architect, emoji: "🏛️", prefix: t.ls_preset_architect_desc },
  ];
}
