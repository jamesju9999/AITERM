export function DatabaseAiChat({ connectionId, schema }: { connectionId: string; schema: string }) {
  return <div style={{ padding: 16, color: "#888", fontSize: 13 }}>AI Chat: {connectionId} / {schema}</div>;
}
