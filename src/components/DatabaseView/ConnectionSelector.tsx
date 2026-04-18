import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { dbListConnections, type DbConnectionInfo, DB_TYPE_LABELS } from "../../ipc/db";

interface Props {
  onSelect: (connId: string) => void;
}

export function ConnectionSelector({ onSelect }: Props) {
  const [connections, setConnections] = useState<DbConnectionInfo[]>([]);
  const navigate = useNavigate();

  useEffect(() => {
    dbListConnections().then(setConnections).catch(console.error);
  }, []);

  return (
    <div style={{ display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", height: "100%", gap: 16 }}>
      <div style={{ color: "#888", fontSize: 14 }}>選擇資料庫連線</div>
      <div style={{ display: "flex", flexDirection: "column", gap: 8, width: 280 }}>
        {connections.map((conn) => (
          <button
            key={conn.id}
            onClick={() => onSelect(conn.id)}
            style={{
              background: "#1a1a1a", border: "1px solid #2a2a2a", borderRadius: 6,
              padding: "10px 14px", display: "flex", justifyContent: "space-between",
              alignItems: "center", cursor: "pointer", color: "#e6e6e6",
            }}
          >
            <span style={{ fontSize: 13 }}>{conn.name}</span>
            <span style={{ fontSize: 11, color: "#888" }}>{DB_TYPE_LABELS[conn.db_type]}</span>
          </button>
        ))}
        <button
          onClick={() => navigate("/settings")}
          style={{
            border: "1px dashed #333", background: "transparent", borderRadius: 6,
            padding: "10px 14px", color: "#555", fontSize: 12, cursor: "pointer",
          }}
        >
          ⚙ 新增 / 管理連線...
        </button>
      </div>
    </div>
  );
}
