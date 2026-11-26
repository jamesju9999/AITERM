import { invoke } from "@tauri-apps/api/core";

/** server 技能庫裡的一個技能（整台 server 共用，管理員上傳）。 */
export interface ServerSkill {
  id: string;
  name: string;
  description: string;
  files: number;
  bytes: number;
  uploaded_by: string;
  updated_at: number;
}

export const listServerSkills = (): Promise<ServerSkill[]> => invoke("server_skills_list");
/** 把本機一個技能資料夾（含 SKILL.md）上傳到 server（管理員限定；同名取代）。 */
export const uploadServerSkill = (path: string): Promise<ServerSkill> => invoke("server_skills_upload", { path });
export const deleteServerSkill = (id: string): Promise<void> => invoke("server_skills_delete", { id });
/** 設定共用專案裡一位 Agent 的技能（server 技能 id，整份取代；管理員限定）。 */
export const setAgentServerSkills = (agentId: string, ids: string[]): Promise<void> =>
  invoke("agents_set_server_skills", { agentId, ids });
