import { useState, useEffect } from "react";
import { dbConnect, dbListConnections, dbListSchemas, dbListTables, type DbConnectionInfo, type TableInfo } from "../../ipc/db";
import { CrossDbAiChat } from "./CrossDbAiChat";
import { CrossDbSqlEditor } from "./CrossDbSqlEditor";
import { useTelegramRemoteControl } from "../../hooks/useTelegramRemoteControl";
import "./CrossDbView.css";

export interface CrossDbViewProps {
  isActive: boolean;
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

export function CrossDbView({ isActive }: CrossDbViewProps) {
  const [allConnections, setAllConnections] = useState<DbConnectionInfo[]>([]);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [connectedDbs, setConnectedDbs] = useState<ConnectedDb[]>([]);
  const [loading, setLoading] = useState(false);
  const [started, setStarted] = useState(false);
  const [subTab, setSubTab] = useState<SubTab>("ai");

  const { isRemoteEnabled, setIsRemoteEnabled, sendRemoteResponse } = useTelegramRemoteControl(
    "cross-db",
    isActive,
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
            <h3>🔗 跨資料庫查詢</h3>
            <p>選擇要參與查詢的資料庫連線，AI 將能跨庫分析與比對資料。</p>
          </div>
          {allConnections.length === 0 ? (
            <div className="crossdb-selector__empty">
              尚無資料庫連線。請先至設定頁面新增連線。
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
            className="crossdb-selector__start"
            disabled={selectedIds.size < 1 || loading}
            onClick={startSession}
          >
            {loading ? "連線中..." : `開始對話 (${selectedIds.size} 個資料庫)`}
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
          AI Chat
        </button>
        <button
          className={`crossdb-subtab ${subTab === "sql" ? "crossdb-subtab--active" : ""}`}
          onClick={() => setSubTab("sql")}
        >
          SQL Editor
        </button>
        <button
          className={`crossdb-subtab ${isRemoteEnabled ? "crossdb-subtab--active" : ""}`}
          style={{ marginLeft: 8 }}
          onClick={() => setIsRemoteEnabled(!isRemoteEnabled)}
          title="啟用/停用 Telegram 遠端控制"
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
          ← 重新選擇
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
