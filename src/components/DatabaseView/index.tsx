export interface DatabaseViewProps {
  tabId: string;
  isActive: boolean;
  dbConnectionId?: string;
  onConnectionSelected: (connId: string) => void;
}

export function DatabaseView(_props: DatabaseViewProps) {
  return <div style={{ color: "#888", padding: "20px" }}>Database view coming soon...</div>;
}
