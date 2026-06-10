import { useState, useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { addMcpServer } from "../../ipc/mcp";
import { useLocale } from "../../contexts/LocaleContext";
import {
  searchSmithery,
  getSmitheryServer,
  type SmitheryServer,
  type SmitheryServerDetail,
} from "../../lib/smithery";
import { McpInstallTerminal, type InstallLogLine } from "./McpInstallTerminal";

type InstallStatus =
  | "idle"
  | "fetching"
  | "running"
  | "success"
  | "error"
  | "no-connections";

interface ServerInstallState {
  status: InstallStatus;
  detail?: SmitheryServerDetail;
  logs: InstallLogLine[];
}

interface Props {
  onInstalled: () => void;
}

export function McpMarketplaceTab({ onInstalled }: Props) {
  const { t } = useLocale();
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SmitheryServer[]>([]);
  const [isSearching, setIsSearching] = useState(false);
  const [networkError, setNetworkError] = useState(false);
  const [installStates, setInstallStates] = useState<
    Record<string, ServerInstallState>
  >({});
  const [activeTerminal, setActiveTerminal] = useState<string | null>(null);

  const mountedRef = useRef(true);
  useEffect(() => () => { mountedRef.current = false; }, []);

  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);

    if (!query.trim()) {
      setResults([]);
      setNetworkError(false);
      return;
    }

    debounceRef.current = setTimeout(async () => {
      setIsSearching(true);
      setNetworkError(false);
      try {
        const res = await searchSmithery(query);
        setResults(res);
      } catch {
        setNetworkError(true);
        setResults([]);
      } finally {
        setIsSearching(false);
      }
    }, 500);

    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, [query]);

  function getInstallState(qualifiedName: string): ServerInstallState {
    return installStates[qualifiedName] ?? { status: "idle", logs: [] };
  }

  function setServerState(
    qualifiedName: string,
    update: Partial<ServerInstallState>
  ) {
    setInstallStates((prev) => ({
      ...prev,
      [qualifiedName]: { ...(prev[qualifiedName] ?? { status: "idle", logs: [] }), ...update },
    }));
  }

  async function handleInstall(server: SmitheryServer) {
    const { qualifiedName } = server;
    const current = getInstallState(qualifiedName);
    if (current.status === "success") return;

    // no-connections: copy command to clipboard
    if (current.status === "no-connections") {
      const detail = current.detail;
      if (detail?.connections[0]) {
        const conn = detail.connections[0];
        const cmd = conn.type === "stdio" && conn.stdioFunction
          ? `${conn.stdioFunction.command} ${conn.stdioFunction.args.join(" ")}`
          : conn.url ?? qualifiedName;
        await navigator.clipboard.writeText(cmd);
      }
      return;
    }

    setServerState(qualifiedName, { status: "fetching", logs: [] });

    let detail: SmitheryServerDetail;
    try {
      detail = await getSmitheryServer(qualifiedName);
    } catch {
      setServerState(qualifiedName, { status: "error" });
      return;
    }

    const stdioConn = detail.connections.find(
      (c) => c.type === "stdio" && c.stdioFunction
    );

    if (!stdioConn || !stdioConn.stdioFunction) {
      setServerState(qualifiedName, { status: "no-connections", detail });
      return;
    }

    const { command, args, env } = stdioConn.stdioFunction;
    const sessionId = `mcp-install-${qualifiedName}`;

    setServerState(qualifiedName, { status: "running", detail, logs: [] });
    setActiveTerminal(qualifiedName);

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
        setInstallStates((prev) => {
          const existing = prev[qualifiedName] ?? { status: "running", logs: [] };
          return {
            ...prev,
            [qualifiedName]: {
              ...existing,
              logs: [...existing.logs, { text: payload.line, isError: payload.is_error }],
            },
          };
        });
      } else {
        unlisten();
        if (payload.success) {
          setInstallStates((prev) => ({
            ...prev,
            [qualifiedName]: {
              ...(prev[qualifiedName] ?? { status: "running", logs: [] }),
              status: "success",
              logs: [...(prev[qualifiedName]?.logs ?? []), { text: t.mcp_marketplace_done_msg, isError: false }],
            },
          }));
          addMcpServer({
            name: detail.displayName || qualifiedName,
            enabled: true,
            transport: "stdio" as const,
            command,
            args,
            env,
          }).catch(() => {});
          setTimeout(() => {
            onInstalled();
          }, 3000);
        } else {
          setServerState(qualifiedName, { status: "error" });
        }
      }
    });

    try {
      await invoke("install_mcp_package", { command, args, sessionId });
    } catch {
      unlisten();
      setServerState(qualifiedName, { status: "error" });
    }
  }

  function getButtonLabel(status: InstallStatus): string {
    switch (status) {
      case "fetching":
      case "running":
        return t.mcp_marketplace_installing;
      case "success":
        return t.mcp_marketplace_installed_done;
      case "error":
        return t.mcp_marketplace_failed;
      case "no-connections":
        return t.mcp_marketplace_copy_cmd;
      default:
        return t.mcp_marketplace_install;
    }
  }

  const activeState = activeTerminal ? getInstallState(activeTerminal) : null;
  const activeDetail = activeState?.detail;
  const activeConn = activeDetail?.connections.find(
    (c) => c.type === "stdio" && c.stdioFunction
  );

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
        <div
          data-testid="network-error"
          style={{ color: "#f87171", fontSize: 12 }}
        >
          {t.mcp_marketplace_network_error}
        </div>
      )}

      {isSearching && (
        <div style={{ color: "#555", fontSize: 12 }}>...</div>
      )}

      {!isSearching && query.trim() && results.length === 0 && !networkError && (
        <div className="mcp-marketplace-empty">{t.mcp_marketplace_no_results}</div>
      )}

      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        {results.map((server) => {
          const state = getInstallState(server.qualifiedName);
          const isDisabled =
            state.status === "fetching" ||
            state.status === "running" ||
            state.status === "success";

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
                justifyContent: "space-between",
                gap: 12,
              }}
            >
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ color: "#eee", fontSize: 13, fontWeight: 500 }}>
                  {server.displayName}
                </div>
                <div
                  style={{
                    color: "#666",
                    fontSize: 11,
                    marginTop: 2,
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                  }}
                >
                  {server.description}
                </div>
                {state.status === "no-connections" && (
                  <div style={{ color: "#f59e0b", fontSize: 11, marginTop: 4 }}>
                    {t.mcp_marketplace_no_connections}
                  </div>
                )}
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

      {activeTerminal &&
        activeConn?.stdioFunction && (
          <McpInstallTerminal
            command={activeConn.stdioFunction.command}
            args={activeConn.stdioFunction.args}
            lines={activeState?.logs ?? []}
            onClose={() => setActiveTerminal(null)}
          />
        )}
    </div>
  );
}
