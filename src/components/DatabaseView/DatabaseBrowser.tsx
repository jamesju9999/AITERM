export function DatabaseBrowser({ connectionId, schema }: { connectionId: string; schema: string }) {
  return <div style={{ padding: 16, color: "#888", fontSize: 13 }}>Browser: {connectionId} / {schema}</div>;
}
