import { useEffect, useState, type CSSProperties } from "react";
import {
  vcsListConnections, vcsAddConnection, vcsUpdateConnection, vcsRemoveConnection,
  vcsTestConnection,
  type VcsConnectionInfo, type VcsConnectionInput, type VcsType, type VcsWriteMode,
} from "../../ipc/vcs";

type FormState = Omit<VcsConnectionInput, "id"> & { id?: string };

const EMPTY_FORM: FormState = {
  name: "", vcs_type: "git", url: "", username: "", secret: "", write_mode: "read_only",
};

const VCS_TYPE_LABELS: Record<VcsType, string> = {
  git: "Git",
  svn: "SVN",
};

const WRITE_MODE_LABELS: Record<VcsWriteMode, string> = {
  read_only: "唯讀 (Read Only)",
  guarded: "保護 (Guarded)",
  full_auto: "全自動 (Full Auto)",
};

export function VcsConnectionsPage() {
  const [connections, setConnections] = useState<VcsConnectionInfo[]>([]);
  const [showForm, setShowForm] = useState(false);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [testStatus, setTestStatus] = useState<"idle" | "testing" | "ok" | "error">("idle");
  const [testMsg, setTestMsg] = useState("");
  const [saving, setSaving] = useState(false);
  const [confirmingDelete, setConfirmingDelete] = useState<string | null>(null);

  const load = () => vcsListConnections().then(setConnections).catch(console.error);
  useEffect(() => { load(); }, []);

  const handleTest = async () => {
    setTestStatus("testing");
    setTestMsg("");
    try {
      const msg = await vcsTestConnection({ ...form });
      setTestStatus("ok");
      setTestMsg(msg);
    } catch (e: unknown) {
      setTestStatus("error");
      setTestMsg(String(e));
    }
  };

  const handleSave = async () => {
    setSaving(true);
    try {
      if (form.id) {
        await vcsUpdateConnection({ ...form, id: form.id });
      } else {
        await vcsAddConnection(form as VcsConnectionInput);
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

  const handleEdit = (conn: VcsConnectionInfo) => {
    setForm({
      id: conn.id,
      name: conn.name,
      vcs_type: conn.vcs_type,
      url: conn.url ?? "",
      username: conn.username ?? "",
      secret: "",
      write_mode: conn.write_mode,
    });
    setShowForm(true);
    setTestStatus("idle");
  };

  const handleDelete = async (id: string) => {
    await vcsRemoveConnection(id);
    setConfirmingDelete(null);
    load();
  };

  const isGit = form.vcs_type === "git";
  const urlLabel = isGit ? "GitHub Repo URL" : "SVN Repo URL";
  const secretLabel = isGit ? "GitHub Token" : "Password";

  return (
    <div style={{ width: "100%", padding: "24px 32px", boxSizing: "border-box" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 16 }}>
        <h2 style={{ margin: 0, fontSize: 16, color: "#e6e6e6" }}>VCS 連線</h2>
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
            <div style={{ color: "#555", fontSize: 13, padding: "20px 0" }}>尚無 VCS 連線。點擊「+ 新增連線」開始。</div>
          )}
          {connections.map((conn) => (
            <div key={conn.id} style={{ background: "#1a1a1a", border: "1px solid #2a2a2a", borderRadius: 6, padding: "10px 14px", display: "flex", justifyContent: "space-between", alignItems: "center", minWidth: 0 }}>
              <div style={{ flex: 1, minWidth: 0, overflow: "hidden" }}>
                <div style={{ color: "#e6e6e6", fontSize: 13, fontWeight: 500, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{conn.name}</div>
                <div style={{ color: "#888", fontSize: 11, marginTop: 2, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                  {VCS_TYPE_LABELS[conn.vcs_type]} · {WRITE_MODE_LABELS[conn.write_mode]}
                  {conn.url ? ` · ${conn.url}` : ""}
                  {conn.has_secret ? " · 🔑 token 已設定" : ""}
                </div>
              </div>
              <div style={{ display: "flex", gap: 8, alignItems: "center", flexShrink: 0, marginLeft: 12 }}>
                <button onClick={() => handleEdit(conn)} style={btnStyle}>編輯</button>
                {confirmingDelete === conn.id ? (
                  <>
                    <button onClick={() => setConfirmingDelete(null)} style={btnStyle}>取消</button>
                    <button onClick={() => handleDelete(conn.id)} style={{ ...btnStyle, color: "#f87171", borderColor: "#f87171" }}>刪除?</button>
                  </>
                ) : (
                  <button onClick={() => setConfirmingDelete(conn.id)} style={{ ...btnStyle, color: "#f87171", borderColor: "#f87171" }}>刪除</button>
                )}
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
          <div style={{ display: "grid", gridTemplateColumns: "130px 1fr", gap: "10px 12px", alignItems: "center" }}>
            <label style={labelStyle}>名稱</label>
            <input
              value={form.name}
              onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))}
              style={inputStyle}
            />

            <label style={labelStyle}>類型</label>
            <select
              value={form.vcs_type}
              onChange={(e) => setForm((f) => ({ ...f, vcs_type: e.target.value as VcsType }))}
              style={inputStyle}
            >
              {(Object.entries(VCS_TYPE_LABELS) as [VcsType, string][]).map(([k, v]) => (
                <option key={k} value={k}>{v}</option>
              ))}
            </select>

            <label style={labelStyle}>{urlLabel}</label>
            <input
              placeholder={isGit ? "https://github.com/owner/repo" : "svn://host/repo"}
              value={form.url ?? ""}
              onChange={(e) => setForm((f) => ({ ...f, url: e.target.value }))}
              style={inputStyle}
            />

            {!isGit && (
              <>
                <label style={labelStyle}>Username</label>
                <input
                  value={form.username ?? ""}
                  onChange={(e) => setForm((f) => ({ ...f, username: e.target.value }))}
                  style={inputStyle}
                />
              </>
            )}

            <label style={labelStyle}>{secretLabel}</label>
            <input
              type="password"
              placeholder={form.id ? "留空則不更新" : ""}
              value={form.secret ?? ""}
              onChange={(e) => setForm((f) => ({ ...f, secret: e.target.value }))}
              style={inputStyle}
            />

            <label style={labelStyle}>寫入模式</label>
            <select
              value={form.write_mode}
              onChange={(e) => setForm((f) => ({ ...f, write_mode: e.target.value as VcsWriteMode }))}
              style={inputStyle}
            >
              {(Object.entries(WRITE_MODE_LABELS) as [VcsWriteMode, string][]).map(([k, v]) => (
                <option key={k} value={k}>{v}</option>
              ))}
            </select>
          </div>

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
