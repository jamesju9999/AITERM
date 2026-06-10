// src/ipc/mcp.ts
import { invoke } from "@tauri-apps/api/core";

export type McpTransport = "stdio" | "http" | "sse";

export interface McpServerInput {
  id?: string;
  name: string;
  enabled: boolean;
  transport: McpTransport;
  command?: string | null;
  args: string[];
  env: Record<string, string>;
  url?: string | null;
}

export interface McpServerInfo {
  id: string;
  name: string;
  enabled: boolean;
  transport: McpTransport;
  command: string | null;
  args: string[];
  url: string | null;
  status: "connecting" | "connected" | "error" | "disabled";
  tool_count: number;
  error_message: string | null;
}

export interface McpToolInfo {
  server_id: string;
  server_name: string;
  name: string;        // encoded: "server_id__tool_name"
  description: string;
}

export interface McpToolResult {
  content: string;
  is_error: boolean;
}

export const listMcpServers = (): Promise<McpServerInfo[]> =>
  invoke("list_mcp_servers");

export const addMcpServer = (input: McpServerInput): Promise<void> =>
  invoke("add_mcp_server", { input });

export const updateMcpServer = (input: McpServerInput): Promise<void> =>
  invoke("update_mcp_server", { input });

export const removeMcpServer = (id: string): Promise<void> =>
  invoke("remove_mcp_server", { id });

export const getMcpTools = (): Promise<McpToolInfo[]> =>
  invoke("get_mcp_tools");

export const executeMcpTool = (
  encodedName: string,
  args: unknown,
): Promise<McpToolResult> =>
  invoke("execute_mcp_tool", { encodedName, args });

export const importClaudeDesktopMcp = (): Promise<McpServerInput[]> =>
  invoke("import_claude_desktop_mcp");

export const setMcpEnabled = (enabled: boolean): Promise<void> =>
  invoke("set_mcp_enabled", { enabled });
