import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import { McpMarketplaceTab } from "./McpMarketplaceTab";

vi.mock("../../lib/smithery", () => ({
  searchSmithery: vi.fn(),
  getSmitheryServer: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));

vi.mock("../../ipc/mcp", () => ({
  addMcpServer: vi.fn(() => Promise.resolve()),
}));

vi.mock("../../contexts/LocaleContext", () => ({
  useLocale: () => ({
    t: {
      mcp_marketplace_search_placeholder: "Search MCP servers...",
      mcp_marketplace_install: "Install",
      mcp_marketplace_installing: "⟳ Installing...",
      mcp_marketplace_installed_done: "✓ Installed",
      mcp_marketplace_failed: "✗ Failed, retry",
      mcp_marketplace_copy_cmd: "Copy Command",
      mcp_marketplace_no_connections: "Auto-install not supported",
      mcp_marketplace_network_error: "Cannot reach marketplace, check network",
      mcp_marketplace_node_missing: "Please install Node.js: https://nodejs.org",
      mcp_marketplace_python_missing: "Please install Python: https://python.org",
      mcp_marketplace_timeout: "Install timed out, run command manually",
      mcp_marketplace_done_msg: "✓ Install complete, added to installed list",
      mcp_marketplace_terminal_title: "Install Log",
      mcp_marketplace_no_results: "No servers found",
      mcp_marketplace_needs_config: "Requires API Key",
      mcp_marketplace_needs_config_btn: "Needs Setup",
      mcp_marketplace_config_title: "Fill in required configuration",
      mcp_marketplace_config_confirm: "Add Server",
      mcp_marketplace_config_no_schema: "No schema available",
      cancel: "Cancel",
    },
  }),
}));

import { searchSmithery, getSmitheryServer } from "../../lib/smithery";
import { invoke } from "@tauri-apps/api/core";

const mockSearch = searchSmithery as ReturnType<typeof vi.fn>;
const mockGetDetail = getSmitheryServer as ReturnType<typeof vi.fn>;
const mockInvoke = invoke as ReturnType<typeof vi.fn>;

beforeEach(() => {
  vi.clearAllMocks();
  mockInvoke.mockResolvedValue(undefined);
});

describe("McpMarketplaceTab", () => {
  it("renders search input", () => {
    render(<McpMarketplaceTab onInstalled={() => {}} />);
    expect(screen.getByRole("textbox")).toBeTruthy();
  });

  it("shows results after search", async () => {
    mockSearch.mockResolvedValueOnce([
      {
        qualifiedName: "@mcp/server-filesystem",
        displayName: "Filesystem",
        description: "Read/write files",
      },
    ]);

    render(<McpMarketplaceTab onInstalled={() => {}} />);
    const input = screen.getByRole("textbox");

    await act(async () => {
      fireEvent.change(input, { target: { value: "filesystem" } });
      await new Promise((r) => setTimeout(r, 600)); // debounce
    });

    await waitFor(() => {
      expect(screen.getByText("Filesystem")).toBeTruthy();
    });
  });

  it("shows network error when search throws", async () => {
    mockSearch.mockRejectedValueOnce(new Error("network error"));

    render(<McpMarketplaceTab onInstalled={() => {}} />);
    const input = screen.getByRole("textbox");

    await act(async () => {
      fireEvent.change(input, { target: { value: "test" } });
      await new Promise((r) => setTimeout(r, 600));
    });

    await waitFor(() => {
      expect(
        document.querySelector('[data-testid="network-error"]') ||
          screen.queryByText(/network|無法連線/i)
      ).toBeTruthy();
    });
  });

  it("shows install button for each result", async () => {
    mockSearch.mockResolvedValueOnce([
      {
        qualifiedName: "@mcp/server-git",
        displayName: "Git",
        description: "Git operations",
      },
    ]);

    render(<McpMarketplaceTab onInstalled={() => {}} />);
    const input = screen.getByRole("textbox");

    await act(async () => {
      fireEvent.change(input, { target: { value: "git" } });
      await new Promise((r) => setTimeout(r, 600));
    });

    await waitFor(() => {
      expect(screen.getByText("Install")).toBeTruthy();
    });
  });

  it("directly adds HTTP server without required config", async () => {
    mockSearch.mockResolvedValueOnce([
      {
        qualifiedName: "@mcp/server-http",
        displayName: "HTTP Server",
        description: "HTTP only",
      },
    ]);
    mockGetDetail.mockResolvedValueOnce({
      qualifiedName: "@mcp/server-http",
      displayName: "HTTP Server",
      description: "HTTP only",
      connections: [{ type: "http", url: "https://example.com" }],
    });

    render(<McpMarketplaceTab onInstalled={() => {}} />);
    const input = screen.getByRole("textbox");

    await act(async () => {
      fireEvent.change(input, { target: { value: "http" } });
      await new Promise((r) => setTimeout(r, 600));
    });

    await waitFor(() => screen.getByText("Install"));

    await act(async () => {
      fireEvent.click(screen.getByText("Install"));
    });

    await waitFor(() => {
      expect(screen.getByText("✓ Installed")).toBeTruthy();
    });
  });

  it("shows inline config form for HTTP server with required config fields", async () => {
    mockSearch.mockResolvedValueOnce([
      {
        qualifiedName: "@mcp/server-brave",
        displayName: "Brave Search",
        description: "Search with Brave",
      },
    ]);
    mockGetDetail.mockResolvedValueOnce({
      qualifiedName: "@mcp/server-brave",
      displayName: "Brave Search",
      description: "Search with Brave",
      connections: [{
        type: "http",
        deploymentUrl: "https://brave.run.tools",
        configSchema: {
          required: ["braveApiKey"],
          properties: {
            braveApiKey: { type: "string", title: "Brave API Key", description: "Your API key" },
          },
        },
      }],
    });

    render(<McpMarketplaceTab onInstalled={() => {}} />);
    const input = screen.getByRole("textbox");

    await act(async () => {
      fireEvent.change(input, { target: { value: "brave" } });
      await new Promise((r) => setTimeout(r, 600));
    });

    await waitFor(() => screen.getByText("Install"));

    await act(async () => {
      fireEvent.click(screen.getByText("Install"));
    });

    await waitFor(() => {
      expect(screen.getByText("Fill in required configuration")).toBeTruthy();
      expect(screen.getByText("Brave API Key")).toBeTruthy();
    });
  });

  it("shows no-connections state when server has no usable connections", async () => {
    mockSearch.mockResolvedValueOnce([
      {
        qualifiedName: "@mcp/server-none",
        displayName: "No Conn",
        description: "No connections",
      },
    ]);
    mockGetDetail.mockResolvedValueOnce({
      qualifiedName: "@mcp/server-none",
      displayName: "No Conn",
      description: "No connections",
      connections: [],
    });

    render(<McpMarketplaceTab onInstalled={() => {}} />);
    const input = screen.getByRole("textbox");

    await act(async () => {
      fireEvent.change(input, { target: { value: "http" } });
      await new Promise((r) => setTimeout(r, 600));
    });

    await waitFor(() => screen.getByText("Install"));

    await act(async () => {
      fireEvent.click(screen.getByText("Install"));
    });

    await waitFor(() => {
      expect(screen.getByText("Copy Command")).toBeTruthy();
    });
  });
});
