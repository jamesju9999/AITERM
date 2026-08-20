import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface McpToolServerStatus {
  running: boolean;
  port: number | null;
  /** 已產生的 token，供「複製手動命令」使用。未啟動時為 null。 */
  token: string | null;
  /** 啟動失敗的原因（例如埠被占用）。這是使用者要處理的狀態，不是例外。 */
  error: string | null;
}

export function mcpToolServerStatus(): Promise<McpToolServerStatus> {
  return invoke<McpToolServerStatus>("mcp_tool_server_status");
}

/** 依目前 config 啟動或停止 server。設定存檔後呼叫。 */
export function mcpToolServerApply(): Promise<McpToolServerStatus> {
  return invoke<McpToolServerStatus>("mcp_tool_server_apply");
}

export interface McpToolServerConfig {
  enabled: boolean;
  port: number;
  coordination_enabled: boolean;
}

/** 存下設定並立刻套用（啟動或停止 server）。 */
export function mcpToolServerSetConfig(value: McpToolServerConfig): Promise<McpToolServerStatus> {
  return invoke<McpToolServerStatus>("mcp_tool_server_set_config", { value });
}

export interface CoordinationTabSpawnedPayload {
  session_id: string;
  command: string | null;
}

/** Fired when an MCP coordination tool call spawns a new terminal tab. */
export function onCoordinationTabSpawned(
  cb: (payload: CoordinationTabSpawnedPayload) => void,
): Promise<UnlistenFn> {
  return listen<CoordinationTabSpawnedPayload>("mcp-coordination-tab-spawned", (e) => cb(e.payload));
}
