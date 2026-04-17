import { useEffect, useState, type CSSProperties } from "react";
import {
  dbListConnections, dbAddConnection, dbUpdateConnection, dbRemoveConnection,
  dbTestConnection, DbConnectionInfo, DbConnectionInput, DbType,
  DB_TYPE_LABELS, DB_DEFAULT_PORTS,
} from "../../ipc/db";

type FormState = Omit<DbConnectionInput, "id"> & { id?: string };

const EMPTY_FORM: FormState = {
  name: "", db_type: "postgresql", host: "localhost",
  port: 5432, database: "", username: "", password: "",
};

export function DatabaseConnectionsPage() {
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
      setTestMsg("連線成功");
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
    if (!confirm("確定刪除此連線？")) return;
    await dbRemoveConnection(id);
    load();
  };

  return (
    <div style={{ maxWidth: 600 }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 16 }}>
        <h2 style={{ margin: 0, fontSize: 16, color: "#e6e6e6" }}>資料庫連線</h2>
        {!showForm && (
          <button
            onClick={() => { setForm(EMPTY_FORM); setShowForm(true); setTestStatus("idle"); }}
            style={{ background: "#1e3a2e", border: "1px solid #34d399", color: "#34d399", borderRadius: 5, padding: "6px 14px", cursor: "pointer", fontSize: 13 }}
          >
            + 新增連線
          </button>
        )}
      </div>

      {!showForm && (
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          {connections.length === 0 && (
            <div style={{ color: "#555", fontSize: 13, padding: "20px 0" }}>尚無資料庫連線。點擊「+ 新增連線」開始。</div>
          )}
          {connections.map((conn) => (
            <div key={conn.id} style={{ background: "#1a1a1a", border: "1px solid #2a2a2a", borderRadius: 6, padding: "10px 14px", display: "flex", justifyContent: "space-between", alignItems: "center" }}>
              <div>
                <div style={{ color: "#e6e6e6", fontSize: 13, fontWeight: 500 }}>{conn.name}</div>
                <div style={{ color: "#888", fontSize: 11, marginTop: 2 }}>
                  {DB_TYPE_LABELS[conn.db_type]} · {conn.host}:{conn.port} / {conn.database}
                </div>
              </div>
              <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
                {conn.is_connected && <span style={{ color: "#34d399", fontSize: 11 }}>● 已連線</span>}
                <button onClick={() => handleEdit(conn)} style={btnStyle}>編輯</button>
                <button onClick={() => handleDelete(conn.id)} style={{ ...btnStyle, color: "#f87171", borderColor: "#f87171" }}>刪除</button>
              </div>
            </div>
          ))}
        </div>
      )}

      {showForm && (
        <div style={{ background: "#1a1a1a", border: "1px solid #2a2a2a", borderRadius: 8, padding: 20 }}>
          <h3 style={{ margin: "0 0 16px", fontSize: 14, color: "#e6e6e6" }}>
            {form.id ? "編輯連線" : "新增連線"}
          </h3>
          <div style={{ display: "grid", gridTemplateColumns: "120px 1fr", gap: "10px 12px", alignItems: "center" }}>
            <label style={labelStyle}>名稱</label>
            <input
              placeholder="我的資料庫"
              value={form.name}
              onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))}
              style={inputStyle}
            />
            <label style={labelStyle}>類型</label>
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
                <label style={labelStyle}>檔案路徑</label>
                <input
                  placeholder="/path/to/database.db"
                  value={form.host}
                  onChange={(e) => setForm((f) => ({ ...f, host: e.target.value }))}
                  style={inputStyle}
                />
              </>
            ) : (
              <>
                <label style={labelStyle}>Host</label>
                <input
                  placeholder="localhost"
                  value={form.host}
                  onChange={(e) => setForm((f) => ({ ...f, host: e.target.value }))}
                  style={inputStyle}
                />
                <label style={labelStyle}>Port</label>
                <input
                  type="number"
                  value={form.port}
                  onChange={(e) => setForm((f) => ({ ...f, port: Number(e.target.value) }))}
                  style={inputStyle}
                />
                <label style={labelStyle}>Database</label>
                <input
                  value={form.database}
                  onChange={(e) => setForm((f) => ({ ...f, database: e.target.value }))}
                  style={inputStyle}
                />
                <label style={labelStyle}>Username</label>
                <input
                  value={form.username}
                  onChange={(e) => setForm((f) => ({ ...f, username: e.target.value }))}
                  style={inputStyle}
                />
                <label style={labelStyle}>Password</label>
                <input
                  type="password"
                  placeholder={form.id ? "留空表示不變更" : ""}
                  value={form.password}
                  onChange={(e) => setForm((f) => ({ ...f, password: e.target.value }))}
                  style={inputStyle}
                />
              </>
            )}
          </div>

          {form.db_type === "db2" && (
            <div style={{ background: "#2a1a00", border: "1px solid #f9a825", borderRadius: 5, padding: "10px 14px", marginTop: 12, fontSize: 12, color: "#f9a825" }}>
              ⚠️ DB2 需要 IBM DB2 ODBC Driver。Windows / macOS: 安裝 IBM Data Server Driver Package。
              <br />Host 欄位請填寫 DSN 名稱或完整 ODBC 連線字串。
            </div>
          )}

          <div style={{ display: "flex", gap: 8, marginTop: 16, alignItems: "center" }}>
            <button onClick={handleTest} disabled={testStatus === "testing"} style={btnStyle}>
              {testStatus === "testing" ? "測試中..." : "測試連線"}
            </button>
            {testStatus === "ok" && <span style={{ color: "#34d399", fontSize: 12 }}>✓ {testMsg}</span>}
            {testStatus === "error" && <span style={{ color: "#f87171", fontSize: 12 }}>✗ {testMsg}</span>}
            <div style={{ flex: 1 }} />
            <button onClick={() => setShowForm(false)} style={btnStyle}>取消</button>
            <button onClick={handleSave} disabled={saving} style={{ ...btnStyle, background: "#1e3a2e", borderColor: "#34d399", color: "#34d399" }}>
              {saving ? "儲存中..." : "儲存"}
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
