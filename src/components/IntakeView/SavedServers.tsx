import { useState } from "react";
import { confirm } from "@tauri-apps/plugin-dialog";
import { INTAKE_UNREACHABLE_PREFIX, intakeRemoteDisconnect, intakeRemoteForget, intakeRemoteSwitch, type IntakeRemoteStatus } from "../../ipc/intake";
import { useIntakeConnections } from "../../hooks/useIntakeRemote";
import { useLocale } from "../../contexts/LocaleContext";
import { ConnectForm } from "./ConnectionCard";

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/**
 * 已記錄的 server：一次只連一組，但可以記錄多組、一鍵切換，不用每次重新輸入邀請碼。
 * 切換時若有從 server 領來還在本機處理的工作，後端會拒絕並說明原因（顯示在這裡）。
 */
export function SavedServers({ status }: { status: IntakeRemoteStatus }) {
  const { t } = useLocale();
  const list = useIntakeConnections();
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);

  // 沒有任何記錄、也沒有要新增：不佔畫面（本機模式的第一次連線由上面的卡片處理）。
  if (list.length === 0) return null;

  async function run(id: string, fn: () => Promise<unknown>) {
    setBusyId(id);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusyId(null);
    }
  }

  /** 切換：對方連不上時先問使用者，確認才硬切（server 暫時關機、換網路都很常見）。 */
  const switchTo = async (c: { id: string; name: string; addr: string }) => {
    setBusyId(c.id);
    setError(null);
    try {
      try {
        await intakeRemoteSwitch(c.id);
      } catch (e) {
        const msg = errText(e);
        if (!msg.startsWith(INTAKE_UNREACHABLE_PREFIX)) throw e;
        const detail = msg.slice(INTAKE_UNREACHABLE_PREFIX.length);
        const ok = await confirm(t.intake_conn_unreachable_confirm.replace("{name}", `${c.name} · ${c.addr}`).replace("{detail}", detail));
        if (ok) await intakeRemoteSwitch(c.id, true);
      }
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusyId(null);
    }
  };

  async function disconnect() {
    setError(null);
    try {
      await intakeRemoteDisconnect();
    } catch (e) {
      setError(errText(e));
    }
  }

  const forget = async (id: string, label: string) => {
    // 要用 plugin-dialog 的非同步 confirm；Tauri 的 webview 沒有真正實作 window.confirm。
    if (!(await confirm(t.intake_conn_forget_confirm.replace("{name}", label)))) return;
    await run(id, () => intakeRemoteForget(id));
  };

  return (
    <section className="intake__card intake__conn" data-testid="saved-servers">
      <div className="intake__card-head">
        <h3 className="intake__card-title">{t.intake_conn_saved_title}</h3>
        {status.connected && (
          <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" aria-expanded={adding} onClick={() => setAdding((v) => !v)}>
            {adding ? t.intake_conn_add_cancel : t.intake_conn_add}
          </button>
        )}
      </div>
      <p className="intake__meta intake__hint">{t.intake_conn_saved_hint}</p>
      {status.connected && adding && (
        <div className="intake__saved-add">
          <p className="intake__meta intake__hint">{t.intake_conn_add_hint}</p>
          <ConnectForm onConnected={() => setAdding(false)} />
        </div>
      )}
      <ul className="intake__saved-list">
        {list.map((c) => {
          const label = `${c.name} · ${c.addr}`;
          return (
            <li key={c.id} className="intake__saved-row" data-testid={`saved-server-${c.id}`}>
              <span className="intake__saved-main">
                <strong>{c.name}</strong>
                <span className="intake__meta">
                  {" · "}
                  {c.role === "admin" ? t.intake_conn_role_admin : t.intake_conn_role_member}
                  {" · "}
                  {c.addr}
                </span>
              </span>
              {c.active ? (
                <span className={`intake__conn-state ${status.online ? "intake__conn-state--on" : "intake__conn-state--off"}`} role="status">
                  {status.online ? t.intake_conn_current : t.intake_conn_current_offline}
                </span>
              ) : (
                <button className="aiterm-btn aiterm-btn--secondary" disabled={busyId !== null} onClick={() => void switchTo(c)}>
                  {t.intake_conn_switch}
                </button>
              )}
              <button className="aiterm-btn aiterm-btn--ghost" disabled={busyId !== null} onClick={() => void forget(c.id, label)}>
                {t.intake_conn_forget}
              </button>
              {c.active && (
                <div className="intake__saved-detail">
                  {!status.online && <p className="intake__meta intake__warn">{t.intake_conn_offline}</p>}
                  {status.error && <div className="intake__error" role="alert">{status.error}</div>}
                  <p className="intake__meta intake__hint">{t.intake_conn_local_stopped}</p>
                  <button className="aiterm-btn aiterm-btn--secondary" onClick={() => void disconnect()}>
                    {t.intake_conn_disconnect}
                  </button>
                </div>
              )}
            </li>
          );
        })}
      </ul>
      {error && <div className="intake__error" role="alert">{error}</div>}
    </section>
  );
}
