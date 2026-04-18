import { useEffect, useState } from "react";
import { dbConnect, dbListSchemas } from "../../ipc/db";
import { ConnectionSelector } from "./ConnectionSelector";
import { DatabaseBrowser } from "./DatabaseBrowser";
import { DatabaseSqlEditor } from "./DatabaseSqlEditor";
import { DatabaseAiChat } from "./DatabaseAiChat";
import "./index.css";

export interface DatabaseViewProps {
  tabId: string;
  isActive: boolean;
  dbConnectionId?: string;
  onConnectionSelected: (connId: string) => void;
}

type SubTab = "browse" | "ai" | "sql";

export function DatabaseView({ tabId: _tabId, isActive: _isActive, dbConnectionId, onConnectionSelected }: DatabaseViewProps) {
  const [subTab, setSubTab] = useState<SubTab>("browse");
  const [schemas, setSchemas] = useState<string[]>([]);
  const [activeSchema, setActiveSchema] = useState<string>("");
  const [connectError, setConnectError] = useState<string | null>(null);

  useEffect(() => {
    if (!dbConnectionId) return;
    dbConnect(dbConnectionId)
      .then(() => dbListSchemas(dbConnectionId))
      .then((s) => {
        setSchemas(s);
        setActiveSchema(s[0] ?? "");
        setConnectError(null);
      })
      .catch((e: unknown) => setConnectError(String(e)));
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
            <div style={{ marginBottom: 8 }}>⚠️ DB2 ODBC Driver 未安裝</div>
            <div style={{ color: "#888", fontSize: 12, marginBottom: 12 }}>
              請安裝 IBM Data Server Driver Package，然後重新嘗試連線。
            </div>
            <button
              onClick={() => setConnectError(null)}
              style={{ background: "#1a1a1a", border: "1px solid #3a3a3a", color: "#ccc", borderRadius: 4, padding: "6px 14px", cursor: "pointer" }}
            >
              重新嘗試
            </button>
          </>
        ) : (
          <>
            <div style={{ marginBottom: 8 }}>連線失敗</div>
            <div style={{ color: "#888", fontSize: 12 }}>{connectError}</div>
            <button
              onClick={() => { setConnectError(null); }}
              style={{ marginTop: 12, background: "#1a1a1a", border: "1px solid #3a3a3a", color: "#ccc", borderRadius: 4, padding: "6px 14px", cursor: "pointer" }}
            >
              重新連線
            </button>
          </>
        )}
      </div>
    );
  }

  return (
    <div className="db-view">
      <div className="db-view__subtabs">
        {([["browse", "瀏覽"], ["ai", "AI Chat"], ["sql", "SQL Editor"]] as [SubTab, string][]).map(([key, label]) => (
          <button
            key={key}
            className={`db-view__subtab ${subTab === key ? "db-view__subtab--active" : ""}`}
            onClick={() => setSubTab(key)}
          >
            {label}
          </button>
        ))}
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
              <DatabaseAiChat connectionId={dbConnectionId} schema={activeSchema} />
            </div>
            <div style={{ display: subTab === "sql" ? "contents" : "none" }}>
              <DatabaseSqlEditor connectionId={dbConnectionId} />
            </div>
          </>
        )}
      </div>
    </div>
  );
}
