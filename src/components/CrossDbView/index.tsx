import { useState, useEffect } from "react";
import { dbConnect, dbListConnections, dbListSchemas, dbListTables, type DbConnectionInfo, type TableInfo } from "../../ipc/db";
import { CrossDbAiChat } from "./CrossDbAiChat";
import { CrossDbSqlEditor } from "./CrossDbSqlEditor";
import { useTelegramRemoteControl } from "../../hooks/useTelegramRemoteControl";
import { useLocale } from "../../contexts/LocaleContext";
import "./CrossDbView.css";

export interface CrossDbViewProps {
  /** 這個分頁的穩定識別碼（`tab.id`），當作 Telegram Remote 的 ownerKey。 */
  tabId: string;
  /** 目前誰擁有 Telegram Remote（`TerminalApp` 的 `remoteTabId`）。null = 沒有人。 */
  remoteOwner: string | null;
  /** 使用者切換這個分頁的 Remote 開關時呼叫。 */
  onRemoteOwnerChange: (owner: string | null) => void;
}

export interface ConnectedDb {
  id: string;
  name: string;
  db_type: string;
  database: string;
  schema: string;
  tables: TableInfo[];
}

type SubTab = "ai" | "sql";

export function CrossDbView({ tabId, remoteOwner, onRemoteOwnerChange }: CrossDbViewProps) {
  const { t } = useLocale();
  const [allConnections, setAllConnections] = useState<DbConnectionInfo[]>([]);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [connectedDbs, setConnectedDbs] = useState<ConnectedDb[]>([]);
  const [loading, setLoading] = useState(false);
  const [started, setStarted] = useState(false);
  const [subTab, setSubTab] = useState<SubTab>("ai");

  // ownerKey 用 tab.id，不是寫死的 "cross-db"：那個字串不保證唯一，兩個
  // cross-db 分頁會撞。
  const { isRemoteEnabled, toggleRemote, sendRemoteResponse } = useTelegramRemoteControl(
    tabId,
    remoteOwner,
    onRemoteOwnerChange,
    (text) => {
      // Forward Telegram messages to the AI Chat
      window.dispatchEvent(new CustomEvent("aiterm:crossdb-remote-msg", { detail: { text } }));
    }
  );

  useEffect(() => {
    dbListConnections().then(setAllConnections).catch(console.error);
  }, []);

  const toggleSelection = (id: string) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const startSession = async () => {
    if (selectedIds.size < 1) return;
    setLoading(true);
    const dbs: ConnectedDb[] = [];

    for (const id of selectedIds) {
      const conn = allConnections.find((c) => c.id === id);
      if (!conn) continue;
      try {
        await dbConnect(id);
        const schemas = await dbListSchemas(id);
        const schema = conn.default_schema?.trim() || schemas[0] || "";
        const tables = schema ? await dbListTables(id, schema) : [];
        dbs.push({
          id: conn.id,
          name: conn.name,
          db_type: conn.db_type,
          database: conn.database,
          schema,
          tables,
        });
      } catch (e) {
        console.error(`Failed to connect ${conn.name}:`, e);
      }
    }

    setConnectedDbs(dbs);
    setStarted(true);
    setLoading(false);
  };

  if (!started) {
    return (
      <div className="crossdb-view">
        <div className="crossdb-selector">
          <div className="crossdb-selector__header">
            <h3>{t.cdb_welcome_title}</h3>
            <p>{t.cdb_welcome_desc}</p>
          </div>
          {allConnections.length === 0 ? (
            <div className="crossdb-selector__empty">
              {t.cdb_welcome_empty}
            </div>
          ) : (
            <div className="crossdb-selector__list">
              {allConnections.map((conn) => (
                <label key={conn.id} className="crossdb-selector__item">
                  <input
                    type="checkbox"
                    checked={selectedIds.has(conn.id)}
                    onChange={() => toggleSelection(conn.id)}
                  />
                  <span className="crossdb-selector__db-type">{conn.db_type.toUpperCase()}</span>
                  <span className="crossdb-selector__name">{conn.name}</span>
                  <span className="crossdb-selector__detail">{conn.database}@{conn.host}:{conn.port}</span>
                </label>
              ))}
            </div>
          )}
          <button
            className="crossdb-selector__start aiterm-btn aiterm-btn--primary"
            disabled={selectedIds.size < 1 || loading}
            onClick={startSession}
          >
            {loading ? t.cdb_btn_connect : t.cdb_btn_start(selectedIds.size)}
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="crossdb-view">
      <div className="crossdb-subtabs">
        <button
          className={`crossdb-subtab ${subTab === "ai" ? "crossdb-subtab--active" : ""}`}
          onClick={() => setSubTab("ai")}
        >
          {t.db_subtab_ai}
        </button>
        <button
          className={`crossdb-subtab ${subTab === "sql" ? "crossdb-subtab--active" : ""}`}
          onClick={() => setSubTab("sql")}
        >
          {t.db_subtab_sql}
        </button>
        <button
          className={`crossdb-subtab ${isRemoteEnabled ? "crossdb-subtab--active" : ""}`}
          style={{ marginLeft: 8 }}
          onClick={toggleRemote}
          title={t.db_remote_tooltip}
        >
          📱 Remote
        </button>
        <div className="crossdb-db-tags">
          {connectedDbs.map((db) => (
            <span key={db.id} className="crossdb-db-tag">
              {db.name} ({db.db_type})
            </span>
          ))}
        </div>
        <button
          className="crossdb-subtab"
          style={{ marginLeft: "auto" }}
          onClick={() => { setStarted(false); setConnectedDbs([]); }}
        >
          {t.cdb_btn_reselect}
        </button>
      </div>
      <div className="crossdb-content">
        <div style={{ display: subTab === "ai" ? "contents" : "none" }}>
          <CrossDbAiChat databases={connectedDbs} sendRemoteResponse={sendRemoteResponse} />
        </div>
        <div style={{ display: subTab === "sql" ? "contents" : "none" }}>
          <CrossDbSqlEditor databases={connectedDbs} />
        </div>
      </div>
    </div>
  );
}

