import { describe, it, expect, vi, beforeEach } from "vitest";
import { searchSmithery, getSmitheryServer } from "./smithery";

const mockFetch = vi.fn();
vi.stubGlobal("fetch", mockFetch);

beforeEach(() => {
  mockFetch.mockReset();
});

describe("searchSmithery", () => {
  it("returns servers from API response", async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        servers: [
          {
            qualifiedName: "@mcp/server-filesystem",
            displayName: "Filesystem",
            description: "Read/write local filesystem",
            homepage: "https://example.com",
          },
        ],
        pagination: { currentPage: 1, pageSize: 20, totalCount: 1 },
      }),
    });

    const results = await searchSmithery("filesystem");
    expect(results).toHaveLength(1);
    expect(results[0].qualifiedName).toBe("@mcp/server-filesystem");
    expect(mockFetch).toHaveBeenCalledWith(
      "https://registry.smithery.ai/servers?q=filesystem&pageSize=20"
    );
  });

  it("throws on network error", async () => {
    mockFetch.mockResolvedValueOnce({ ok: false, status: 500 });
    await expect(searchSmithery("test")).rejects.toThrow("Smithery API error: 500");
  });
});

describe("getSmitheryServer", () => {
  it("returns server detail with connections", async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        qualifiedName: "@mcp/server-filesystem",
        displayName: "Filesystem",
        description: "Read/write local filesystem",
        connections: [
          {
            type: "stdio",
            stdioFunction: {
              command: "npx",
              args: ["-y", "@modelcontextprotocol/server-filesystem"],
              env: {},
            },
          },
        ],
      }),
    });

    const detail = await getSmitheryServer("@mcp/server-filesystem");
    expect(detail.connections).toHaveLength(1);
    expect(detail.connections[0].type).toBe("stdio");
    expect(detail.connections[0].stdioFunction?.command).toBe("npx");
  });
});
