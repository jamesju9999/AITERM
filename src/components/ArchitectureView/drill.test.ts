import { describe, expect, it } from "vitest";
import { MAX_DRILL_LAYERS, buildDrillGraph, drillKey, externalRefs, filesOfModule } from "./drill";
import type { Architecture } from "./model";
import type { ScanResult } from "../../ipc/architecture";

const scanOf = (paths: string[], edges: [string, string][] = []): ScanResult => ({
  root: "/p",
  files: paths.map((path) => ({ path, lang: "ts", lines: 10 })),
  edges: edges.map(([from, to]) => ({ from, to })),
  deps: [], truncated: false, git_head: "h",
});
const layerOf = (a: Architecture, id: string) => a.modules.find((m) => m.id === id)!.layer;
const ids = (a: Architecture) => a.modules.map((m) => m.id).sort();

describe("buildDrillGraph 分群", () => {
  it("共同前綴之下第一層目錄成群，直接在前綴下的檔案各自成為節點", () => {
    const files = ["a/b/x.ts", "a/b/c/y.ts", "a/b/c/z.ts", "a/b/d/w.ts"];
    const { arch, hidden } = buildDrillGraph(scanOf(files), files);
    expect(hidden).toBe(0);
    expect(ids(arch)).toEqual(["a/b/c", "a/b/d", "a/b/x.ts"]);
    const c = arch.modules.find((m) => m.id === "a/b/c")!;
    expect(c.name).toBe("c");
    expect(c.files.sort()).toEqual(["a/b/c/y.ts", "a/b/c/z.ts"]);
    const x = arch.modules.find((m) => m.id === "a/b/x.ts")!;
    expect(x.name).toBe("x.ts");
    expect(x.files).toEqual(["a/b/x.ts"]);
  });

  it("前綴不是檔案名稱：只有 a/b/x.ts 與 a/b/c/y.ts 時前綴是 a/b，不是 a", () => {
    const files = ["a/b/x.ts", "a/b/c/y.ts"];
    const { arch } = buildDrillGraph(scanOf(files), files);
    expect(ids(arch)).toEqual(["a/b/c", "a/b/x.ts"]);
  });

  it("沒有共同目錄時前綴為空，id 直接是第一段", () => {
    const files = ["src/a.ts", "lib/b.ts", "top.ts"];
    const { arch } = buildDrillGraph(scanOf(files), files);
    expect(ids(arch)).toEqual(["lib", "src", "top.ts"]);
  });
});

