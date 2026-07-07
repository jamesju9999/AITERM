import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import { McpMarketplaceTab } from "./McpMarketplaceTab";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));

vi.mock("../../ipc/mcp", () => ({
  addMcpServer: vi.fn(() => Promise.resolve()),
}));

vi.mock("../../ipc/shell", () => ({
  openUrl: vi.fn(() => Promise.resolve()),
}));

vi.mock("../../contexts/LocaleContext", () => ({
  useLocale: () => ({
    t: {
      mcp_marketplace_search_placeholder: "Search MCP servers...",
      mcp_marketplace_install: "Install",
      mcp_marketplace_installing: "⟳ Installing...",
      mcp_marketplace_installed_done: "✓ Installed",
      mcp_marketplace_failed: "✗ Failed, retry",
      mcp_marketplace_network_error: "Cannot reach marketplace, check network",
      mcp_marketplace_no_results: "No servers found",
      mcp_marketplace_sort_downloads: "Sort by downloads",
    },
  }),
}));

import { invoke } from "@tauri-apps/api/core";

const mockInvoke = invoke as ReturnType<typeof vi.fn>;

function mockSearchOnce(results: Array<{ qualified_name: string; display_name: string; description: string }>, total?: number) {
  mockInvoke.mockImplementationOnce((cmd: string) => {
    if (cmd === "npm_mcp_search") {
      return Promise.resolve(JSON.stringify({ results, total: total ?? results.length }));
    }
    return Promise.resolve(undefined);
  });
}

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
    mockSearchOnce([
      { qualified_name: "@mcp/server-filesystem", display_name: "Filesystem", description: "Read/write files" },
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
    mockInvoke.mockImplementationOnce((cmd: string) => {
      if (cmd === "npm_mcp_search") return Promise.reject(new Error("network error"));
      return Promise.resolve(undefined);
    });

    render(<McpMarketplaceTab onInstalled={() => {}} />);
    const input = screen.getByRole("textbox");

    await act(async () => {
      fireEvent.change(input, { target: { value: "test" } });
      await new Promise((r) => setTimeout(r, 600));
    });

    await waitFor(() => {
      expect(document.querySelector('[data-testid="network-error"]')).toBeTruthy();
    });
  });

  it("shows install button for each result", async () => {
    mockSearchOnce([
      { qualified_name: "@mcp/server-git", display_name: "Git", description: "Git operations" },
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

  it("installs a server directly with npx on Install click", async () => {
    mockSearchOnce([
      { qualified_name: "@mcp/server-http", display_name: "HTTP Server", description: "HTTP only" },
    ]);

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
});
