import { useState, useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { addMcpServer } from "../../ipc/mcp";
import { useLocale } from "../../contexts/LocaleContext";
import { searchNpmMcp, type NpmMcpServer, PAGE_SIZE } from "../../lib/npmRegistry";
import { McpInstallTerminal, type InstallLogLine } from "./McpInstallTerminal";

type InstallStatus = "idle" | "running" | "success" | "error";

interface ServerInstallState {
  status: InstallStatus;
  logs: InstallLogLine[];
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
  const [activeTerminal, setActiveTerminal] = useState<string | null>(null);

  const mountedRef = useRef(true);
  useEffect(() => () => { mountedRef.current = false; }, []);

  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const doSearch = async (q: string, offset: number) => {
    setIsSearching(true);
    setNetworkError(false);
    try {
      const res = await searchNpmMcp(q, offset);
      if (!mountedRef.current) return;
      setResults(res.results);
      setTotal(res.total);
      setFrom(offset);
    } catch {
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
    debounceRef.current = setTimeout(() => doSearch(query, 0), 500);
    return () => { if (debounceRef.current) clearTimeout(debounceRef.current); };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query]);

  function getInstallState(pkg: string): ServerInstallState {
    return installStates[pkg] ?? { status: "idle", logs: [] };
  }

  function setServerState(pkg: string, update: Partial<ServerInstallState>) {
    setInstallStates(prev => ({
      ...prev,
      [pkg]: { ...(prev[pkg] ?? { status: "idle", logs: [] }), ...update },
    }));
  }

  async function handleInstall(server: NpmMcpServer) {
    const pkg = server.qualifiedName;
    const state = getInstallState(pkg);
    if (state.status === "success" || state.status === "running") return;

    const sessionId = `mcp-install-${pkg}`;
    setServerState(pkg, { status: "running", logs: [] });
    setActiveTerminal(pkg);

    const unlisten = await listen<{
      session_id: string;
      line: string;
      is_error: boolean;
      done: boolean;
      success?: boolean;
    }>("mcp-install-log", (event) => {
      if (!mountedRef.current) return;
      const payload = event.payload;
      if (payload.session_id !== sessionId) return;

      if (!payload.done) {
        setInstallStates(prev => {
          const existing = prev[pkg] ?? { status: "running", logs: [] };
          return {
            ...prev,
            [pkg]: { ...existing, logs: [...existing.logs, { text: payload.line, isError: payload.is_error }] },
          };
        });
      } else {
        unlisten();
        if (payload.success) {
          setInstallStates(prev => ({
            ...prev,
            [pkg]: {
              ...(prev[pkg] ?? { status: "running", logs: [] }),
              status: "success",
              logs: [...(prev[pkg]?.logs ?? []), { text: t.mcp_marketplace_done_msg, isError: false }],
            },
          }));
          addMcpServer({
            name: server.displayName || pkg,
            enabled: true,
            transport: "stdio" as const,
            command: "npx",
            args: ["-y", pkg],
            env: {},
          }).catch(() => {});
          setTimeout(() => { onInstalled(); }, 3000);
        } else {
          setServerState(pkg, { status: "error" });
        }
      }
    });

    try {
      await invoke("install_mcp_package", { command: "npx", args: ["-y", pkg], sessionId });
    } catch {
      unlisten();
      setServerState(pkg, { status: "error" });
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

  const activeState = activeTerminal ? getInstallState(activeTerminal) : null;

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

      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        {results.map((server) => {
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
                  flexShrink: 0,
                }}
              >
                {getButtonLabel(state.status)}
              </button>
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

      {activeTerminal && (
        <McpInstallTerminal
          command="npx"
          args={["-y", activeTerminal]}
          lines={activeState?.logs ?? []}
          onClose={() => setActiveTerminal(null)}
        />
      )}
    </div>
  );
}
