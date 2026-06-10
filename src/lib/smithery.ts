export interface SmitheryServer {
  qualifiedName: string;
  displayName: string;
  description: string;
  homepage?: string;
}

export interface SmitheryConnection {
  type: "stdio" | "http" | "sse";
  stdioFunction?: {
    command: string;
    args: string[];
    env: Record<string, string>;
  };
  url?: string;
}

export interface SmitheryServerDetail {
  qualifiedName: string;
  displayName: string;
  description: string;
  connections: SmitheryConnection[];
}

const BASE = "https://registry.smithery.ai";

export async function searchSmithery(query: string): Promise<SmitheryServer[]> {
  const url = `${BASE}/servers?q=${encodeURIComponent(query)}&pageSize=20`;
  const res = await fetch(url);
  if (!res.ok) throw new Error(`Smithery API error: ${res.status}`);
  const data = await res.json();
  return data.servers as SmitheryServer[];
}

export async function getSmitheryServer(
  qualifiedName: string
): Promise<SmitheryServerDetail> {
  const url = `${BASE}/servers/${encodeURIComponent(qualifiedName)}`;
  const res = await fetch(url);
  if (!res.ok) throw new Error(`Smithery API error: ${res.status}`);
  return res.json() as Promise<SmitheryServerDetail>;
}
