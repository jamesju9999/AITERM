// src/components/Settings/McpServerForm.tsx
import { useState } from "react";
import {
  addMcpServer, updateMcpServer,
  type McpServerInfo, type McpServerInput, type McpTransport,
} from "../../ipc/mcp";
import { useLocale } from "../../contexts/LocaleContext";

interface Props {
  existing: McpServerInfo | null;
  onSave: () => void;
  onCancel: () => void;
}

const EMPTY_FORM: McpServerInput = {
  id: undefined,
  name: "",
  enabled: true,
  transport: "stdio",
  command: "",
  args: [],
  env: {},
  url: "",
};

export function McpServerForm({ existing, onSave, onCancel }: Props) {
  const { t } = useLocale();
  const [form, setForm] = useState<McpServerInput>(() =>
    existing
      ? {
          id: existing.id,
          name: existing.name,
          enabled: existing.enabled,
          transport: existing.transport,
          command: existing.command ?? "",
          args: existing.args,
          env: {},  // env is not fetched from backend for security
          url: existing.url ?? "",
        }
      : EMPTY_FORM
  );
  const [argsText, setArgsText] = useState(() => form.args.join("\n"));
  const [envText, setEnvText] = useState(() =>
    Object.entries(form.env).map(([k, v]) => `${k}=${v}`).join("\n")
  );
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const parseArgs = (text: string): string[] =>
    text.split("\n").map(s => s.trim()).filter(Boolean);

  const parseEnv = (text: string): Record<string, string> => {
    const result: Record<string, string> = {};
    for (const line of text.split("\n")) {
      const eq = line.indexOf("=");
      if (eq > 0) {
        result[line.slice(0, eq).trim()] = line.slice(eq + 1).trim();
      }
    }
    return result;
  };

  const handleSave = async () => {
    if (!form.name.trim()) { setError("名稱不可為空"); return; }
    if (form.transport === "stdio" && !form.command?.trim()) {
      setError("stdio transport 需要填寫 Command"); return;
    }
    if ((form.transport === "http" || form.transport === "sse") && !form.url?.trim()) {
      setError("HTTP/SSE transport 需要填寫 URL"); return;
    }

    setSaving(true);
    setError(null);
    try {
      const payload: McpServerInput = {
        ...form,
        args: parseArgs(argsText),
        env: parseEnv(envText),
      };
      if (existing) {
        await updateMcpServer(payload);
      } else {
        await addMcpServer(payload);
      }
      onSave();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div style={{
      position: "fixed", inset: 0, background: "rgba(0,0,0,0.6)",
      display: "flex", alignItems: "center", justifyContent: "center", zIndex: 100,
    }}>
      <div style={{
        background: "#1a1a1a", border: "1px solid #333", borderRadius: 8,
        padding: 24, width: 480, maxWidth: "90vw", maxHeight: "80vh",
        overflowY: "auto", display: "flex", flexDirection: "column", gap: 16,
      }}>
        <h3 style={{ margin: 0 }}>{existing ? "編輯 MCP Server" : "新增 MCP Server"}</h3>

        {/* Name */}
        <label style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <span style={{ fontSize: 13, color: "#aaa" }}>名稱</span>
          <input
            className="settings-input"
            value={form.name}
            onChange={e => setForm(f => ({ ...f, name: e.target.value }))}
            placeholder="例如：Filesystem"
            autoCorrect="off"
            spellCheck={false}
          />
        </label>

        {/* Transport */}
        <label style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <span style={{ fontSize: 13, color: "#aaa" }}>Transport</span>
          <select
            className="step-select"
            value={form.transport}
            onChange={e => setForm(f => ({ ...f, transport: e.target.value as McpTransport }))}
          >
            <option value="stdio">{t.mcp_transport_stdio}</option>
            <option value="http">{t.mcp_transport_http}</option>
            <option value="sse">{t.mcp_transport_sse}</option>
          </select>
        </label>

        {/* stdio fields */}
        {form.transport === "stdio" && (
          <>
            <label style={{ display: "flex", flexDirection: "column", gap: 4 }}>
              <span style={{ fontSize: 13, color: "#aaa" }}>Command</span>
              <input
                className="settings-input"
                value={form.command ?? ""}
                onChange={e => setForm(f => ({ ...f, command: e.target.value }))}
                placeholder="例如：npx / python3 / uvx"
                autoCorrect="off"
                spellCheck={false}
              />
            </label>
            <label style={{ display: "flex", flexDirection: "column", gap: 4 }}>
              <span style={{ fontSize: 13, color: "#aaa" }}>Args（每行一個）</span>
              <textarea
                className="settings-input"
                rows={3}
                value={argsText}
                onChange={e => setArgsText(e.target.value)}
                placeholder="-y&#10;@modelcontextprotocol/server-filesystem&#10;/path/to/dir"
                autoCorrect="off"
                spellCheck={false}
                style={{ resize: "vertical", fontFamily: "monospace", fontSize: 12 }}
              />
            </label>
            <label style={{ display: "flex", flexDirection: "column", gap: 4 }}>
              <span style={{ fontSize: 13, color: "#aaa" }}>
                Env Vars（每行 KEY=VALUE，不需要引號）
                {existing && <span style={{ color: "#666", fontSize: 11, marginLeft: 6 }}>— 留空保留原有設定</span>}
              </span>
              <textarea
                className="settings-input"
                rows={3}
                value={envText}
                onChange={e => setEnvText(e.target.value)}
                placeholder="BRAVE_API_KEY=your_key_here"
                autoCorrect="off"
                spellCheck={false}
                style={{ resize: "vertical", fontFamily: "monospace", fontSize: 12 }}
              />
            </label>
          </>
        )}

        {/* http/sse fields */}
        {(form.transport === "http" || form.transport === "sse") && (
          <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
            <span style={{ fontSize: 13, color: "#aaa" }}>URL</span>
            <input
              className="settings-input"
              value={form.url ?? ""}
              onChange={e => setForm(f => ({ ...f, url: e.target.value }))}
              placeholder="例如：https://server.run.tools"
              autoCorrect="off"
              spellCheck={false}
            />
            <span style={{ fontSize: 11, color: "#555", lineHeight: 1.5 }}>
              如需 API Key，附加於 URL 後面，例如：
              <code style={{ color: "#888", fontSize: 11 }}>
                https://server.run.tools?apiKey=your_key
              </code>
            </span>
          </div>
        )}

        {/* Enabled */}
        <label style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <input
            type="checkbox"
            checked={form.enabled}
            onChange={e => setForm(f => ({ ...f, enabled: e.target.checked }))}
          />
          <span style={{ fontSize: 13, color: "#ccc" }}>啟用此 Server</span>
        </label>

        {error && <p style={{ color: "#f87171", fontSize: 13, margin: 0 }}>{error}</p>}

        <div style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
          <button className="mcp-btn-sm" onClick={onCancel}>{t.cancel}</button>
          <button className="add-btn" onClick={handleSave} disabled={saving}>
            {saving ? t.saving_btn : t.save}
          </button>
        </div>
      </div>
    </div>
  );
}
