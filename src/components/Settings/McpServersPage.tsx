// src/components/Settings/McpServersPage.tsx
import { useState, useEffect } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import {
  listMcpServers, removeMcpServer, importClaudeDesktopMcp,
  addMcpServer, setMcpEnabled, getMcpTools,
  type McpServerInfo, type McpServerInput, type McpToolInfo,
} from "../../ipc/mcp";
import { getConfig } from "../../ipc/config";
import { McpServerForm } from "./McpServerForm";
import { McpMarketplaceTab } from "./McpMarketplaceTab";
import "./McpServersPage.css";

export function McpServersPage() {
  const { t } = useLocale();
  const [servers, setServers] = useState<McpServerInfo[]>([]);
  const [mcpEnabled, setMcpEnabledState] = useState(true);
  const [activeTab, setActiveTab] = useState<"installed" | "marketplace">("installed");
  const [editingServer, setEditingServer] = useState<McpServerInfo | null | "new">(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [importList, setImportList] = useState<McpServerInput[] | null>(null);
  const [importSelected, setImportSelected] = useState<Set<string>>(new Set());
  const [importError, setImportError] = useState<string | null>(null);
  const [expandedServerId, setExpandedServerId] = useState<string | null>(null);
  const [allTools, setAllTools] = useState<McpToolInfo[]>([]);

  const reload = async () => {
    const [svrs, cfg] = await Promise.all([listMcpServers(), getConfig()]);
    setServers(svrs);
    setMcpEnabledState(cfg.mcp_enabled ?? true);
  };

  useEffect(() => { reload(); }, []);

  // Refresh tool list whenever a server becomes connected
  useEffect(() => {
    if (servers.some(s => s.status === "connected")) {
      getMcpTools().then(setAllTools).catch(() => {});
    }
  }, [servers]);

  const handleDelete = async (id: string) => {
    if (deletingId !== id) {
      setDeletingId(id);
      return;
    }
    setDeletingId(null);
    await removeMcpServer(id);
    await reload();
  };

  const handleToggleGlobal = async (enabled: boolean) => {
    setMcpEnabledState(enabled);
    await setMcpEnabled(enabled);
  };

  const handleImportClick = async () => {
    setImportError(null);
    try {
      const list = await importClaudeDesktopMcp();
      if (list.length === 0) {
        setImportError(t.mcp_import_none_found);
        return;
      }
      setImportList(list);
      setImportSelected(new Set(list.map(s => s.id ?? s.name)));
    } catch {
      setImportError(t.mcp_import_none_found);
    }
  };

  const handleImportConfirm = async () => {
    if (!importList) return;
    const toImport = importList.filter(s => importSelected.has(s.id ?? s.name));
    for (const server of toImport) {
      try { await addMcpServer(server); } catch { /* skip duplicates */ }
    }
    setImportList(null);
    await reload();
  };

  const statusLabel = (s: McpServerInfo) => {
    switch (s.status) {
      case "connected": return t.mcp_status_connected(s.tool_count);
      case "connecting": return t.mcp_status_connecting;
      case "error": return t.mcp_status_error;
      case "disabled": return t.mcp_status_disabled;
    }
  };

  return (
    <div className="mcp-servers-page">
      <h2>{t.mcp_servers}</h2>
      <p className="section-desc">{t.mcp_servers_desc}</p>

      <div className="mcp-tab-row" role="tablist">
        <button
          className={`mcp-tab-btn${activeTab === "installed" ? " active" : ""}`}
          onClick={() => setActiveTab("installed")}
          role="tab"
          aria-selected={activeTab === "installed"}
        >
          {t.mcp_marketplace_installed}
        </button>
        <button
          className={`mcp-tab-btn${activeTab === "marketplace" ? " active" : ""}`}
          onClick={() => setActiveTab("marketplace")}
          role="tab"
          aria-selected={activeTab === "marketplace"}
        >
          🌐 {t.mcp_marketplace}
        </button>
      </div>

      {activeTab === "installed" && (<>
      {/* Global toggle */}
      <div className="mcp-global-toggle">
        <input
          type="checkbox"
          id="mcp-enabled"
          checked={mcpEnabled}
          onChange={e => handleToggleGlobal(e.target.checked)}
        />
        <div>
          <label htmlFor="mcp-enabled" style={{ fontWeight: 500, cursor: "pointer" }}>
            {t.mcp_enabled_label}
          </label>
          <p className="section-desc" style={{ margin: "2px 0 0" }}>{t.mcp_enabled_desc}</p>
        </div>
      </div>

      {/* Toolbar */}
      <div className="mcp-toolbar">
        <button className="add-btn" onClick={() => setEditingServer("new")}>
          {t.mcp_add_server}
        </button>
        <button className="mcp-btn-sm" onClick={handleImportClick}>
          {t.mcp_import_claude}
        </button>
      </div>
      {importError && <p style={{ color: "#f87171", fontSize: 13 }}>{importError}</p>}

      {/* Server list */}
      <div className="mcp-server-list">
        {servers.length === 0 && (
          <p className="section-desc">{t.mcp_no_servers}</p>
        )}
        {servers.map(s => (
          <div key={s.id} className="mcp-server-row" style={{ flexDirection: "column", alignItems: "stretch", gap: 0 }}>
            <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
              <div style={{ display: "flex", flexDirection: "column", flex: 1, gap: 2 }}>
                <span className="mcp-server-name">{s.name}</span>
                <span className="mcp-server-meta">
                  {s.transport === "stdio" ? s.command : s.url}
                </span>
                {s.status === "error" && s.error_message && (
                  <span style={{ color: "#f87171", fontSize: 11, marginTop: 2 }}>
                    {s.error_message}
                  </span>
                )}
              </div>
              {s.status === "connected" ? (
                <button
                  className={`mcp-status-badge connected`}
                  style={{ cursor: "pointer", background: "none", border: "none", padding: 0 }}
                  onClick={() => setExpandedServerId(prev => prev === s.id ? null : s.id)}
                  title="點擊展開工具列表"
                >
                  {statusLabel(s)} {expandedServerId === s.id ? "▲" : "▼"}
                </button>
              ) : (
                <span className={`mcp-status-badge ${s.status}`}>{statusLabel(s)}</span>
              )}
              <div className="mcp-row-actions">
                <button className="mcp-btn-sm" onClick={() => setEditingServer(s)}>
                  {t.edit}
                </button>
                {deletingId === s.id ? (
                  <>
                    <button className="mcp-btn-sm danger" onClick={() => handleDelete(s.id)}>
                      {t.mcp_confirm_delete_yes}
                    </button>
                    <button className="mcp-btn-sm" onClick={() => setDeletingId(null)}>
                      {t.cancel}
                    </button>
                  </>
                ) : (
                  <button className="mcp-btn-sm danger" onClick={() => handleDelete(s.id)}>
                    {t.delete}
                  </button>
                )}
              </div>
            </div>
            {/* Tool list expansion */}
            {expandedServerId === s.id && (
              <div style={{
                padding: "8px 12px 10px",
                borderTop: "1px solid #1e1e1e",
                display: "flex",
                flexWrap: "wrap",
                gap: "4px 8px",
              }}>
                {allTools
                  .filter(t => t.server_id === s.id)
                  .map(tool => {
                    const displayName = tool.name.includes("__")
                      ? tool.name.split("__").slice(1).join("__")
                      : tool.name;
                    return (
                      <span
                        key={tool.name}
                        title={tool.description}
                        style={{
                          fontSize: 11,
                          background: "#0a1a0a",
                          border: "1px solid #1e3a1e",
                          borderRadius: 3,
                          padding: "1px 7px",
                          color: "#6ee7b7",
                          fontFamily: "monospace",
                        }}
                      >
                        {displayName}
                      </span>
                    );
                  })}
              </div>
            )}
          </div>
        ))}
      </div>

      {/* Add/Edit form */}
      {editingServer !== null && (
        <McpServerForm
          existing={editingServer === "new" ? null : editingServer}
          onSave={async () => { setEditingServer(null); await reload(); }}
          onCancel={() => setEditingServer(null)}
        />
      )}

      {/* Import modal */}
      {importList !== null && (
        <div className="mcp-import-modal" onClick={() => setImportList(null)}>
          <div className="mcp-import-box" onClick={e => e.stopPropagation()}>
            <h3 style={{ margin: 0 }}>{t.mcp_import_title}</h3>
            <p className="section-desc">{t.mcp_import_desc}</p>
            {importList.map(s => {
              const key = s.id ?? s.name;
              return (
                <div key={key} className="mcp-import-item">
                  <input
                    type="checkbox"
                    checked={importSelected.has(key)}
                    onChange={e => {
                      const next = new Set(importSelected);
                      if (e.target.checked) next.add(key); else next.delete(key);
                      setImportSelected(next);
                    }}
                  />
                  <div>
                    <div style={{ fontWeight: 500, fontSize: 13 }}>{s.name}</div>
                    <div style={{ fontSize: 11, color: "#888" }}>{s.command} {s.args.join(" ")}</div>
                  </div>
                </div>
              );
            })}
            <div style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
              <button className="mcp-btn-sm" onClick={() => setImportList(null)}>{t.cancel}</button>
              <button className="add-btn" onClick={handleImportConfirm}>
                {t.mcp_import_confirm}
              </button>
            </div>
          </div>
        </div>
      )}
      </>)}
      {activeTab === "marketplace" && (
        <McpMarketplaceTab
          onInstalled={() => { void reload(); setActiveTab("installed"); }}
        />
      )}
    </div>
  );
}
