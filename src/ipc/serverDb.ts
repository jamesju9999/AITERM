import { invoke } from "@tauri-apps/api/core";

import type { DbSkill } from "./agents";

/** server 上分享的資料庫連線。成員只有 id／名稱／類型／資料庫；主機與帳號只有管理員拿得到。 */
export interface ServerDbConn {
  id: string;
  name: string;
  db_type: string;
  database: string;
  host?: string;
  port?: number;
  username?: string;
}

/** server 上的一筆資料庫查詢記錄；待核准清單也是這個形狀。 */
export interface ServerDbQuery {
  id: number;
  card_id: string;
  agent_id: string | null;
  connection_id: string;
  connection_name: string;
  sql: string;
  decision: "allowed" | "pending" | "approved" | "denied";
  started_at: number;
  finished_at: number | null;
  rows: number | null;
  error: string | null;
  decided_by: string | null;
}

export const listServerDbConns = (): Promise<ServerDbConn[]> => invoke("server_db_conns_list");
/** 把本機設定好的連線分享到 server（管理員限定）；帳密由 server 加密保存。 */
export const shareDbConnection = (connectionId: string): Promise<ServerDbConn> => invoke("server_db_conn_share", { connectionId });
export const deleteServerDbConn = (id: string): Promise<void> => invoke("server_db_conn_delete", { id });
/** 讓 server 實際連一次（管理員限定）；失敗會丟出原因。 */
export const testServerDbConn = (id: string): Promise<void> => invoke("server_db_conn_test", { id });
/** 設定（null＝移除）共用專案裡一位 Agent 的資料庫技能（管理員限定）。 */
export const setAgentServerDbSkill = (agentId: string, skill: DbSkill | null): Promise<void> => invoke("agents_set_server_db_skill", { agentId, skill });
export const listServerDbApprovals = (projectId: string): Promise<ServerDbQuery[]> => invoke("server_db_approvals", { projectId });
export const listServerDbQueries = (projectId: string): Promise<ServerDbQuery[]> => invoke("server_db_queries", { projectId });
export const decideServerDb = (id: number, approve: boolean): Promise<void> => invoke("server_db_decide", { id, approve });
