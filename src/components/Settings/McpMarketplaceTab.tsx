import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { addMcpServer } from "../../ipc/mcp";
import { useLocale } from "../../contexts/LocaleContext";
import { type NpmMcpServer, PAGE_SIZE } from "../../lib/npmRegistry";
import { openUrl } from "../../ipc/shell";

type InstallStatus = "idle" | "running" | "success" | "error";

interface ServerInstallState {
  status: InstallStatus;
}

interface Props {
  onInstalled: () => void;
}

function formatDownloads(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

export function McpMarketplaceTab({ onInstalled }: Props) {
  const { t } = useLocale();
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<NpmMcpServer[]>([]);
  const [total, setTotal] = useState(0);
  const [from, setFrom] = useState(0);
  const [isSearching, setIsSearching] = useState(false);
  const [networkError, setNetworkError] = useState(false);
  const [installStates, setInstallStates] = useState<Record<string, ServerInstallState>>({});
  const [sortByDownloads, setSortByDownloads] = useState(false);

  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const doSearch = async (q: string, offset: number) => {
    setIsSearching(true);
    setNetworkError(false);
    setFrom(offset);
    try {
      const jsonStr = await invoke("npm_mcp_search", { query: q, offset }) as string;
      if (!mountedRef.current) return;
      const raw = JSON.parse(jsonStr);
      const mapped: NpmMcpServer[] = (raw.results || []).map((r: any) => ({
        qualifiedName: r.qualified_name ?? "",
        displayName: r.display_name ?? "",
        description: r.description ?? "",
        homepage: r.homepage ?? undefined,
        npxCommand: r.npx_command ?? undefined,
        weeklyDownloads: r.weekly_downloads ?? 0,
      }));
      setResults(mapped);
      setTotal(raw.total ?? 0);
    } catch (err) {
      console.error("[McpMarketplaceTab] search error:", err);
      if (!mountedRef.current) return;
      setNetworkError(true);
      setResults([]);
    } finally {
      if (mountedRef.current) setIsSearching(false);
    }
  };

  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    if (!query.trim()) {
      setResults([]);
      setTotal(0);
      setFrom(0);
      setNetworkError(false);
      return;
    }
    debounceRef.current = setTimeout(() => {
      doSearch(query, 0);
    }, 500);
    return () => { if (debounceRef.current) clearTimeout(debounceRef.current); };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query]);

  function getInstallState(pkg: string): ServerInstallState {
    return installStates[pkg] ?? { status: "idle" };
  }

  function setServerState(pkg: string, status: InstallStatus) {
    setInstallStates(prev => ({
      ...prev,
      [pkg]: { status },
    }));
  }

  async function handleInstall(server: NpmMcpServer) {
    const pkg = server.qualifiedName;
    const state = getInstallState(pkg);
    if (state.status === "success" || state.status === "running") return;

    setServerState(pkg, "running");

    try {
      await addMcpServer({
        name: server.displayName || pkg,
        enabled: true,
        transport: "stdio" as const,
        command: "npx",
        args: ["-y", pkg],
        env: {},
      });
      if (!mountedRef.current) return;
      setServerState(pkg, "success");
      setTimeout(() => { onInstalled(); }, 1500);
    } catch {
      if (!mountedRef.current) return;
      setServerState(pkg, "error");
    }
  }

  function getButtonLabel(status: InstallStatus): string {
    switch (status) {
      case "running": return t.mcp_marketplace_installing;
      case "success": return t.mcp_marketplace_installed_done;
      case "error": return t.mcp_marketplace_failed;
      default: return t.mcp_marketplace_install;
    }
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 12, padding: "12px 0" }}>
      <input
        type="text"
        role="textbox"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder={t.mcp_marketplace_search_placeholder}
        className="mcp-marketplace-search"
      />

      {networkError && (
        <div data-testid="network-error" style={{ color: "#f87171", fontSize: 12 }}>
          {t.mcp_marketplace_network_error}
        </div>
      )}

      {isSearching && <div style={{ color: "#555", fontSize: 12 }}>...</div>}

      {!isSearching && query.trim() && results.length === 0 && !networkError && (
        <div className="mcp-marketplace-empty">{t.mcp_marketplace_no_results}</div>
      )}

      {results.length > 0 && (
        <div style={{ display: "flex", justifyContent: "flex-end" }}>
          <button
            onClick={() => setSortByDownloads(prev => !prev)}
            style={{
              background: sortByDownloads ? "#1e3a5f" : "none",
              border: `1px solid ${sortByDownloads ? "#3b82f6" : "#2a2a2a"}`,
              borderRadius: 4,
              color: sortByDownloads ? "#93c5fd" : "#666",
              cursor: "pointer",
              fontSize: 11,
              padding: "3px 8px",
            }}
          >
            ↓ {t.mcp_marketplace_sort_downloads}
          </button>
        </div>
      )}

      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        {(sortByDownloads ? [...results].sort((a, b) => b.weeklyDownloads - a.weeklyDownloads) : results).map((server) => {
          const state = getInstallState(server.qualifiedName);
          const isDisabled = state.status === "running" || state.status === "success";

          return (
            <div
              key={server.qualifiedName}
              style={{
                background: "#1a1a1a",
                border: "1px solid #2a2a2a",
                borderRadius: 6,
                padding: "10px 12px",
                display: "flex",
                alignItems: "flex-start",
                gap: 12,
              }}
            >
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ color: "#eee", fontSize: 13, fontWeight: 500 }}>
                  {server.displayName}
                </div>
                <div style={{
                  color: "#666", fontSize: 11, marginTop: 2,
                  overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
                }}>
                  {server.description}
                </div>
                <div style={{ color: "#555", fontSize: 11, marginTop: 3 }}>
                  ↓ {formatDownloads(server.weeklyDownloads)}/週
                </div>
              </div>
              <div style={{ display: "flex", flexDirection: "column", gap: 4, flexShrink: 0 }}>
                <button
                  onClick={() => handleInstall(server)}
                  disabled={isDisabled}
                  style={{
                    background: state.status === "success" ? "#166534" : "#2a2a2a",
                    border: "1px solid #3a3a3a",
                    borderRadius: 4,
                    color: state.status === "error" ? "#f87171" : "#ccc",
                    cursor: isDisabled ? "default" : "pointer",
                    fontSize: 12,
                    padding: "4px 10px",
                    whiteSpace: "nowrap",
                  }}
                >
                  {getButtonLabel(state.status)}
                </button>
                <button
                  onClick={() => openUrl(server.homepage ?? `https://www.npmjs.com/package/${server.qualifiedName}`).catch(console.error)}
                  style={{
                    background: "none",
                    border: "1px solid #2a2a2a",
                    borderRadius: 4,
                    color: "#666",
                    cursor: "pointer",
                    fontSize: 11,
                    padding: "3px 10px",
                    whiteSpace: "nowrap",
                  }}
                >
                  介紹
                </button>
              </div>
            </div>
          );
        })}
      </div>

      {/* Pagination */}
      {total > PAGE_SIZE && (
        <div style={{ display: "flex", gap: 8, justifyContent: "center", marginTop: 4 }}>
          <button
            className="mcp-btn-sm"
            disabled={from === 0 || isSearching}
            onClick={() => doSearch(query, Math.max(0, from - PAGE_SIZE))}
          >
            ← 上一頁
          </button>
          <span style={{ fontSize: 12, color: "#666", alignSelf: "center" }}>
            {from + 1}–{Math.min(from + PAGE_SIZE, total)} / {total}
          </span>
          <button
            className="mcp-btn-sm"
            disabled={from + PAGE_SIZE >= total || isSearching}
            onClick={() => doSearch(query, from + PAGE_SIZE)}
          >
            下一頁 →
          </button>
        </div>
      )}

    </div>
  );
}
