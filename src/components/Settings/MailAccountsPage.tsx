import { useEffect, useRef, useState, type CSSProperties } from "react";
import { isPermissionGranted, requestPermission } from "@tauri-apps/plugin-notification";
import {
  mailAddAccount, mailListAccounts, mailRemoveAccount,
  type MailAccount, type MailAccountInput,
} from "../../ipc/mail";
import { useLocale } from "../../contexts/LocaleContext";

type FormState = MailAccountInput & { poll_interval_secs: number };

const EMPTY_FORM: FormState = {
  email: "", imap_host: "", imap_port: 993,
  smtp_host: "", smtp_port: 587,
  username: "", password: "", poll_interval_secs: 300,
};

/**
 * Raises the OS notification prompt at the one moment the user is provably
 * looking at the app. useMailSync would otherwise ask lazily, on the first
 * `important` mail — which arrives from a background poll timer while the app
 * is most likely unfocused, and macOS presents the authorization dialog in the
 * app's context: the user never sees it, `granted` stays false, and the very
 * first important mail silently notifies nothing.
 *
 * Deliberately fire-and-forget and never rethrows: the account was already
 * added successfully, so a permission problem must not surface as an
 * add-account failure.
 */
function promptForNotificationPermission() {
  isPermissionGranted()
    .then((granted) => { if (!granted) return requestPermission(); })
    .catch((err) => {
      console.error("[mail] notification permission request failed:", err);
    });
}