describe("buildDrillGraph 連線與分層", () => {
  const files = ["p/a.ts", "p/b.ts", "p/c.ts", "p/d.ts"];

  it("菱形 A→B、A→C、B→D、C→D：D 在最深一欄，A 在最左，B 與 C 同欄", () => {
    const { arch } = buildDrillGraph(scanOf(files, [
      ["p/a.ts", "p/b.ts"], ["p/a.ts", "p/c.ts"], ["p/b.ts", "p/d.ts"], ["p/c.ts", "p/d.ts"],
    ]), files);
    expect(layerOf(arch, "p/a.ts")).toBe("d0");
    expect(layerOf(arch, "p/b.ts")).toBe("d1");
    expect(layerOf(arch, "p/c.ts")).toBe("d1");
    expect(layerOf(arch, "p/d.ts")).toBe("d2");
    expect(arch.layers).toEqual([{ id: "d0", name: "第 1 層" }, { id: "d1", name: "第 2 層" }, { id: "d2", name: "第 3 層" }]);
    expect(arch.edges).toHaveLength(4);
    expect(arch.edges.every((e) => e.kind === "import" && e.source === "scan" && e.confidence === 1)).toBe(true);
  });

  it("長短路徑並存取最長：A→B→C 與 A→C，C 在第 3 欄", () => {
    const { arch } = buildDrillGraph(scanOf(files, [["p/a.ts", "p/b.ts"], ["p/b.ts", "p/c.ts"], ["p/a.ts", "p/c.ts"]]), files);
    expect(layerOf(arch, "p/c.ts")).toBe("d2");
  });

  it("含環 A→B→C→A 不會無窮迴圈；連線仍保留；環上節點深度有限", () => {
    const { arch } = buildDrillGraph(scanOf(files, [
      ["p/a.ts", "p/b.ts"], ["p/b.ts", "p/c.ts"], ["p/c.ts", "p/a.ts"], ["p/c.ts", "p/d.ts"],
    ]), files);
    expect(arch.edges).toHaveLength(4);
    expect(layerOf(arch, "p/a.ts")).toBe("d0");
    expect(layerOf(arch, "p/b.ts")).toBe("d1");
    expect(layerOf(arch, "p/c.ts")).toBe("d2");
    expect(layerOf(arch, "p/d.ts")).toBe("d3");
  });

  it("純環（沒有入度為 0 的節點）也能分層", () => {
    const f = ["p/a.ts", "p/b.ts"];
    const { arch } = buildDrillGraph(scanOf(f, [["p/a.ts", "p/b.ts"], ["p/b.ts", "p/a.ts"]]), f);
    expect(arch.modules).toHaveLength(2);
    expect(new Set(arch.modules.map((m) => m.layer)).size).toBe(2);
  });

  it("群之間的 import 彙整成一條；群內部與同節點連線略過；外部檔案的連線排除", () => {
    const f = ["q/g1/a.ts", "q/g1/b.ts", "q/g2/c.ts", "q/g2/d.ts"];
    const { arch } = buildDrillGraph(scanOf(f, [
      ["q/g1/a.ts", "q/g2/c.ts"], ["q/g1/b.ts", "q/g2/d.ts"], // 兩條彙整成 g1→g2
      ["q/g1/a.ts", "q/g1/b.ts"],                              // 群內
      ["q/g1/a.ts", "outside/z.ts"], ["outside/z.ts", "q/g2/c.ts"], // 外部
    ]), f);
    expect(arch.edges).toEqual([{ from: "q/g1", to: "q/g2", kind: "import", source: "scan", confidence: 1 }]);
  });

  it("沒有連線的節點放第 1 欄；沒有任何節點時回傳空圖", () => {
    const { arch } = buildDrillGraph(scanOf(["p/a.ts", "p/b.ts"]), ["p/a.ts", "p/b.ts"]);
    expect(arch.modules.every((m) => m.layer === "d0")).toBe(true);
    const empty = buildDrillGraph(scanOf([]), []);
    expect(empty.arch.modules).toEqual([]);
    expect(empty.arch.layers).toEqual([]);
    expect(empty.hidden).toBe(0);
  });
});

describe("buildDrillGraph 節點上限", () => {
  it("超過 maxNodes 時保留度數最高者，hidden 為被隱藏的數量，連線只剩保留節點之間", () => {
    // hub 與 5 個葉子相連（度 5）；m1-m2 各度 1（彼此相連）；iso1、iso2 度 0
    const f = ["p/hub.ts", "p/l1.ts", "p/l2.ts", "p/l3.ts", "p/l4.ts", "p/l5.ts", "p/m1.ts", "p/m2.ts", "p/iso1.ts", "p/iso2.ts"];
    const edges: [string, string][] = [
      ["p/hub.ts", "p/l1.ts"], ["p/hub.ts", "p/l2.ts"], ["p/hub.ts", "p/l3.ts"], ["p/hub.ts", "p/l4.ts"], ["p/hub.ts", "p/l5.ts"],
      ["p/m1.ts", "p/m2.ts"],
    ];
    const { arch, hidden } = buildDrillGraph(scanOf(f, edges), f, 6);
    expect(arch.modules).toHaveLength(6);
    expect(hidden).toBe(4);
    expect(ids(arch)).toContain("p/hub.ts");
    expect(ids(arch)).not.toContain("p/iso1.ts");
    expect(ids(arch)).not.toContain("p/iso2.ts");
    const kept = new Set(ids(arch));
    expect(arch.edges.every((e) => kept.has(e.from) && kept.has(e.to))).toBe(true);
    // 度數同為 1 的葉子與 m1/m2 以 id 字母序取捨：留下 hub + l1..l5，m1-m2 那條連線隨之消失
    expect(arch.edges).toHaveLength(5);
  });

  it("預設上限 60", () => {
    const f = Array.from({ length: 75 }, (_, i) => `p/f${i}.ts`);
    const { arch, hidden } = buildDrillGraph(scanOf(f), f);
    expect(arch.modules).toHaveLength(60);
    expect(hidden).toBe(15);
  });
});

