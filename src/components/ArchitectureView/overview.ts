import type { ScanResult } from "../../ipc/architecture";
import type { Architecture } from "./model";

export interface Overview {
  languages: { lang: string; count: number }[];
  tech: { name: string; count: number }[];
  deps: string[];
  moduleCount: number;
  edgeCount: number;
}

function tally(items: string[]): [string, number][] {
  const m = new Map<string, number>();
  for (const i of items) m.set(i, (m.get(i) ?? 0) + 1);
  // 次數降冪；同次數依名稱字母序，讓畫面穩定。
  return [...m.entries()].sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
}

/** 技術棧總覽：語言分布、模組技術標籤彙整（每個模組同標籤只算一次）、依賴前 30、模組與連線數。 */
export function buildOverview(scan: ScanResult, arch: Architecture): Overview {
  return {
    languages: tally(scan.files.map((f) => f.lang)).map(([lang, count]) => ({ lang, count })),
    tech: tally(arch.modules.flatMap((m) => [...new Set(m.tech)])).map(([name, count]) => ({ name, count })),
    deps: scan.deps.slice(0, 30),
    moduleCount: arch.modules.length,
    edgeCount: arch.edges.length,
  };
}
