import { useEffect, useState } from "react";
import { dbListTables, dbGetTableSchema, dbExecuteQuery, TableInfo, ColumnInfo, QueryResult } from "../../ipc/db";

interface Props {
  connectionId: string;
  schema: string;
}

type ViewMode = "data" | "structure";

export function DatabaseBrowser({ connectionId, schema }: Props) {
  const [tables, setTables] = useState<TableInfo[]>([]);
  const [selectedTable, setSelectedTable] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<ViewMode>("data");
  const [queryResult, setQueryResult] = useState<QueryResult | null>(null);
  const [columns, setColumns] = useState<ColumnInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [page, setPage] = useState(0);
  const PAGE_SIZE = 100;

  useEffect(() => {
    if (!schema) return;
    dbListTables(connectionId, schema).then(setTables).catch(console.error);
    setSelectedTable(null);
    setQueryResult(null);
  }, [connectionId, schema]);

  const selectTable = async (name: string) => {
    setSelectedTable(name);
    setPage(0);
    setLoading(true);
    try {
      if (viewMode === "data") {
        const result = await dbExecuteQuery(connectionId, `SELECT * FROM "${schema}"."${name}" LIMIT ${PAGE_SIZE} OFFSET 0`);
        setQueryResult(result);
      } else {
        const cols = await dbGetTableSchema(connectionId, schema, name);
        setColumns(cols);
      }
    } finally {
      setLoading(false);
    }
  };

  const switchMode = async (mode: ViewMode) => {
    setViewMode(mode);
    if (!selectedTable) return;
    setLoading(true);
    try {
      if (mode === "data") {
        const result = await dbExecuteQuery(connectionId, `SELECT * FROM "${schema}"."${selectedTable}" LIMIT ${PAGE_SIZE} OFFSET ${page * PAGE_SIZE}`);
        setQueryResult(result);
      } else {
        const cols = await dbGetTableSchema(connectionId, schema, selectedTable);
        setColumns(cols);
      }
    } finally {
      setLoading(false);
    }
  };

  const loadPage = async (newPage: number) => {
    if (!selectedTable) return;
    setPage(newPage);
    setLoading(true);
    try {
      const result = await dbExecuteQuery(connectionId, `SELECT * FROM "${schema}"."${selectedTable}" LIMIT ${PAGE_SIZE} OFFSET ${newPage * PAGE_SIZE}`);
      setQueryResult(result);
    } finally {
      setLoading(false);
    }
  };

  return (
    <div style={{ display: "flex", height: "100%", overflow: "hidden" }}>
      {/* Object tree */}
      <div style={{ width: 200, flexShrink: 0, background: "#111", borderRight: "1px solid #1e1e1e", overflowY: "auto", padding: "8px 0" }}>
        <div style={{ color: "#666", fontSize: 10, letterSpacing: 1, padding: "4px 12px", marginBottom: 4 }}>TABLES</div>
        {tables.map((t) => (
          <div
            key={t.name}
            onClick={() => selectTable(t.name)}
            style={{
              padding: "5px 12px", cursor: "pointer", fontSize: 12,
              color: selectedTable === t.name ? "#34d399" : "#ccc",
              background: selectedTable === t.name ? "#1a2a1a" : "transparent",
              display: "flex", alignItems: "center", gap: 6,
            }}
          >
            <span style={{ fontSize: 10, color: "#555" }}>{t.table_type === "view" ? "👁" : "▤"}</span>
            {t.name}
          </div>
        ))}
      </div>

      {/* Content area */}
      <div style={{ flex: 1, display: "flex", flexDirection: "column", overflow: "hidden" }}>
        {selectedTable && (
          <div style={{ background: "#111", borderBottom: "1px solid #1e1e1e", padding: "6px 12px", display: "flex", gap: 0, alignItems: "center" }}>
            <button onClick={() => switchMode("data")} style={{ ...modeBtn, ...(viewMode === "data" ? modeBtnActive : {}) }}>資料</button>
            <button onClick={() => switchMode("structure")} style={{ ...modeBtn, ...(viewMode === "structure" ? modeBtnActive : {}) }}>結構</button>
            <span style={{ color: "#555", fontSize: 11, marginLeft: "auto" }}>{selectedTable}</span>
          </div>
        )}

        {loading && <div style={{ padding: 16, color: "#888", fontSize: 12 }}>載入中...</div>}

        {!loading && !selectedTable && (
          <div style={{ padding: 24, color: "#555", fontSize: 13 }}>← 從左側選擇資料表</div>
        )}

        {!loading && selectedTable && viewMode === "data" && queryResult && (
          <DataGrid result={queryResult} page={page} pageSize={PAGE_SIZE} onPageChange={loadPage} />
        )}

        {!loading && selectedTable && viewMode === "structure" && (
          <StructureView columns={columns} />
        )}
      </div>
    </div>
  );
}

