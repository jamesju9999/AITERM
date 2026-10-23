import { describe, expect, it } from "vitest";
import { parseAnalysis, pickKeyFiles, buildAnalyzeMessage, mergeScanEdges } from "./analyze";
import { buildBaseGraph } from "./model";
import type { ScanResult } from "../../ipc/architecture";

const scan: ScanResult = {
  root: "/p",
  files: [
    { path: "web/App.tsx", lang: "typescript", lines: 50 },
    { path: "web/api.ts", lang: "typescript", lines: 20 },
    { path: "server/routes.py", lang: "python", lines: 80 },
    { path: "server/db.py", lang: "python", lines: 30 },
  ],
  edges: [{ from: "web/App.tsx", to: "web/api.ts" }],
  deps: ["react", "flask"],
  truncated: false,
  git_head: "abc",
};

const good = {
  layers: [{ id: "frontend", name: "前端" }, { id: "backend", name: "後端" }],
  modules: [
    { id: "web", layer: "frontend", name: "網頁", files: ["web/"], tech: ["React"], summary: "UI", dataflow: "呼叫 /api" },
    { id: "server", layer: "backend", name: "服務", files: ["server/routes.py", "server/db.py"], tech: ["Flask"], summary: "API", dataflow: "讀 DB" },
  ],
  edges: [
    { from: "web", to: "server", kind: "http", confidence: 0.9, evidence: { file: "web/api.ts", line: 5 }, note: "fetch" },
  ],
};

describe("parseAnalysis", () => {
  it("接受合法回應並展開目錄前綴為真實檔案", () => {
    const r = parseAnalysis("```json\n" + JSON.stringify(good) + "\n```", scan)!;
    expect(r.modules.find((m) => m.id === "web")!.files.sort()).toEqual(["web/App.tsx", "web/api.ts"]);
    expect(r.edges).toHaveLength(1);
    expect(r.edges[0]).toMatchObject({ source: "ai", kind: "http", confidence: 0.9 });
    expect(r.meta.analyzed).toBe(true);
    expect(r.meta.dropped).toBe(0);
  });

  it("丟掉幻覺：不存在的檔案、超出行數、未知模組、未知 kind、未知 layer", () => {
    const bad = structuredClone(good);
    bad.modules.push({ id: "ghost", layer: "frontend", name: "g", files: ["nope/x.ts"], tech: [], summary: "", dataflow: "" });
    bad.modules.push({ id: "m2", layer: "nolayer", name: "g", files: ["web/App.tsx"], tech: [], summary: "", dataflow: "" });
    bad.edges.push(
      { from: "web", to: "server", kind: "http", confidence: 0.5, evidence: { file: "web/api.ts", line: 999 }, note: "" },
      { from: "web", to: "server", kind: "http", confidence: 0.5, evidence: { file: "web/ghost.ts", line: 1 }, note: "" },
      { from: "web", to: "ghost", kind: "http", confidence: 0.5, evidence: { file: "web/api.ts", line: 1 }, note: "" },
      { from: "web", to: "server", kind: "carrier-pigeon", confidence: 0.5, evidence: { file: "web/api.ts", line: 1 }, note: "" },
    );
    const r = parseAnalysis(JSON.stringify(bad), scan)!;
    expect(r.modules.map((m) => m.id).sort()).toEqual(["server", "web"]);
    expect(r.edges).toHaveLength(1);
    expect(r.meta.dropped).toBe(6);
  });

  it("confidence 夾在 0..1", () => {
    const g = structuredClone(good);
    g.edges[0].confidence = 7;
    expect(parseAnalysis(JSON.stringify(g), scan)!.edges[0].confidence).toBe(1);
  });

  it("不是 JSON 時回傳 null", () => {
    expect(parseAnalysis("抱歉我做不到", scan)).toBeNull();
  });
});

describe("pickKeyFiles", () => {
  it("優先挑入口／路由／資料庫檔案，最多 12 個", () => {
    const files = Array.from({ length: 40 }, (_, i) => ({ path: `pkg/f${i}.ts`, lang: "typescript", lines: 5 }));
    files.push({ path: "server/routes.py", lang: "python", lines: 5 }, { path: "db/schema.sql", lang: "sql", lines: 5 });
    const picked = pickKeyFiles({ ...scan, files });
    expect(picked.length).toBeLessThanOrEqual(12);
    expect(picked).toContain("server/routes.py");
    expect(picked).toContain("db/schema.sql");
  });
});

describe("buildAnalyzeMessage", () => {
  it("包含依賴、模組概況與關鍵檔案內容", () => {
    const msg = buildAnalyzeMessage(scan, buildBaseGraph(scan), [["server/routes.py", "@app.route('/x')"]]);
    expect(msg).toContain("flask");
    expect(msg).toContain("server/routes.py");
    expect(msg).toContain("@app.route('/x')");
  });
});

describe("mergeScanEdges", () => {
  it("把檔案層級 import 映射成 AI 模組間的實線，模組內部的不算", () => {
    const ai = parseAnalysis(JSON.stringify(good), scan)!;
    const cross = { ...scan, edges: [{ from: "web/api.ts", to: "server/routes.py" }, { from: "web/App.tsx", to: "web/api.ts" }] };
    const merged = mergeScanEdges(ai, cross);
    const s = merged.edges.filter((e) => e.source === "scan");
    expect(s).toEqual([{ from: "web", to: "server", kind: "import", source: "scan", confidence: 1 }]);
    expect(merged.edges.filter((e) => e.source === "ai")).toHaveLength(1);
  });

  it("AI 的 import 連線與掃描連線重複時只留 scan 那條；其他 kind 不受影響", () => {
    const g = structuredClone(good);
    g.edges.push({ from: "web", to: "server", kind: "import", confidence: 0.6, evidence: { file: "web/api.ts", line: 1 }, note: "" });
    const ai = parseAnalysis(JSON.stringify(g), scan)!;
    const cross = { ...scan, edges: [{ from: "web/api.ts", to: "server/routes.py" }] };
    const merged = mergeScanEdges(ai, cross);
    expect(merged.edges.filter((e) => e.kind === "import")).toHaveLength(1);
    expect(merged.edges.filter((e) => e.kind === "import")[0].source).toBe("scan");
    expect(merged.edges.filter((e) => e.kind === "http")).toHaveLength(1);
  });
});

describe("parseAnalysis layer.role", () => {
  it("合法的 role 保留，非法值丟掉", () => {
    const withRole = { ...good, layers: [{ id: "frontend", name: "前端", role: "main" }, { id: "backend", name: "後端", role: "bogus" }] };
    const a = parseAnalysis(JSON.stringify(withRole), scan)!;
    expect(a.layers).toEqual([{ id: "frontend", name: "前端", role: "main" }, { id: "backend", name: "後端" }]);
  });
});
