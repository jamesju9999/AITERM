import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { McpToolServerPage } from "./McpToolServerPage";

vi.mock("../../ipc/mcpToolServer", () => ({
  mcpToolServerStatus: vi.fn(),
  mcpToolServerSetConfig: vi.fn(),
}));
vi.mock("../../ipc/config", () => ({ getConfig: vi.fn() }));

import { mcpToolServerStatus, mcpToolServerSetConfig } from "../../ipc/mcpToolServer";
import type { McpToolServerStatus } from "../../ipc/mcpToolServer";
import { getConfig } from "../../ipc/config";
import type { AppConfig } from "../../ipc/config";

const BASE_CONFIG: AppConfig = {
  default_provider: null,
  providers: [],
  execution_mode: "graded",
  submit_shortcut: "enter",
  doc_convert_engine: "auto",
  onboarding_done: true,
  max_agent_steps: 5,
  default_tab: "terminal",
  enterprise_server_url: null,
  enterprise_device_id: null,
  enterprise_policy: null,
  claude_bridge: { enabled: false, port: 8317, default_on_new_tab: false, opus: null, sonnet: null, haiku: null },
  mcp_tool_server: { enabled: false, port: 8318, coordination_enabled: false },
};

const STOPPED_STATUS: McpToolServerStatus = { running: false, port: null, token: null, error: null };
const RUNNING_STATUS: McpToolServerStatus = { running: true, port: 8318, token: "abc123", error: null };

beforeEach(() => {
  vi.mocked(getConfig).mockResolvedValue(BASE_CONFIG);
  vi.mocked(mcpToolServerStatus).mockResolvedValue(STOPPED_STATUS);
  vi.mocked(mcpToolServerSetConfig).mockResolvedValue(RUNNING_STATUS);
});

describe("McpToolServerPage", () => {
  it("loads the saved config and shows the stopped status", async () => {
    render(<McpToolServerPage />);
    await waitFor(() => expect(screen.getAllByRole("checkbox")[0]).not.toBeChecked());
  });

  it("enabling and saving calls mcpToolServerSetConfig with enabled: true", async () => {
    const user = userEvent.setup();
    render(<McpToolServerPage />);
    await waitFor(() => screen.getAllByRole("checkbox"));

    await user.click(screen.getAllByRole("checkbox")[0]);
    await user.click(screen.getByRole("button", { name: /save|儲存/i }));

    await waitFor(() => {
      expect(mcpToolServerSetConfig).toHaveBeenCalledWith(
        expect.objectContaining({ enabled: true, port: 8318 }),
      );
    });
  });

  it("enabling coordination tools calls mcpToolServerSetConfig with coordination_enabled: true", async () => {
    const user = userEvent.setup();
    render(<McpToolServerPage />);
    await waitFor(() => screen.getAllByRole("checkbox"));

    const checkboxes = screen.getAllByRole("checkbox");
    // Second checkbox is the coordination toggle (first is the server enable toggle).
    await user.click(checkboxes[1]);
    await user.click(screen.getByRole("button", { name: /save|儲存/i }));

    await waitFor(() => {
      expect(mcpToolServerSetConfig).toHaveBeenCalledWith(
        expect.objectContaining({ coordination_enabled: true }),
      );
    });
  });

  it("shows the running port and a copyable claude mcp add command once started", async () => {
    vi.mocked(mcpToolServerStatus).mockResolvedValue(RUNNING_STATUS);
    const { container } = render(<McpToolServerPage />);
    // Scoped to the specific command block rather than screen.getByText(/8318/):
    // the status line above it also renders the port ("· :8318"), so a
    // page-wide text query would match two elements and throw on ambiguity.
    await waitFor(() => {
      const command = container.querySelector(".mcp-tool-server-command");
      expect(command).not.toBeNull();
      expect(command!.textContent).toContain("8318");
      expect(command!.textContent).toContain("abc123");
    });
  });

  it("shows the error message instead of throwing when start fails", async () => {
    vi.mocked(mcpToolServerStatus).mockResolvedValue({
      running: false, port: null, token: null, error: "無法綁定 127.0.0.1:8318（埠已被占用）",
    });
    render(<McpToolServerPage />);
    await waitFor(() => {
      expect(screen.getByText(/埠已被占用|already in use|無法綁定/i)).toBeInTheDocument();
    });
  });
});
