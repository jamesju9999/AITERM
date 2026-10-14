import { useCallback, useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import {
  computerAuditTail,
  computerOpenPrivacySettings,
  computerRevokeApp,
  computerSetEnabled,
  computerStatus,
  type ComputerAuditEntry,
  type ComputerStatus,
} from "../../ipc/computer";
import "./McpToolServerPage.css";
import "./ComputerUsePage.css";

function fmtTime(ts: number): string {
  return new Date(ts * 1000).toLocaleString();
}

/**
 * 「電腦操作」設定頁：總開關、權限狀態、已授權 App、稽核紀錄。
 * 開關同時管 Ask AI 與 MCP 工具伺服器兩條路，所以獨立成一頁、切換就存檔。
 * 版面沿用 McpToolServerPage.css 的通用樣式。
 */
export function ComputerUsePage() {
  const { t } = useLocale();
  const [status, setStatus] = useState<ComputerStatus | null>(null);
  const [audit, setAudit] = useState<ComputerAuditEntry[]>([]);

  const reload = useCallback(async () => {
    try {
      const [s, a] = await Promise.all([computerStatus(), computerAuditTail(20)]);
      setStatus(s);
      setAudit(a);
    } catch (e) {
      console.error("讀取電腦操作狀態失敗:", e);
    }
  }, []);

  useEffect(() => { void reload(); }, [reload]);

  const toggle = async (enabled: boolean) => {
    try {
      await computerSetEnabled(enabled);
    } catch (e) {
      console.error("儲存電腦操作開關失敗:", e);
    }
    await reload();
  };

  const revoke = async (key: string) => {
    await computerRevokeApp(key);
    await reload();
  };

  const perm = (ok: boolean, label: string, pane: "accessibility" | "screen") => (
    <div className="mcp-tool-server-row" key={pane}>
      <span>{label}：{ok ? t.computer_perm_granted : t.computer_perm_missing}</span>
      {!ok && (
        <button type="button" onClick={() => void computerOpenPrivacySettings(pane)}>
          {t.computer_perm_open}
        </button>
      )}
    </div>
  );

  return (
    <div className="mcp-tool-server-page">
      <h2>{t.computer_title}</h2>
      <p className="mcp-tool-server-desc">{t.computer_desc}</p>
      <p className="mcp-tool-server-desc">{t.computer_scope}</p>

      {status && (
        <>
          <section className="mcp-tool-server-section">
            <label className="mcp-tool-server-row">
              <input type="checkbox" checked={status.enabled} onChange={(e) => void toggle(e.target.checked)} />
              {t.computer_enable}
            </label>
            {perm(status.accessibility, t.computer_perm_accessibility, "accessibility")}
            {perm(status.screen_recording, t.computer_perm_screen, "screen")}
          </section>

          <h4>{t.computer_allowed_apps}</h4>
          {status.allowed_apps.length === 0 ? (
            <p className="mcp-tool-server-desc">{t.computer_allowed_none}</p>
          ) : (
            <ul className="computer-app-list">
              {status.allowed_apps.map((k) => (
                <li key={k} className="computer-app-row">
                  <span className="computer-app-name">{k === "*screen*" ? "整個螢幕" : k}</span>
                  <button type="button" onClick={() => void revoke(k)}>{t.computer_revoke}</button>
                </li>
              ))}
            </ul>
          )}

          <h4>{t.computer_audit}</h4>
          {audit.length === 0 ? (
            <p className="mcp-tool-server-desc">{t.computer_audit_none}</p>
          ) : (
            <ul className="computer-audit-list">
              {audit.map((a, i) => {
                const err = a.outcome.startsWith("error: ") ? a.outcome.slice("error: ".length) : null;
                const kind = err !== null ? "error" : a.outcome === "ok" ? "ok" : "warn";
                return (
                  <li key={`${a.ts}-${i}`} className="computer-audit-row">
                    <span className="computer-audit-time">{fmtTime(a.ts)}</span>
                    <span className="computer-audit-tool">{a.tool.replace("computer_", "")}</span>
                    <span className="computer-audit-app">{a.app || "-"}</span>
                    <span className="computer-audit-detail" title={a.detail}>{a.detail}</span>
                    <span className={`computer-audit-outcome computer-audit-outcome--${kind}`}>
                      {err !== null ? "error" : a.outcome}
                    </span>
                    {err !== null && <span className="computer-audit-error" title={err}>{err}</span>}
                  </li>
                );
              })}
            </ul>
          )}
        </>
      )}
    </div>
  );
}
