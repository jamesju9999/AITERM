import { useEffect, useState, type CSSProperties } from "react";
import {
  vcsListConnections, vcsAddConnection, vcsUpdateConnection, vcsRemoveConnection,
  vcsTestConnection,
  type VcsConnectionInfo, type VcsConnectionInput, type VcsType, type VcsWriteMode,
} from "../../ipc/vcs";
import { useLocale } from "../../contexts/LocaleContext";

type FormState = Omit<VcsConnectionInput, "id"> & { id?: string };

const EMPTY_FORM: FormState = {
  name: "", vcs_type: "git", url: "", username: "", secret: "", write_mode: "read_only",
};

const VCS_TYPE_LABELS: Record<VcsType, string> = {
  git: "Git",
  svn: "SVN",
};

export function VcsConnectionsPage() {
  const { t } = useLocale();
  const [connections, setConnections] = useState<VcsConnectionInfo[]>([]);
  const [showForm, setShowForm] = useState(false);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [testStatus, setTestStatus] = useState<"idle" | "testing" | "ok" | "error">("idle");
  const [testMsg, setTestMsg] = useState("");
  const [saving, setSaving] = useState(false);
  const [confirmingDelete, setConfirmingDelete] = useState<string | null>(null);

  const load = () => vcsListConnections().then(setConnections).catch(console.error);
  useEffect(() => { load(); }, []);

  const WRITE_MODE_LABELS: Record<VcsWriteMode, string> = {
    read_only: t.vcs_write_readonly,
    guarded: t.vcs_write_guarded,
    full_auto: t.vcs_write_fullauto,
  };

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
  const urlLabel = isGit ? t.vcs_github_url_label : t.vcs_svn_url_label;
  const secretLabel = isGit ? t.vcs_github_token_label : t.vcs_svn_password_label;

  return (
    <div style={{ width: "100%", padding: "24px 32px", boxSizing: "border-box" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 16 }}>
        <h2 style={{ margin: 0, fontSize: 16, color: "#e6e6e6" }}>{t.vcs_page_title}</h2>
        {!showForm && (
          <button
            onClick={() => { setForm(EMPTY_FORM); setShowForm(true); setTestStatus("idle"); }}
            className="aiterm-btn aiterm-btn--primary"
          >
            {t.vcs_add_conn}
          </button>
        )}
      </div>

      {!showForm && (
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          {connections.length === 0 && (
            <div style={{ color: "#555", fontSize: 13, padding: "20px 0" }}>{t.vcs_empty_state}</div>
          )}
          {connections.map((conn) => (
            <div key={conn.id} style={{ background: "#1a1a1a", border: "1px solid #2a2a2a", borderRadius: 6, padding: "10px 14px", display: "flex", justifyContent: "space-between", alignItems: "center", minWidth: 0 }}>
              <div style={{ flex: 1, minWidth: 0, overflow: "hidden" }}>
                <div style={{ color: "#e6e6e6", fontSize: 13, fontWeight: 500, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{conn.name}</div>
                <div style={{ color: "#888", fontSize: 11, marginTop: 2, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                  {VCS_TYPE_LABELS[conn.vcs_type]} · {WRITE_MODE_LABELS[conn.write_mode]}
                  {conn.url ? ` · ${conn.url}` : ""}
                  {conn.has_secret ? ` · ${t.vcs_token_set}` : ""}
                </div>
              </div>
              <div style={{ display: "flex", gap: 8, alignItems: "center", flexShrink: 0, marginLeft: 12 }}>
                <button onClick={() => handleEdit(conn)} className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm">{t.vcs_edit_btn}</button>
                {confirmingDelete === conn.id ? (
                  <>
                    <button onClick={() => setConfirmingDelete(null)} className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm">{t.cancel}</button>
                    <button onClick={() => handleDelete(conn.id)} className="aiterm-btn aiterm-btn--danger-solid aiterm-btn--sm">{t.vcs_delete_confirm_btn}</button>
                  </>
                ) : (
                  <button onClick={() => setConfirmingDelete(conn.id)} className="aiterm-btn aiterm-btn--danger aiterm-btn--sm">{t.vcs_delete_btn}</button>
                )}
              </div>
            </div>
          ))}
        </div>
      )}

      {showForm && (
        <div style={{ background: "#1a1a1a", border: "1px solid #2a2a2a", borderRadius: 8, padding: 20 }}>
          <h3 style={{ margin: "0 0 16px", fontSize: 14, color: "#e6e6e6" }}>
            {form.id ? t.vcs_form_title_edit : t.vcs_form_title_add}
          </h3>
          <div style={{ display: "grid", gridTemplateColumns: "130px 1fr", gap: "10px 12px", alignItems: "center" }}>
            <label style={labelStyle}>{t.vcs_form_name}</label>
            <input
              value={form.name}
              onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))}
              style={inputStyle}
            />

            <label style={labelStyle}>{t.vcs_form_type}</label>
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
                <label style={labelStyle}>{t.vcs_form_username}</label>
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
              placeholder={form.id ? t.vcs_form_password_hint : ""}
              value={form.secret ?? ""}
              onChange={(e) => setForm((f) => ({ ...f, secret: e.target.value }))}
              style={inputStyle}
            />

            <label style={labelStyle}>{t.vcs_form_write_mode}</label>
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
            <button onClick={handleTest} disabled={testStatus === "testing"} className="aiterm-btn aiterm-btn--secondary">
              {testStatus === "testing" ? t.vcs_form_testing : t.vcs_form_test}
            </button>
            {testStatus === "ok" && <span style={{ color: "#34d399", fontSize: 12 }}>✓ {testMsg}</span>}
            {testStatus === "error" && <span style={{ color: "#f87171", fontSize: 12 }}>✗ {testMsg}</span>}
            <div style={{ flex: 1 }} />
            <button onClick={() => setShowForm(false)} className="aiterm-btn aiterm-btn--secondary">{t.cancel}</button>
            <button onClick={handleSave} disabled={saving} className="aiterm-btn aiterm-btn--primary">
              {saving ? t.vcs_form_saving : t.vcs_form_save}
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

