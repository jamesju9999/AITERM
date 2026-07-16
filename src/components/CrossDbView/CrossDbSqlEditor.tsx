import { useState } from "react";
import { dbExecuteQuery, type QueryResult } from "../../ipc/db";
import type { ConnectedDb } from "./index";
import { useLocale } from "../../contexts/LocaleContext";

interface Props {
  databases: ConnectedDb[];
}

export function CrossDbSqlEditor({ databases }: Props) {
  const { t } = useLocale();
  const [sql, setSql] = useState("SELECT 1;");
  const [selectedDbId, setSelectedDbId] = useState(databases[0]?.id ?? "");
  const [result, setResult] = useState<QueryResult | null>(null);
  const [running, setRunning] = useState(false);

  const selectedDb = databases.find((db) => db.id === selectedDbId);

  const run = async () => {
    if (!sql.trim() || !selectedDbId) return;
    setRunning(true);
    try {
      const r = await dbExecuteQuery(selectedDbId, sql, selectedDb?.schema || undefined);
      setResult(r);
    } catch (e: unknown) {
      setResult({ columns: [], rows: [], affected_rows: null, execution_time_ms: 0, error: String(e) });
    } finally {
      setRunning(false);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
      e.preventDefault();
      run();
    }
    if (e.key === "Tab") {
      e.preventDefault();
      const el = e.currentTarget;
      const start = el.selectionStart;
      const end = el.selectionEnd;
      const newVal = sql.slice(0, start) + "  " + sql.slice(end);
      setSql(newVal);
      requestAnimationFrame(() => {
        el.selectionStart = el.selectionEnd = start + 2;
      });
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", flex: 1, minWidth: 0, overflow: "hidden" }}>
      <div style={{ flex: "0 0 40%", display: "flex", flexDirection: "column", borderBottom: "1px solid #1e1e1e" }}>
        <div style={{ background: "#111", padding: "6px 12px", display: "flex", alignItems: "center", gap: 8, borderBottom: "1px solid #1e1e1e" }}>
          <span style={{ color: "#888", fontSize: 11 }}>SQL Editor</span>
          <select
            value={selectedDbId}
            onChange={(e) => setSelectedDbId(e.target.value)}
            style={{
              background: "#1a1a1a",
              border: "1px solid #2a2a2a",
              color: "#ccc",
              fontSize: 12,
              borderRadius: 4,
              padding: "3px 8px",
              marginLeft: 8,
            }}
          >
            {databases.map((db) => (
              <option key={db.id} value={db.id}>
                [{db.db_type.toUpperCase()}] {db.name} — {db.schema}
              </option>
            ))}
          </select>
          <div style={{ flex: 1 }} />
          <button
            onClick={run}
            disabled={running}
            className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
          >
            {running ? t.db_sql_running : t.db_sql_btn_run}
          </button>
        </div>
        <textarea
          value={sql}
          onChange={(e) => setSql(e.target.value)}
          onKeyDown={handleKeyDown}
          spellCheck={false}
          style={{
            flex: 1, background: "#0c0c0c", color: "#e6e6e6", border: "none",
            resize: "none", padding: "12px 14px", fontFamily: '"Cascadia Mono", Consolas, monospace',
            fontSize: 13, lineHeight: 1.6, outline: "none",
          }}
        />
      </div>

      <div style={{ flex: 1, overflow: "auto" }}>
        {!result && !running && (
          <div style={{ padding: 20, color: "#555", fontSize: 12 }}>
            {t.cdb_sql_empty_state}
          </div>
        )}
        {running && <div style={{ padding: 20, color: "#888", fontSize: 12 }}>{t.db_sql_running}</div>}
        {result && !running && (
          <>
            {selectedDb && (
              <div style={{ padding: "6px 12px", color: "#888", fontSize: 11, borderBottom: "1px solid #1e1e1e", background: "#111" }}>
                {t.cdb_sql_target(selectedDb.db_type.toUpperCase(), selectedDb.name)}
                {result.error ? "" : ` · ${result.rows.length} ${t.db_rows} · ${result.execution_time_ms}ms`}
              </div>
            )}
            {result.error && (
              <div style={{ padding: "12px 16px", color: "#f87171", fontSize: 12, fontFamily: "monospace" }}>
                ✗ {result.error}
              </div>
            )}
            {!result.error && result.affected_rows !== null && (
              <div style={{ padding: "12px 16px", color: "#34d399", fontSize: 12 }}>
                ✓ {result.affected_rows} {t.db_sql_affected_rows} ({result.execution_time_ms}ms)
              </div>
            )}
            {!result.error && result.columns.length > 0 && (
              <table style={{ borderCollapse: "collapse", width: "100%", fontSize: 12, fontFamily: "monospace" }}>
                <thead>
                  <tr style={{ background: "#151515" }}>
                    {result.columns.map((col) => (
                      <th key={col} style={{ padding: "6px 10px", textAlign: "left", color: "#888", borderBottom: "1px solid #222", fontWeight: "normal", whiteSpace: "nowrap" }}>
                        {col}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {result.rows.map((row, i) => (
                    <tr key={i} style={{ borderBottom: "1px solid #111", background: i % 2 === 0 ? "transparent" : "#0f0f0f" }}>
                      {row.map((cell, j) => (
                        <td key={j} style={{ padding: "4px 10px", color: cell === null ? "#444" : typeof cell === "number" ? "#f9a825" : "#ccc", whiteSpace: "nowrap" }}>
                          {cell === null ? "NULL" : String(cell)}
                        </td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </>
        )}
      </div>
    </div>
  );
}

