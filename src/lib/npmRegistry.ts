// npm registry MCP marketplace data source.
// Search is proxied through the Rust backend (reqwest) to avoid WebView fetch restrictions.

import { invoke } from "@tauri-apps/api/core";

export interface NpmSearchPage {
  results: NpmMcpServer[];
  total: number;
}

export interface NpmMcpServer {
  qualifiedName: string;
  displayName: string;
  description: string;
  homepage?: string;
  /** `npx -y <package>` install command; present when package is likely a runnable server */
  npxCommand?: string;
  weeklyDownloads: number;
}

export const PAGE_SIZE = 20;

export async function searchNpmMcp(query: string, from = 0): Promise<NpmSearchPage> {
  const raw = await invoke("npm_mcp_search", { query, offset: from });
  console.log("[npmRegistry] raw:", JSON.stringify(raw).slice(0, 300));
  const page = raw as { results: RustNpmResult[]; total: number };
  if (!page || !Array.isArray(page.results)) {
    console.error("[npmRegistry] unexpected response shape:", page);
    throw new Error(`Unexpected response: ${JSON.stringify(page)}`);
  }

  return {
    results: page.results.map((r) => ({
      qualifiedName: r.qualified_name,
      displayName: r.display_name,
      description: r.description,
      homepage: r.homepage ?? undefined,
      npxCommand: r.npx_command ?? undefined,
      weeklyDownloads: r.weekly_downloads,
    })),
    total: page.total,
  };
}

interface RustNpmResult {
  qualified_name: string;
  display_name: string;
  description: string;
  homepage: string | null;
  npx_command: string | null;
  weekly_downloads: number;
}
