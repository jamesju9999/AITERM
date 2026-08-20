import { useCallback, useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { getConfig } from "../../ipc/config";
import {
  mcpToolServerSetConfig,
  mcpToolServerStatus,
  type McpToolServerConfig,
  type McpToolServerStatus,
} from "../../ipc/mcpToolServer";
import "./McpToolServerPage.css";

export function McpToolServerPage() {
  const { t } = useLocale();
  const [cfg, setCfg] = useState<McpToolServerConfig | null>(null);
  const [status, setStatus] = useState<McpToolServerStatus | null>(null);
  const [saving, setSaving] = useState(false);
  const [copied, setCopied] = useState(false);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    void (async () => {
      const [c, s] = await Promise.all([getConfig(), mcpToolServerStatus()]);
      setCfg(c.mcp_tool_server);
      setStatus(s);
    })();
  }, []);

  const updateCfg: typeof setCfg = useCallback((next) => {
    setSaved(false);
    setCfg(next);
  }, []);

  const save = useCallback(async () => {
    if (!cfg) return;
    setSaving(true);
    try {
      setStatus(await mcpToolServerSetConfig(cfg));
      setSaved(true);
    } finally {
      setSaving(false);
    }
  }, [cfg]);

  const registerCommand = (): string => {
    const port = status?.port ?? cfg?.port ?? 8318;
    const token = status?.token ?? "<token>";
    return `claude mcp add --transport http aiterm-tools http://127.0.0.1:${port}/mcp --header "Authorization: Bearer ${token}"`;
  };

  if (!cfg) return <div className="mcp-tool-server-page" />;

  return (
    <div className="mcp-tool-server-page">
      <h2>{t.mcp_tool_server_title}</h2>
      <p className="mcp-tool-server-desc">{t.mcp_tool_server_desc}</p>

      <div className="mcp-tool-server-status">
        <span className={status?.running ? "mcp-tool-server-dot mcp-tool-server-dot--on" : "mcp-tool-server-dot"} />
        {status?.running ? t.mcp_tool_server_status_running : t.mcp_tool_server_status_stopped}
        {status?.port ? ` · :${status.port}` : ""}
      </div>
      {status?.error && <div className="mcp-tool-server-error">{status.error}</div>}

      <section className="mcp-tool-server-section">
        <label className="mcp-tool-server-row">
          <input
            type="checkbox"
            checked={cfg.enabled}
            onChange={(e) => updateCfg({ ...cfg, enabled: e.target.checked })}
          />
          {t.mcp_tool_server_enable}
        </label>

        <label className="mcp-tool-server-row">
          {t.mcp_tool_server_port}
          <input
            type="number"
            value={cfg.port}
            onChange={(e) => updateCfg({ ...cfg, port: Number(e.target.value) || 8318 })}
          />
        </label>
      </section>

      {status?.running && (
        <section className="mcp-tool-server-section">
          <h3>{t.mcp_tool_server_section_register}</h3>
          <div className="mcp-tool-server-command">{registerCommand()}</div>
        </section>
      )}

      <div className="mcp-tool-server-actions">
        <button onClick={() => void save()} disabled={saving}>
          {saved ? `${t.mcp_tool_server_saved} ✓` : t.save}
        </button>
        {status?.running && (
          <button
            onClick={() => {
              void navigator.clipboard.writeText(registerCommand());
              setCopied(true);
            }}
          >
            {copied ? t.mcp_tool_server_copied : t.mcp_tool_server_copy_command}
          </button>
        )}
      </div>
    </div>
  );
}