export function MailAccountsPage() {
  const { t } = useLocale();
  const [accounts, setAccounts] = useState<MailAccount[]>([]);
  // Guards the empty state: without it, "no accounts yet" flashes on every
  // mount before the fetch resolves. `loadFailed` matters just as much — a
  // failed fetch must not read as "you have no accounts".
  const [loading, setLoading] = useState(true);
  const [loadFailed, setLoadFailed] = useState(false);
  const [showForm, setShowForm] = useState(false);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState<string | null>(null);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  const load = () => {
    mailListAccounts().then((list) => {
      if (!mountedRef.current) return;
      setAccounts(list);
      setLoadFailed(false);
    }).catch((err) => {
      // Console logging alone is not enough: this app's Tauri window is hard
      // to attach devtools to, so the failure has to reach the UI.
      console.error("[mail] failed to list accounts:", err);
      if (mountedRef.current) setLoadFailed(true);
    }).finally(() => {
      if (mountedRef.current) setLoading(false);
    });
  };

  useEffect(() => { load(); }, []);

  const handleSave = async () => {
    setSaving(true);
    setError(null);
    try {
      await mailAddAccount(form);
      if (mountedRef.current) {
        setShowForm(false);
        setForm(EMPTY_FORM);
        load();
      }
      promptForNotificationPermission();
    } catch (err) {
      console.error("[mail] failed to add account:", err);
      // Form stays open with its values: retyping eight fields after a
      // transient IMAP failure would be miserable.
      if (mountedRef.current) setError(`${t.mail_add_failed}${String(err)}`);
    } finally {
      if (mountedRef.current) setSaving(false);
    }
  };

  const handleRemove = async (id: string) => {
    setError(null);
    try {
      await mailRemoveAccount(id);
      if (!mountedRef.current) return;
      setConfirmingDelete(null);
      load();
    } catch (err) {
      console.error("[mail] failed to remove account:", err);
      if (mountedRef.current) setError(`${t.mail_remove_failed}${String(err)}`);
    }
  };

  return (
    <div style={{ width: "100%", padding: "24px 32px", boxSizing: "border-box" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 16 }}>
        <h2 style={{ margin: 0, fontSize: 16, color: "#e6e6e6" }}>{t.mail_accounts_settings_title}</h2>
        {!showForm && (
          <button
            onClick={() => { setForm(EMPTY_FORM); setError(null); setShowForm(true); }}
            className="aiterm-btn aiterm-btn--primary"
          >
            {t.mail_add}
          </button>
        )}
      </div>

      {error && (
        <div style={{ color: "#f87171", fontSize: 12, marginBottom: 12 }}>{error}</div>
      )}

      {!showForm && (
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          {!loading && loadFailed && (
            <div style={{ color: "#f87171", fontSize: 13, padding: "20px 0" }}>{t.mail_load_failed}</div>
          )}
          {!loading && !loadFailed && accounts.length === 0 && (
            <div style={{ color: "#555", fontSize: 13, padding: "20px 0" }}>{t.mail_accounts_empty}</div>
          )}
          {accounts.map((acc) => (
            <div key={acc.id} style={{ background: "#1a1a1a", border: "1px solid #2a2a2a", borderRadius: 6, padding: "10px 14px", display: "flex", justifyContent: "space-between", alignItems: "center", minWidth: 0 }}>
              <div style={{ flex: 1, minWidth: 0, overflow: "hidden" }}>
                <div style={{ color: "#e6e6e6", fontSize: 13, fontWeight: 500, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{acc.email}</div>
                <div style={{ color: "#888", fontSize: 11, marginTop: 2, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                  IMAP {acc.imap_host}:{acc.imap_port} · SMTP {acc.smtp_host}:{acc.smtp_port} · {acc.poll_interval_secs}s
                </div>
              </div>
              <div style={{ display: "flex", gap: 8, alignItems: "center", flexShrink: 0, marginLeft: 12 }}>
                {confirmingDelete === acc.id ? (
                  <>
                    <button onClick={() => setConfirmingDelete(null)} className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm">{t.cancel}</button>
                    <button onClick={() => handleRemove(acc.id)} className="aiterm-btn aiterm-btn--danger-solid aiterm-btn--sm">{t.mail_remove}?</button>
                  </>
                ) : (
                  <button onClick={() => setConfirmingDelete(acc.id)} className="aiterm-btn aiterm-btn--danger aiterm-btn--sm">{t.mail_remove}</button>
                )}
              </div>
            </div>
          ))}
        </div>
      )}

      {showForm && (
        <div style={{ background: "#1a1a1a", border: "1px solid #2a2a2a", borderRadius: 8, padding: 20 }}>
          <h3 style={{ margin: "0 0 16px", fontSize: 14, color: "#e6e6e6" }}>{t.mail_add_form_title}</h3>
          {/* The sibling pages' <label>s are visual only (no htmlFor), so each
              input carries its own accessible name via aria-label. */}
          <div style={{ display: "grid", gridTemplateColumns: "150px 1fr", gap: "10px 12px", alignItems: "center" }}>
            <label style={labelStyle}>{t.mail_email}</label>
            <input
              aria-label={t.mail_email}
              placeholder="you@example.com"
              value={form.email}
              onChange={(e) => setForm((f) => ({ ...f, email: e.target.value }))}
              style={inputStyle}
            />

            <label style={labelStyle}>{t.mail_imap_host}</label>
            <input
              aria-label={t.mail_imap_host}
              placeholder="imap.example.com"
              value={form.imap_host}
              onChange={(e) => setForm((f) => ({ ...f, imap_host: e.target.value }))}
              style={inputStyle}
            />

            <label style={labelStyle}>{t.mail_imap_port}</label>
            <input
              aria-label={t.mail_imap_port}
              type="number"
              value={form.imap_port}
              onChange={(e) => setForm((f) => ({ ...f, imap_port: Number(e.target.value) }))}
              style={inputStyle}
            />

            <label style={labelStyle}>{t.mail_smtp_host}</label>
            <input
              aria-label={t.mail_smtp_host}
              placeholder="smtp.example.com"
              value={form.smtp_host}
              onChange={(e) => setForm((f) => ({ ...f, smtp_host: e.target.value }))}
              style={inputStyle}
            />

            <label style={labelStyle}>{t.mail_smtp_port}</label>
            <input
              aria-label={t.mail_smtp_port}
              type="number"
              value={form.smtp_port}
              onChange={(e) => setForm((f) => ({ ...f, smtp_port: Number(e.target.value) }))}
              style={inputStyle}
            />

            <label style={labelStyle}>{t.mail_username}</label>
            <input
              aria-label={t.mail_username}
              value={form.username}
              onChange={(e) => setForm((f) => ({ ...f, username: e.target.value }))}
              style={inputStyle}
            />

            <label style={labelStyle}>{t.mail_password}</label>
            <input
              aria-label={t.mail_password}
              type="password"
              value={form.password}
              onChange={(e) => setForm((f) => ({ ...f, password: e.target.value }))}
              style={inputStyle}
            />

            <label style={labelStyle}>{t.mail_poll_interval}</label>
            <input
              aria-label={t.mail_poll_interval}
              type="number"
              value={form.poll_interval_secs}
              onChange={(e) => setForm((f) => ({ ...f, poll_interval_secs: Number(e.target.value) }))}
              style={inputStyle}
            />
          </div>

          <div style={{ display: "flex", gap: 8, marginTop: 16, alignItems: "center", justifyContent: "flex-end" }}>
            <button onClick={() => setShowForm(false)} className="aiterm-btn aiterm-btn--secondary">{t.cancel}</button>
            <button onClick={handleSave} disabled={saving} className="aiterm-btn aiterm-btn--primary">
              {saving ? t.mail_saving : t.save}
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
