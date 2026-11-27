import { invoke } from "@tauri-apps/api/core";

/** server 上已分享的登入 session：誰、何時、幾個 cookie（cookie 本身不回傳）。 */
export interface ServerSystemSession {
  exported_by: string;
  exported_at: number;
  cookie_count: number;
}

/** server 上的網頁系統連線。成員拿不到 `allowed_hosts`。 */
export interface ServerSystem {
  id: string;
  name: string;
  start_url: string;
  mode: "read_only" | "approve_each" | "autonomous";
  enabled: boolean;
  allowed_hosts?: string[] | null;
  session?: ServerSystemSession | null;
}

/** 共用專案裡等管理員核准的系統請求。 */
export interface ServerSysApproval {
  id: string;
  system_id: string;
  system_name: string;
  card_id: string;
  agent_id: string | null;
  intent: string;
  method: string;
  url: string;
  declared_read: boolean;
  ts_ms: number;
}

export const listServerSystems = (): Promise<ServerSystem[]> => invoke("server_systems_list");
/** 把本機設定好的網頁系統（名稱、起始網址、白名單、權限模式）分享到 server（管理員限定）。 */
export const shareSystem = (systemId: string): Promise<ServerSystem> => invoke("server_system_share", { systemId });
/** 把這個系統目前登入的 session 分享給 server（管理員限定）；要先在它的瀏覽器視窗登入。回傳 server 保存的 cookie 數。 */
export const shareSystemSession = (systemId: string): Promise<number> => invoke("server_system_share_session", { systemId });
export const deleteServerSystem = (id: string): Promise<void> => invoke("server_system_delete", { id });
export const deleteServerSystemSession = (id: string): Promise<void> => invoke("server_system_session_delete", { id });
export const setAgentServerSystems = (agentId: string, ids: string[]): Promise<void> => invoke("agents_set_server_systems", { agentId, ids });
export const listServerSysApprovals = (projectId: string): Promise<ServerSysApproval[]> => invoke("server_sys_approvals", { projectId });
export const decideServerSys = (id: string, approve: boolean): Promise<void> => invoke("server_sys_decide", { id, approve });
