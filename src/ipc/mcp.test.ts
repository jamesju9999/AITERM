// src/ipc/mcp.test.ts
import { describe, it, expect } from "vitest";
import type { McpServerInfo, McpToolInfo } from "./mcp";

describe("McpServerInfo shape", () => {
  it("accepts a connected server object", () => {
    const info: McpServerInfo = {
      id: "fs",
      name: "Filesystem",
      enabled: true,
      transport: "stdio",
      command: "npx",
      args: [],
      url: null,
      status: "connected",
      tool_count: 5,
      error_message: null,
      env_keys: ["API_KEY"],
    };
    expect(info.status).toBe("connected");
    expect(info.tool_count).toBe(5);
  });

  it("accepts a disabled server object", () => {
    const info: McpServerInfo = {
      id: "search",
      name: "Search",
      enabled: false,
      transport: "http",
      command: null,
      args: [],
      url: "http://localhost:3000",
      status: "disabled",
      tool_count: 0,
      error_message: null,
      env_keys: [],
    };
    expect(info.status).toBe("disabled");
  });
});

describe("McpToolInfo shape", () => {
  it("has encoded name", () => {
    const tool: McpToolInfo = {
      server_id: "fs",
      server_name: "Filesystem",
      name: "fs__read_file",
      description: "Read a file",
    };
    expect(tool.name).toContain("__");
  });
});
