import { useEffect, useState, type CSSProperties } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  dbListConnections, dbAddConnection, dbUpdateConnection, dbRemoveConnection,
  dbTestConnection, dbCheckImportFile,
  type DbConnectionInfo, type DbConnectionInput, type DbType,
  DB_TYPE_LABELS, DB_DEFAULT_PORTS,
} from "../../ipc/db";
import { useLocale } from "../../contexts/LocaleContext";
import { DbExportPanel, DbImportPanel, translateDbTransferError } from "./DbConnectionTransfer";

type FormState = Omit<DbConnectionInput, "id"> & { id?: string };

const EMPTY_FORM: FormState = {
  name: "", db_type: "postgresql", host: "localhost",
  port: 5432, database: "", default_schema: "", username: "", password: "",
};

export function DatabaseConnectionsPage() {
  const { t } = useLocale();
  const [connections, setConnections] = useState<DbConnectionInfo[]>([]);
  const [showForm, setShowForm] = useState(false);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [testStatus, setTestStatus] = useState<"idle" | "testing" | "ok" | "error">("idle");
  const [testMsg, setTestMsg] = useState("");
  const [saving, setSaving] = useState(false);
  const [confirmingDelete, setConfirmingDelete] = useState<string | null>(null);
  const [transfer, setTransfer] = useState<
    { kind: "export" } | { kind: "import"; path: string } | null
  >(null);
  const [notice, setNotice] = useState("");
  const [transferError, setTransferError] = useState("");

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
      default_schema: conn.default_schema ?? "",
      username: conn.username, password: "",
    });
    setShowForm(true);
    setTestStatus("idle");
  };

  const handleDelete = async (id: string) => {
    await dbRemoveConnection(id);
    setConfirmingDelete(null);
    load();
  };

  // 選檔與 header 檢查都放在 click handler 裡——這是使用者手勢，StrictMode
  // 不會像對待 effect 那樣重跑一次。壞掉的檔案在這裡就結束，連面板都不會開。
  const handleImportClick = async () => {
    setNotice("");
    setTransferError("");
    try {
      const picked = await open({
        multiple: false,
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (typeof picked !== "string") return; // 使用者取消
      await dbCheckImportFile(picked);
      setTransfer({ kind: "import", path: picked });
    } catch (e) {
      setTransferError(translateDbTransferError(t, e));
    }
  };

  return (
    <div style={{ width: "100%", padding: "24px 32px", boxSizing: "border-box" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 16 }}>
        <h2 style={{ margin: 0, fontSize: 16, color: "#e6e6e6" }}>{t.db_connections}</h2>
        {!showForm && !transfer && (
          <div style={{ display: "flex", gap: 8 }}>
            <button
              onClick={() => { setTransfer({ kind: "export" }); setNotice(""); setTransferError(""); }}
              disabled={connections.length === 0}
              className="aiterm-btn aiterm-btn--secondary"
            >
              {t.db_export}
            </button>
            <button
              onClick={handleImportClick}
              className="aiterm-btn aiterm-btn--secondary"
            >
              {t.db_import}
            </button>
            <button
              onClick={() => { setForm(EMPTY_FORM); setShowForm(true); setTestStatus("idle"); }}
              className="aiterm-btn aiterm-btn--primary"
            >
              {t.add_connection}
            </button>
          </div>
        )}
      </div>

      {notice && <div style={{ color: "#34d399", fontSize: 12, marginBottom: 12 }}>{notice}</div>}
      {transferError && <div style={{ color: "#f87171", fontSize: 12, marginBottom: 12 }}>{transferError}</div>}

      {transfer?.kind === "export" && (
        <DbExportPanel
          connections={connections}
          onClose={() => setTransfer(null)}
          onDone={(msg) => { setNotice(msg); setTransfer(null); }}
        />
      )}
      {transfer?.kind === "import" && (
        <DbImportPanel
          path={transfer.path}
          onClose={() => setTransfer(null)}
          onDone={() => load()}
        />
      )}

      {!showForm && !transfer && (
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
                  {conn.default_schema ? ` · schema=${conn.default_schema}` : ""}
                </div>
              </div>
              <div style={{ display: "flex", gap: 8, alignItems: "center", flexShrink: 0, marginLeft: 12 }}>
                {conn.is_connected && <span style={{ color: "#34d399", fontSize: 11 }}>{t.connected}</span>}
                <button onClick={() => handleEdit(conn)} className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm">{t.edit}</button>
                {confirmingDelete === conn.id ? (
                  <>
                    <button onClick={() => setConfirmingDelete(null)} className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm">{t.cancel}</button>
                    <button onClick={() => handleDelete(conn.id)} className="aiterm-btn aiterm-btn--danger-solid aiterm-btn--sm">{t.delete}?</button>
                  </>
                ) : (
                  <button onClick={() => setConfirmingDelete(conn.id)} className="aiterm-btn aiterm-btn--danger aiterm-btn--sm">{t.delete}</button>
                )}
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
                <label style={labelStyle}>{t.default_schema}</label>
                <input
                  placeholder={t.default_schema_placeholder}
                  value={form.default_schema ?? ""}
                  onChange={(e) => setForm((f) => ({ ...f, default_schema: e.target.value }))}
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
            <button onClick={handleTest} disabled={testStatus === "testing"} className="aiterm-btn aiterm-btn--secondary">
              {testStatus === "testing" ? t.testing : t.test_connection}
            </button>
            {testStatus === "ok" && <span style={{ color: "#34d399", fontSize: 12 }}>✓ {testMsg}</span>}
            {testStatus === "error" && <span style={{ color: "#f87171", fontSize: 12 }}>✗ {testMsg}</span>}
            <div style={{ flex: 1 }} />
            <button onClick={() => setShowForm(false)} className="aiterm-btn aiterm-btn--secondary">{t.cancel}</button>
            <button onClick={handleSave} disabled={saving} className="aiterm-btn aiterm-btn--primary">
              {saving ? t.saving_btn : t.save}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

const labelStyle: CSSProperties = { color: "#888", fontSize: 12 };
const inputStyle: CSSProperties = {
  background: "#0f0f0f", border: "1px solid #2a2a2a", color: "#e6e6e6",
  borderRadius: 4, padding: "6px 8px", fontSize: 13, width: "100%", boxSizing: "border-box",
};
