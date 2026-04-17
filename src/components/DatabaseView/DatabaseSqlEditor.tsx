export function DatabaseSqlEditor({ connectionId }: { connectionId: string }) {
  return <div style={{ padding: 16, color: "#888", fontSize: 13 }}>SQL Editor: {connectionId}</div>;
}
