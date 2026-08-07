import { useState, type CSSProperties } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { dbExportConnections, DB_TYPE_LABELS, type DbConnectionInfo } from "../../ipc/db";
import { useLocale } from "../../contexts/LocaleContext";
import type { Translations } from "../../lib/i18n";

/** 後端 `ImportError` 的變體名稱 → i18n key。這些字串是介面契約的一部分，
 *  改動時要同步 `src-tauri/src/commands/db_export.rs` 的 `#[error(...)]`。 */
const ERROR_KEYS = {
  not_an_export_file: "db_err_not_an_export_file",
  unsupported_version: "db_err_unsupported_version",
  wrong_passphrase: "db_err_wrong_passphrase",
  unsupported_kdf: "db_err_unsupported_kdf",
} as const;

/** 已知錯誤碼轉成本地化訊息；其餘（例如 `io_error: ...`）原樣顯示。 */
export function translateDbTransferError(t: Translations, e: unknown): string {
  const raw = String(e);
  const key = ERROR_KEYS[raw as keyof typeof ERROR_KEYS];
  return key ? (t[key] as string) : raw;
}

const DEFAULT_EXPORT_NAME = "aiterm-db-connections.json";

export function DbExportPanel({
  connections, onClose, onDone,
}: {
  connections: DbConnectionInfo[];
  onClose: () => void;
  onDone: (message: string) => void;
}) {
  const { t } = useLocale();
  const [selected, setSelected] = useState<Set<string>>(
    () => new Set(connections.map((c) => c.id)),
  );
  const [passphrase, setPassphrase] = useState("");
  const [confirmPass, setConfirmPass] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const mismatch = confirmPass.length > 0 && passphrase !== confirmPass;
  const canExport =
    selected.size > 0 && passphrase.length > 0 && passphrase === confirmPass && !busy;

  const toggle = (id: string) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const handleExport = async () => {
    const path = await save({
      defaultPath: DEFAULT_EXPORT_NAME,
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (!path) return; // 使用者取消對話框——不呼叫任何 IPC
    setBusy(true);
    setError("");
    try {
      const n = await dbExportConnections(path, [...selected], passphrase);
      onDone(t.db_export_done(n));
    } catch (e) {
      setError(translateDbTransferError(t, e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div style={panelStyle}>
      <h3 style={headingStyle}>{t.db_export_title}</h3>

      <div style={{ display: "flex", flexDirection: "column", gap: 6, marginBottom: 16 }}>
        {connections.map((c) => (
          <label key={c.id} style={rowStyle}>
            <input
              type="checkbox"
              checked={selected.has(c.id)}
              onChange={() => toggle(c.id)}
            />
            <span style={{ flex: 1, minWidth: 0 }}>
              <span style={{ color: "#e6e6e6", fontSize: 13 }}>{c.name}</span>
              <span style={{ color: "#888", fontSize: 11, marginLeft: 8 }}>
                {DB_TYPE_LABELS[c.db_type]} · {c.host}:{c.port}
              </span>
            </span>
          </label>
        ))}
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "120px 1fr", gap: "10px 12px", alignItems: "center" }}>
        <label style={labelStyle} htmlFor="db-export-pass">{t.db_transfer_passphrase}</label>
        <input
          id="db-export-pass"
          type="password"
          value={passphrase}
          onChange={(e) => setPassphrase(e.target.value)}
          style={inputStyle}
        />
        <label style={labelStyle} htmlFor="db-export-pass2">{t.db_transfer_passphrase_confirm}</label>
        <input
          id="db-export-pass2"
          type="password"
          value={confirmPass}
          onChange={(e) => setConfirmPass(e.target.value)}
          style={inputStyle}
        />
      </div>

      <div style={{ color: "#888", fontSize: 11, marginTop: 8 }}>{t.db_transfer_passphrase_hint}</div>
      {mismatch && <div style={errorStyle}>{t.db_transfer_passphrase_mismatch}</div>}
      {error && <div style={errorStyle}>{error}</div>}

      <div style={{ display: "flex", gap: 8, marginTop: 16, justifyContent: "flex-end" }}>
        <button onClick={onClose} className="aiterm-btn aiterm-btn--secondary">{t.cancel}</button>
        <button onClick={handleExport} disabled={!canExport} className="aiterm-btn aiterm-btn--primary">
          {t.db_export}
        </button>
      </div>
    </div>
  );
}

const panelStyle: CSSProperties = {
  background: "#1a1a1a", border: "1px solid #2a2a2a", borderRadius: 8, padding: 20,
};
const headingStyle: CSSProperties = { margin: "0 0 16px", fontSize: 14, color: "#e6e6e6" };
const rowStyle: CSSProperties = {
  display: "flex", alignItems: "center", gap: 10, padding: "8px 10px",
  background: "#141414", border: "1px solid #2a2a2a", borderRadius: 5, cursor: "pointer",
};
const labelStyle: CSSProperties = { color: "#888", fontSize: 12 };
const inputStyle: CSSProperties = {
  background: "#0f0f0f", border: "1px solid #2a2a2a", color: "#e6e6e6",
  borderRadius: 4, padding: "6px 8px", fontSize: 13, width: "100%", boxSizing: "border-box",
};
const errorStyle: CSSProperties = { color: "#f87171", fontSize: 12, marginTop: 8 };