describe("drillKey / filesOfModule / 舊存檔", () => {
  it("drillKey 以 \\u0000 連接模組 id", () => {
    expect(drillKey(["a", "b/c"])).toBe("a\u0000b/c");
    expect(drillKey(["a"])).toBe("a");
    expect(drillKey([])).toBe("");
  });

  it("filesOfModule 回傳模組檔案；找不到回空陣列", () => {
    const f = ["p/a.ts", "p/b.ts"];
    const { arch } = buildDrillGraph(scanOf(f), f);
    expect(filesOfModule(arch, "p/a.ts")).toEqual(["p/a.ts"]);
    expect(filesOfModule(arch, "nope")).toEqual([]);
  });

  it("讀到沒有 drills 欄位的舊存檔不報錯，查表得 undefined", () => {
    const old = JSON.parse(JSON.stringify({
      layers: [], modules: [], edges: [], meta: { scannedAt: "", gitHead: null, analyzed: true, dropped: 0 },
    })) as Architecture;
    expect(old.drills?.[drillKey(["a"])]).toBeUndefined();
  });
});

describe("externalRefs", () => {
  it("只算跨越模組邊界的 import：被外部引用與引用外部分開計，內部與無關的不算", () => {
    const scan = scanOf(
      ["m/a.ts", "m/b.ts", "x/c.ts", "y/d.ts"],
      [["x/c.ts", "m/a.ts"], ["y/d.ts", "m/a.ts"], ["y/d.ts", "m/b.ts"], ["m/a.ts", "x/c.ts"], ["m/a.ts", "m/b.ts"], ["x/c.ts", "y/d.ts"]],
    );
    expect(externalRefs(scan, ["m/a.ts", "m/b.ts"])).toEqual({ inbound: 3, outbound: 1 });
  });

  it("沒有檔案時兩者皆為 0", () => {
    expect(externalRefs(scanOf(["a.ts"], [["a.ts", "a.ts"]]), [])).toEqual({ inbound: 0, outbound: 0 });
  });
});

describe("buildDrillGraph 層數上限", () => {
  it("MAX_DRILL_LAYERS 是 5", () => { expect(MAX_DRILL_LAYERS).toBe(5); });

  it("長鏈 c0→…→c7 加菱形：深度 ≥ 4 的節點全進最後一層 d4，沒有 d5 以後；連線照真實連通保留", () => {
    const chain = Array.from({ length: 8 }, (_, i) => `p/c${i}.ts`);
    const dia = ["p/a.ts", "p/b.ts", "p/c.ts", "p/d.ts"];
    const files = [...chain, ...dia];
    const edges: [string, string][] = [
      ...chain.slice(0, -1).map((f, i) => [f, chain[i + 1]] as [string, string]),
      ["p/a.ts", "p/b.ts"], ["p/a.ts", "p/c.ts"], ["p/b.ts", "p/d.ts"], ["p/c.ts", "p/d.ts"],
      ["p/d.ts", "p/c5.ts"], // 菱形尾巴接進鏈中段：c5 仍在 d4
    ];
    const { arch } = buildDrillGraph(scanOf(files, edges), files);
    expect(arch.layers.map((l) => l.id)).toEqual(["d0", "d1", "d2", "d3", "d4"]);
    expect(arch.layers[4].name).toBe("第 5 層起");
    expect(arch.layers[0].name).toBe("第 1 層");
    expect(new Set(arch.modules.map((m) => m.layer))).toEqual(new Set(["d0", "d1", "d2", "d3", "d4"]));
    for (const i of [4, 5, 6, 7]) expect(layerOf(arch, `p/c${i}.ts`)).toBe("d4");
    expect(layerOf(arch, "p/c3.ts")).toBe("d3");
    expect(layerOf(arch, "p/d.ts")).toBe("d2");
    expect(arch.edges).toHaveLength(edges.length);
    expect(arch.edges.some((e) => e.from === "p/c6.ts" && e.to === "p/c7.ts")).toBe(true);
  });

  it("深度剛好不超過 4 時不多不少", () => {
    const files = ["p/a.ts", "p/b.ts"];
    const { arch } = buildDrillGraph(scanOf(files, [["p/a.ts", "p/b.ts"]]), files);
    expect(arch.layers.map((l) => l.id)).toEqual(["d0", "d1"]);
  });
});
