import { useEffect, useState } from "react";
import { dbConnect, dbListConnections, dbListSchemas } from "../../ipc/db";
import { ConnectionSelector } from "./ConnectionSelector";
import { DatabaseBrowser } from "./DatabaseBrowser";
import { DatabaseSqlEditor } from "./DatabaseSqlEditor";
import { DatabaseAiChat } from "./DatabaseAiChat";
import { useTelegramRemoteControl } from "../../hooks/useTelegramRemoteControl";
import { useLocale } from "../../contexts/LocaleContext";
import "./index.css";

export interface DatabaseViewProps {
  tabId: string;
  isActive: boolean;
  dbConnectionId?: string;
  onConnectionSelected: (connId: string) => void;
  /** 目前誰擁有 Telegram Remote（`TerminalApp` 的 `remoteTabId`）。null = 沒有人。 */
  remoteOwner: string | null;
  /** 使用者切換這個分頁的 Remote 開關時呼叫。 */
  onRemoteOwnerChange: (owner: string | null) => void;
}

type SubTab = "browse" | "ai" | "sql";

export function DatabaseView({ tabId, isActive: _isActive, dbConnectionId, onConnectionSelected, remoteOwner, onRemoteOwnerChange }: DatabaseViewProps) {
  const { t } = useLocale();
  const [subTab, setSubTab] = useState<SubTab>("browse");
  const [schemas, setSchemas] = useState<string[]>([]);
  const [activeSchema, setActiveSchema] = useState<string>("");
  const [connectError, setConnectError] = useState<string | null>(null);

  // ownerKey 用 tab.id，不是 dbConnectionId：後者在沒選連線前是 undefined，
  // 且换連線就變，跟這個分頁本身「是不是唯一 Remote 分頁」無關。
  const { isRemoteEnabled, toggleRemote, sendRemoteResponse } = useTelegramRemoteControl(
    tabId,
    remoteOwner,
    onRemoteOwnerChange,
    (text) => {
      setSubTab("ai");
      setTimeout(() => {
        window.dispatchEvent(new CustomEvent("aiterm:db-remote-msg", { detail: { text } }));
      }, 50);
    }
  );

  useEffect(() => {
    if (!dbConnectionId) return;
    let cancelled = false;
    dbConnect(dbConnectionId)
      .then(() => Promise.all([dbListSchemas(dbConnectionId), dbListConnections()]))
      .then(([s, connections]) => {
        if (cancelled) return;
        const preferredSchema = connections
          .find((c) => c.id === dbConnectionId)
          ?.default_schema
          ?.trim();
        const initialSchema =
          preferredSchema && s.includes(preferredSchema)
            ? preferredSchema
            : (s[0] ?? "");
        setSchemas(s);
        setActiveSchema(initialSchema);
        setConnectError(null);
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setConnectError(String(e));
      });

    return () => {
      cancelled = true;
    };
  }, [dbConnectionId]);

  if (!dbConnectionId) {
    return <ConnectionSelector onSelect={onConnectionSelected} />;
  }

  if (connectError) {
    const isOdbc = connectError.includes("odbc_driver_not_found");
    return (
      <div style={{ padding: 24, color: "#f87171", fontSize: 13 }}>
        {isOdbc ? (
          <>
            <div style={{ marginBottom: 8 }}>{t.db_odbc_missing_title}</div>
            <div style={{ color: "#888", fontSize: 12, marginBottom: 12 }}>
              {t.db_odbc_missing_desc}
            </div>
            <button
              onClick={() => setConnectError(null)}
              className="aiterm-btn aiterm-btn--secondary"
            >
              {t.db_btn_retry}
            </button>
          </>
        ) : (
          <>
            <div style={{ marginBottom: 8 }}>{t.db_connect_failed}</div>
            <div style={{ color: "#888", fontSize: 12 }}>{connectError}</div>
            <button
              onClick={() => { setConnectError(null); }}
              style={{ marginTop: 12 }}
              className="aiterm-btn aiterm-btn--secondary"
            >
              {t.db_btn_reconnect}
            </button>
          </>
        )}
      </div>
    );
  }

  return (
    <div className="db-view">
      <div className="db-view__subtabs">
        {([["browse", t.db_subtab_browse], ["ai", t.db_subtab_ai], ["sql", t.db_subtab_sql]] as [SubTab, string][]).map(([key, label]) => (
          <button
            key={key}
            className={`db-view__subtab ${subTab === key ? "db-view__subtab--active" : ""}`}
            onClick={() => setSubTab(key)}
          >
            {label}
          </button>
        ))}
        {dbConnectionId && (
          <button
            className={`db-view__subtab ${isRemoteEnabled ? "aiterm-agent-toggle--on" : ""}`}
            style={{ marginLeft: 8 }}
            onClick={toggleRemote}
            title={t.db_remote_tooltip}
          >
            📱 Remote
          </button>
        )}
        {schemas.length > 1 && (
          <select
            value={activeSchema}
            onChange={(e) => setActiveSchema(e.target.value)}
            style={{ marginLeft: "auto", background: "#1a1a1a", border: "1px solid #2a2a2a", color: "#888", fontSize: 11, borderRadius: 3, padding: "2px 6px" }}
          >
            {schemas.map((s) => <option key={s} value={s}>{s}</option>)}
          </select>
        )}
      </div>
      <div className="db-view__content">
        {dbConnectionId && (
          <>
            <div style={{ display: subTab === "browse" ? "contents" : "none" }}>
              <DatabaseBrowser connectionId={dbConnectionId} schema={activeSchema} />
            </div>
            <div style={{ display: subTab === "ai" ? "contents" : "none" }}>
              <DatabaseAiChat connectionId={dbConnectionId} schema={activeSchema} sendRemoteResponse={sendRemoteResponse} />
            </div>
            <div style={{ display: subTab === "sql" ? "contents" : "none" }}>
              <DatabaseSqlEditor connectionId={dbConnectionId} schema={activeSchema} />
            </div>
          </>
        )}
      </div>
    </div>
  );
}

