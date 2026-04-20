import { useState } from "react";
import { dbExecuteQuery, type QueryResult } from "../../ipc/db";

interface Props {
  connectionId: string;
  schema: string;
}

export function DatabaseSqlEditor({ connectionId, schema }: Props) {
  const [sql, setSql] = useState("SELECT 1;");
  const [result, setResult] = useState<QueryResult | null>(null);
  const [running, setRunning] = useState(false);

  const run = async () => {
    if (!sql.trim()) return;
    setRunning(true);
    try {
      const r = await dbExecuteQuery(connectionId, sql, schema || undefined);
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
          <div style={{ flex: 1 }} />
          <button
            onClick={run}
            disabled={running}
            style={{ background: "#1e3a2e", border: "1px solid #34d399", color: "#34d399", borderRadius: 4, padding: "4px 14px", cursor: "pointer", fontSize: 12 }}
          >
            {running ? "執行中..." : "▶ 執行 (Ctrl+Enter)"}
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
          <div style={{ padding: 20, color: "#555", fontSize: 12 }}>執行 SQL 後結果將顯示於此</div>
        )}
        {running && <div style={{ padding: 20, color: "#888", fontSize: 12 }}>執行中...</div>}
        {result && !running && (
          <>
            {result.error && (
              <div style={{ padding: "12px 16px", color: "#f87171", fontSize: 12, fontFamily: "monospace" }}>
                ✗ {result.error}
              </div>
            )}
            {!result.error && result.affected_rows !== null && (
              <div style={{ padding: "12px 16px", color: "#34d399", fontSize: 12 }}>
                ✓ {result.affected_rows} 列受影響 ({result.execution_time_ms}ms)
              </div>
            )}
            {!result.error && result.columns.length > 0 && (
              <>
                <div style={{ padding: "6px 12px", color: "#888", fontSize: 11, borderBottom: "1px solid #1e1e1e", background: "#111" }}>
                  {result.rows.length} 列 · {result.execution_time_ms}ms
                </div>
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
              </>
            )}
          </>
        )}
      </div>
    </div>
  );
}