function DataGrid({ result, page, pageSize, onPageChange }: { result: QueryResult; page: number; pageSize: number; onPageChange: (p: number) => void }) {
  if (result.error) {
    return <div style={{ padding: 16, color: "#f87171", fontSize: 12 }}>錯誤：{result.error}</div>;
  }
  return (
    <div style={{ flex: 1, overflow: "auto" }}>
      <table style={{ borderCollapse: "collapse", width: "100%", fontSize: 12, fontFamily: "monospace" }}>
        <thead>
          <tr style={{ background: "#151515", position: "sticky", top: 0 }}>
            {result.columns.map((col) => (
              <th key={col} style={{ padding: "6px 10px", textAlign: "left", color: "#888", borderBottom: "1px solid #222", whiteSpace: "nowrap", fontWeight: "normal" }}>
                {col}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {result.rows.map((row, i) => (
            <tr key={i} style={{ borderBottom: "1px solid #111", background: i % 2 === 0 ? "transparent" : "#0f0f0f" }}>
              {row.map((cell, j) => (
                <td key={j} style={{ padding: "4px 10px", color: cell === null ? "#444" : typeof cell === "number" ? "#f9a825" : "#ccc", whiteSpace: "nowrap", maxWidth: 300, overflow: "hidden", textOverflow: "ellipsis" }}>
                  {cell === null ? "NULL" : String(cell)}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      <div style={{ padding: "8px 12px", display: "flex", gap: 8, alignItems: "center", borderTop: "1px solid #1e1e1e", fontSize: 11, color: "#888" }}>
        <span>{result.rows.length} 列 · {result.execution_time_ms}ms</span>
        <div style={{ flex: 1 }} />
        {page > 0 && <button onClick={() => onPageChange(page - 1)} style={pageBtn}>← 上一頁</button>}
        <span>第 {page + 1} 頁</span>
        {result.rows.length === pageSize && <button onClick={() => onPageChange(page + 1)} style={pageBtn}>下一頁 →</button>}
      </div>
    </div>
  );
}

function StructureView({ columns }: { columns: ColumnInfo[] }) {
  return (
    <div style={{ flex: 1, overflow: "auto" }}>
      <table style={{ borderCollapse: "collapse", width: "100%", fontSize: 12 }}>
        <thead>
          <tr style={{ background: "#151515" }}>
            {["欄位名稱", "類型", "可為 NULL", "預設值"].map((h) => (
              <th key={h} style={{ padding: "6px 10px", textAlign: "left", color: "#888", borderBottom: "1px solid #222", fontWeight: "normal" }}>{h}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {columns.map((col, i) => (
            <tr key={col.name} style={{ borderBottom: "1px solid #111", background: i % 2 === 0 ? "transparent" : "#0f0f0f" }}>
              <td style={{ padding: "4px 10px", color: "#e6e6e6" }}>{col.name}</td>
              <td style={{ padding: "4px 10px", color: "#60a5fa", fontFamily: "monospace" }}>{col.data_type}</td>
              <td style={{ padding: "4px 10px", color: col.nullable ? "#34d399" : "#f87171" }}>{col.nullable ? "YES" : "NO"}</td>
              <td style={{ padding: "4px 10px", color: "#888" }}>{col.default ?? "—"}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

const modeBtn: React.CSSProperties = { background: "transparent", border: "none", color: "#888", fontSize: 12, padding: "4px 12px", cursor: "pointer" };
const modeBtnActive: React.CSSProperties = { color: "#34d399", borderBottom: "2px solid #34d399" };
const pageBtn: React.CSSProperties = { background: "#1a1a1a", border: "1px solid #2a2a2a", color: "#888", fontSize: 11, padding: "2px 10px", borderRadius: 3, cursor: "pointer" };
