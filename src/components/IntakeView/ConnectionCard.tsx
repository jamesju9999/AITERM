import { useState } from "react";
import { intakeRemoteDisconnect, intakeRemotePair, type IntakeRemoteStatus } from "../../ipc/intake";
import { useLocale } from "../../contexts/LocaleContext";
import { validInviteCode, validServerAddr } from "../../lib/intakeServerConn";

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export function ConnectionCard({ status }: { status: IntakeRemoteStatus }) {
  const { t } = useLocale();
  const [addr, setAddr] = useState("");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const addrOk = validServerAddr(addr);
  const codeOk = validInviteCode(code);

  async function connect() {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await intakeRemotePair(addr.trim(), code.trim());
      setCode("");
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  }

  async function disconnect() {
    setError(null);
    try {
      await intakeRemoteDisconnect();
    } catch (e) {
      setError(errText(e));
    }
  }

  if (status.connected) {
    return (
      <section className="intake__card intake__conn">
        <h3 className="intake__card-title">{t.intake_conn_title}</h3>
        <p className="intake__conn-line">
          <strong>{status.name}</strong>
          <span className="intake__meta">
            {" · "}
            {status.role === "admin" ? t.intake_conn_role_admin : t.intake_conn_role_member}
            {" · "}
            {status.addr}
          </span>
        </p>
        <p className={`intake__conn-state ${status.online ? "intake__conn-state--on" : "intake__conn-state--off"}`} role="status">
          {status.online ? t.intake_conn_online : t.intake_conn_offline}
        </p>
        {status.error && <div className="intake__error" role="alert">{status.error}</div>}
        <p className="intake__meta intake__hint">{t.intake_conn_local_stopped}</p>
        {error && <div className="intake__error" role="alert">{error}</div>}
        <div className="intake__actions">
          <button className="aiterm-btn aiterm-btn--secondary" onClick={() => void disconnect()}>
            {t.intake_conn_disconnect}
          </button>
        </div>
      </section>
    );
  }

  return (
    <section className="intake__card intake__conn">
      <h3 className="intake__card-title">{t.intake_conn_title}</h3>
      <p className="intake__meta intake__hint">{t.intake_conn_intro}</p>
      <label className="intake__field">
        <span>{t.intake_conn_addr}</span>
        <input aria-label={t.intake_conn_addr} value={addr} placeholder="192.168.1.20:8443" onChange={(e) => setAddr(e.target.value)} disabled={busy} />
      </label>
      {addr.trim() !== "" && !addrOk && <p className="intake__meta intake__warn" role="alert">{t.intake_conn_addr_bad}</p>}
      <label className="intake__field">
        <span>{t.intake_conn_code}</span>
        <input aria-label={t.intake_conn_code} value={code} placeholder="XXXX-XXXX-XXXX-XXXX-XXXX-XX" onChange={(e) => setCode(e.target.value)} disabled={busy} autoComplete="off" />
      </label>
      {code.trim() !== "" && !codeOk && <p className="intake__meta intake__warn" role="alert">{t.intake_conn_code_bad}</p>}
      {error && <div className="intake__error" role="alert">{error}</div>}
      <div className="intake__actions">
        <button className="aiterm-btn aiterm-btn--primary" disabled={busy || !addrOk || !codeOk} onClick={() => void connect()}>
          {busy ? t.intake_conn_connecting : t.intake_conn_connect}
        </button>
      </div>
    </section>
  );
}
