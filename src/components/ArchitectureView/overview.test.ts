import { describe, expect, it } from "vitest";
import { buildOverview } from "./overview";
import type { Architecture, Module } from "./model";
import type { ScanResult } from "../../ipc/architecture";

const mod = (id: string, tech: string[]): Module => ({ id, layer: "x", name: id, files: [], tech, summary: "", dataflow: "" });
const arch = (modules: Module[], nEdges = 0): Architecture => ({
  layers: [], modules,
  edges: Array.from({ length: nEdges }, (_, i) => ({ from: "a", to: `t${i}`, kind: "import" as const, source: "scan" as const, confidence: 1 })),
  meta: { scannedAt: "", gitHead: null, analyzed: false, dropped: 0 },
});
const scan = (langs: string[], deps: string[] = []): ScanResult => ({
  root: "/p", files: langs.map((lang, i) => ({ path: `f${i}`, lang, lines: 1 })), edges: [], deps, truncated: false, git_head: null,
});

describe("buildOverview", () => {
  it("語言依檔案數降冪；同數量依字母序", () => {
    const o = buildOverview(scan(["py", "ts", "ts", "rs", "ts", "py"]), arch([]));
    expect(o.languages).toEqual([{ lang: "ts", count: 3 }, { lang: "py", count: 2 }, { lang: "rs", count: 1 }]);
    const tie = buildOverview(scan(["b", "a"]), arch([]));
    expect(tie.languages.map((l) => l.lang)).toEqual(["a", "b"]);
  });

  it("技術標籤跨模組彙整計次（同一模組內重複只算一次）並降冪", () => {
    const o = buildOverview(scan([]), arch([mod("m1", ["React", "Vite", "React"]), mod("m2", ["Rust", "React"]), mod("m3", ["Rust"])]));
    expect(o.tech).toEqual([{ name: "React", count: 2 }, { name: "Rust", count: 2 }, { name: "Vite", count: 1 }]);
  });

  it("依賴只取前 30 個；模組與連線數", () => {
    const deps = Array.from({ length: 45 }, (_, i) => `d${i}`);
    const o = buildOverview(scan([], deps), arch([mod("a", []), mod("b", [])], 3));
    expect(o.deps).toHaveLength(30);
    expect(o.deps[29]).toBe("d29");
    expect(o.moduleCount).toBe(2);
    expect(o.edgeCount).toBe(3);
  });
});
