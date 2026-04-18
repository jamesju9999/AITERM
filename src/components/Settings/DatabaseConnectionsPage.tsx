import { useEffect, useState, type CSSProperties } from "react";
import {
  dbListConnections, dbAddConnection, dbUpdateConnection, dbRemoveConnection,
  dbTestConnection,
  type DbConnectionInfo, type DbConnectionInput, type DbType,
  DB_TYPE_LABELS, DB_DEFAULT_PORTS,
} from "../../ipc/db";
import { useLocale } from "../../contexts/LocaleContext";

type FormState = Omit<DbConnectionInput, "id"> & { id?: string };

const EMPTY_FORM: FormState = {
  name: "", db_type: "postgresql", host: "localhost",
  port: 5432, database: "", username: "", password: "",
};

export function DatabaseConnectionsPage() {
  const { t } = useLocale();
  const [connections, setConnections] = useState<DbConnectionInfo[]>([]);
  const [showForm, setShowForm] = useState(false);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [testStatus, setTestStatus] = useState<"idle" | "testing" | "ok" | "error">("idle");
  const [testMsg, setTestMsg] = useState("");
  const [saving, setSaving] = useState(false);

  const load = () => dbListConnections().then(setConnections).catch(console.error);
  useEffect(() => { load(); }, []);

  const handleDbTypeChange = (db_type: DbType) => {
    setForm((f) => ({ ...f, db_type, port: DB_DEFAULT_PORTS[db_type] }));
  };

  const handleTest = async () => {
    setTestStatus("testing");
    setTestMsg("");
    try {
      await dbTestConnection({ ...form, password: form.password });
      setTestStatus("ok");
      setTestMsg(t.connection_success);
    } catch (e: unknown) {
      setTestStatus("error");
      setTestMsg(String(e));
    }
  };

  const handleSave = async () => {
    setSaving(true);
    try {
      if (form.id) {
        await dbUpdateConnection({ ...form, id: form.id });
      } else {
        await dbAddConnection(form as DbConnectionInput);
      }
      setShowForm(false);
      setForm(EMPTY_FORM);
      load();
    } catch (e) {
      console.error(e);
    } finally {
      setSaving(false);
    }
  };

  const handleEdit = (conn: DbConnectionInfo) => {
    setForm({
      id: conn.id, name: conn.name, db_type: conn.db_type,
      host: conn.host, port: conn.port, database: conn.database,
      username: conn.username, password: "",
    });
    setShowForm(true);
    setTestStatus("idle");
  };

  const handleDelete = async (id: string) => {
    if (!confirm(t.confirm_delete)) return;
    await dbRemoveConnection(id);
    load();
  };

  return (
    <div style={{ width: "100%", padding: "24px 32px", boxSizing: "border-box" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 16 }}>
        <h2 style={{ margin: 0, fontSize: 16, color: "#e6e6e6" }}>{t.db_connections}</h2>
        {!showForm && (
          <button
            onClick={() => { setForm(EMPTY_FORM); setShowForm(true); setTestStatus("idle"); }}
            style={{ background: "#1e3a2e", border: "1px solid #34d399", color: "#34d399", borderRadius: 5, padding: "6px 14px", cursor: "pointer", fontSize: 13 }}
          >
            {t.add_connection}
          </button>
        )}
      </div>

      {!showForm && (
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          {connections.length === 0 && (
            <div style={{ color: "#555", fontSize: 13, padding: "20px 0" }}>{t.no_connections}</div>
          )}
          {connections.map((conn) => (
            <div key={conn.id} style={{ background: "#1a1a1a", border: "1px solid #2a2a2a", borderRadius: 6, padding: "10px 14px", display: "flex", justifyContent: "space-between", alignItems: "center", minWidth: 0 }}>
              <div style={{ flex: 1, minWidth: 0, overflow: "hidden" }}>
                <div style={{ color: "#e6e6e6", fontSize: 13, fontWeight: 500, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{conn.name}</div>
                <div style={{ color: "#888", fontSize: 11, marginTop: 2, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                  {DB_TYPE_LABELS[conn.db_type]} · {conn.host}:{conn.port} / {conn.database}
                </div>
              </div>
              <div style={{ display: "flex", gap: 8, alignItems: "center", flexShrink: 0, marginLeft: 12 }}>
                {conn.is_connected && <span style={{ color: "#34d399", fontSize: 11 }}>{t.connected}</span>}
                <button onClick={() => handleEdit(conn)} style={btnStyle}>{t.edit}</button>
                <button onClick={() => handleDelete(conn.id)} style={{ ...btnStyle, color: "#f87171", borderColor: "#f87171" }}>{t.delete}</button>
              </div>
            </div>
          ))}
        </div>
      )}

      {showForm && (
        <div style={{ background: "#1a1a1a", border: "1px solid #2a2a2a", borderRadius: 8, padding: 20 }}>
          <h3 style={{ margin: "0 0 16px", fontSize: 14, color: "#e6e6e6" }}>
            {form.id ? t.edit_connection : t.new_connection}
          </h3>
          <div style={{ display: "grid", gridTemplateColumns: "120px 1fr", gap: "10px 12px", alignItems: "center" }}>
            <label style={labelStyle}>{t.name}</label>
            <input
              value={form.name}
              onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))}
              style={inputStyle}
            />
            <label style={labelStyle}>{t.type}</label>
            <select
              value={form.db_type}
              onChange={(e) => handleDbTypeChange(e.target.value as DbType)}
              style={inputStyle}
            >
              {(Object.entries(DB_TYPE_LABELS) as [DbType, string][]).map(([k, v]) => (
                <option key={k} value={k}>{v}</option>
              ))}
            </select>

            {form.db_type === "sqlite" ? (
              <>
                <label style={labelStyle}>{t.file_path}</label>
                <input
                  placeholder="/path/to/database.db"
                  value={form.host}
                  onChange={(e) => setForm((f) => ({ ...f, host: e.target.value }))}
                  style={inputStyle}
                />
              </>
            ) : (
              <>
                <label style={labelStyle}>{t.host}</label>
                <input
                  placeholder="localhost"
                  value={form.host}
                  onChange={(e) => setForm((f) => ({ ...f, host: e.target.value }))}
                  style={inputStyle}
                />
                <label style={labelStyle}>{t.port}</label>
                <input
                  type="number"
                  value={form.port}
                  onChange={(e) => setForm((f) => ({ ...f, port: Number(e.target.value) }))}
                  style={inputStyle}
                />
                <label style={labelStyle}>{t.database}</label>
                <input
                  value={form.database}
                  onChange={(e) => setForm((f) => ({ ...f, database: e.target.value }))}
                  style={inputStyle}
                />
                <label style={labelStyle}>{t.username}</label>
                <input
                  value={form.username}
                  onChange={(e) => setForm((f) => ({ ...f, username: e.target.value }))}
                  style={inputStyle}
                />
                <label style={labelStyle}>{t.password}</label>
                <input
                  type="password"
                  placeholder={form.id ? t.password_placeholder : ""}
                  value={form.password}
                  onChange={(e) => setForm((f) => ({ ...f, password: e.target.value }))}
                  style={inputStyle}
                />
              </>
            )}
          </div>

          {form.db_type === "db2" && (
            <div style={{ background: "#2a1a00", border: "1px solid #f9a825", borderRadius: 5, padding: "10px 14px", marginTop: 12, fontSize: 12, color: "#f9a825" }}>
              {t.db2_warning_line1}
              <br />
              {t.db2_warning_line2}
            </div>
          )}

          <div style={{ display: "flex", gap: 8, marginTop: 16, alignItems: "center" }}>
            <button onClick={handleTest} disabled={testStatus === "testing"} style={btnStyle}>
              {testStatus === "testing" ? t.testing : t.test_connection}
            </button>
            {testStatus === "ok" && <span style={{ color: "#34d399", fontSize: 12 }}>✓ {testMsg}</span>}
            {testStatus === "error" && <span style={{ color: "#f87171", fontSize: 12 }}>✗ {testMsg}</span>}
            <div style={{ flex: 1 }} />
            <button onClick={() => setShowForm(false)} style={btnStyle}>{t.cancel}</button>
            <button onClick={handleSave} disabled={saving} style={{ ...btnStyle, background: "#1e3a2e", borderColor: "#34d399", color: "#34d399" }}>
              {saving ? t.saving_btn : t.save}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

const btnStyle: CSSProperties = {
  background: "transparent", border: "1px solid #3a3a3a", color: "#ccc",
  borderRadius: 4, padding: "4px 12px", cursor: "pointer", fontSize: 12,
};
const labelStyle: CSSProperties = { color: "#888", fontSize: 12 };
const inputStyle: CSSProperties = {
  background: "#0f0f0f", border: "1px solid #2a2a2a", color: "#e6e6e6",
  borderRadius: 4, padding: "6px 8px", fontSize: 13, width: "100%", boxSizing: "border-box",
};
