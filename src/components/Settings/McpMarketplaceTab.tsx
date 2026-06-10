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
  type SmitheryConnection,
} from "../../lib/smithery";
import { McpInstallTerminal, type InstallLogLine } from "./McpInstallTerminal";

type InstallStatus =
  | "idle"
  | "fetching"
  | "running"
  | "success"
  | "error"
  | "no-connections"
  | "http-added"
  | "needs-config"
  | "confirming-config";

interface ServerInstallState {
  status: InstallStatus;
  detail?: SmitheryServerDetail;
  logs: InstallLogLine[];
  configInputs: Record<string, string>;
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
    return installStates[qualifiedName] ?? { status: "idle", logs: [], configInputs: {} };
  }

  function setServerState(qualifiedName: string, update: Partial<ServerInstallState>) {
    setInstallStates((prev) => ({
      ...prev,
      [qualifiedName]: { ...(prev[qualifiedName] ?? { status: "idle", logs: [], configInputs: {} }), ...update },
    }));
  }

  function setConfigInput(qualifiedName: string, key: string, value: string) {
    setInstallStates((prev) => {
      const cur = prev[qualifiedName] ?? { status: "idle", logs: [], configInputs: {} };
      return {
        ...prev,
        [qualifiedName]: { ...cur, configInputs: { ...cur.configInputs, [key]: value } },
      };
    });
  }

  async function handleInstall(server: SmitheryServer) {
    const { qualifiedName } = server;
    const current = getInstallState(qualifiedName);
    if (current.status === "success" || current.status === "http-added") return;

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

    setServerState(qualifiedName, { status: "fetching", logs: [], configInputs: {} });

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

    if (!stdioConn) {
      const httpConn = detail.connections.find(
        (c) => (c.type === "http" || c.type === "sse") && (c.url ?? c.deploymentUrl)
      );
      if (httpConn) {
        const requiredFields = httpConn.configSchema?.required ?? [];
        if (requiredFields.length > 0) {
          // Show inline config form
          setServerState(qualifiedName, { status: "needs-config", detail, logs: [], configInputs: {} });
          return;
        }
        await addHttpServer(qualifiedName, detail, httpConn, {});
        return;
      }
      setServerState(qualifiedName, { status: "no-connections", detail });
      return;
    }

    await runStdioInstall(qualifiedName, detail, stdioConn);
  }

  async function handleConfigConfirm(qualifiedName: string) {
    const state = getInstallState(qualifiedName);
    const detail = state.detail!;
    const httpConn = detail.connections.find(
      (c) => (c.type === "http" || c.type === "sse") && (c.url ?? c.deploymentUrl)
    )!;
    setServerState(qualifiedName, { status: "confirming-config" });
    await addHttpServer(qualifiedName, detail, httpConn, state.configInputs);
  }

  async function addHttpServer(
    qualifiedName: string,
    detail: SmitheryServerDetail,
    httpConn: SmitheryConnection,
    configInputs: Record<string, string>
  ) {
    const baseUrl = httpConn.url ?? httpConn.deploymentUrl ?? "";
    const params = new URLSearchParams();
    for (const [key, value] of Object.entries(configInputs)) {
      if (value.trim()) params.set(key, value.trim());
    }
    const url = params.toString() ? `${baseUrl}?${params.toString()}` : baseUrl;
    try {
      await addMcpServer({
        name: detail.displayName || qualifiedName,
        enabled: true,
        transport: "http" as const,
        command: "",
        args: [],
        env: {},
        url,
      });
      setServerState(qualifiedName, { status: "http-added" });
      setTimeout(() => { onInstalled(); }, 2000);
    } catch {
      setServerState(qualifiedName, { status: "error" });
    }
  }

  async function runStdioInstall(
    qualifiedName: string,
    detail: SmitheryServerDetail,
    stdioConn: SmitheryConnection
  ) {
    const { command, args, env } = stdioConn.stdioFunction!;
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
          const existing = prev[qualifiedName] ?? { status: "running", logs: [], configInputs: {} };
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
              ...(prev[qualifiedName] ?? { status: "running", logs: [], configInputs: {} }),
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
          setTimeout(() => { onInstalled(); }, 3000);
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
      case "confirming-config":
        return t.mcp_marketplace_installing;
      case "running":
        return t.mcp_marketplace_installing;
      case "success":
      case "http-added":
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
          const showConfigForm = state.status === "needs-config";
          const httpConn = state.detail?.connections.find(
            (c) => c.type === "http" || c.type === "sse"
          );
          const configProps = httpConn?.configSchema?.properties ?? {};
          const requiredFields = httpConn?.configSchema?.required ?? [];

          const isDisabled =
            state.status === "fetching" ||
            state.status === "running" ||
            state.status === "confirming-config" ||
            state.status === "success" ||
            state.status === "http-added" ||
            state.status === "needs-config";

          return (
            <div
              key={server.qualifiedName}
              style={{
                background: "#1a1a1a",
                border: `1px solid ${showConfigForm ? "#34d39944" : "#2a2a2a"}`,
                borderRadius: 6,
                padding: "10px 12px",
                display: "flex",
                flexDirection: "column",
                gap: 10,
              }}
            >
              {/* Header row */}
              <div style={{ display: "flex", alignItems: "flex-start", gap: 12 }}>
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
                  {state.status === "no-connections" && (
                    <div style={{ color: "#f59e0b", fontSize: 11, marginTop: 4 }}>
                      {t.mcp_marketplace_no_connections}
                    </div>
                  )}
                </div>
                {!showConfigForm && (
                  <button
                    onClick={() => handleInstall(server)}
                    disabled={isDisabled}
                    style={{
                      background: (state.status === "success" || state.status === "http-added") ? "#166534" : "#2a2a2a",
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
                )}
              </div>

              {/* Inline config form for HTTP servers requiring API keys */}
              {showConfigForm && (
                <div style={{
                  borderTop: "1px solid #2a2a2a",
                  paddingTop: 10,
                  display: "flex",
                  flexDirection: "column",
                  gap: 10,
                }}>
                  <div style={{ color: "#aaa", fontSize: 12 }}>
                    {t.mcp_marketplace_config_title}
                  </div>

                  {Object.entries(configProps).map(([key, prop]) => {
                    const isRequired = requiredFields.includes(key);
                    return (
                      <div key={key} style={{ display: "flex", flexDirection: "column", gap: 3 }}>
                        <label style={{ fontSize: 12, color: "#ccc" }}>
                          {prop.title ?? key}
                          {isRequired && <span style={{ color: "#f87171", marginLeft: 3 }}>*</span>}
                        </label>
                        <input
                          className="mcp-marketplace-search"
                          style={{ fontSize: 12, padding: "5px 8px" }}
                          type={key.toLowerCase().includes("key") || key.toLowerCase().includes("token") || key.toLowerCase().includes("secret") ? "password" : "text"}
                          placeholder={prop.description ? prop.description.slice(0, 60) : key}
                          value={state.configInputs[key] ?? ""}
                          onChange={(e) => setConfigInput(server.qualifiedName, key, e.target.value)}
                          autoCorrect="off"
                          spellCheck={false}
                        />
                        {prop.description && (
                          <span style={{ fontSize: 10, color: "#555", lineHeight: 1.4 }}>
                            {prop.description}
                          </span>
                        )}
                      </div>
                    );
                  })}

                  {/* If no properties defined in schema, show a generic URL input */}
                  {Object.keys(configProps).length === 0 && (
                    <div style={{ color: "#555", fontSize: 12 }}>
                      {t.mcp_marketplace_config_no_schema}
                    </div>
                  )}

                  <div style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
                    <button
                      className="mcp-btn-sm"
                      onClick={() => setServerState(server.qualifiedName, { status: "idle" })}
                    >
                      {t.cancel}
                    </button>
                    <button
                      className="add-btn"
                      style={{ fontSize: 12, padding: "4px 14px" }}
                      onClick={() => handleConfigConfirm(server.qualifiedName)}
                      disabled={requiredFields.some(f => !state.configInputs[f]?.trim())}
                    >
                      {t.mcp_marketplace_config_confirm}
                    </button>
                  </div>
                </div>
              )}
            </div>
          );
        })}
      </div>

      {activeTerminal && activeConn?.stdioFunction && (
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
