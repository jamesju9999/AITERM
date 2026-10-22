import { describe, expect, it } from "vitest";
import { flowPaths, groupEdges } from "./flow";
import type { Architecture, Edge, EdgeKind } from "./model";

const e = (from: string, to: string, kind: EdgeKind = "import"): Edge => ({ from, to, kind, source: "scan", confidence: 1 });
const archOf = (edges: Edge[]): Architecture => ({
  layers: [], modules: [], edges, meta: { scannedAt: "", gitHead: null, analyzed: false, dropped: 0 },
});
const names = (paths: Edge[][]) => paths.map((p) => [p[0].from, ...p.map((x) => x.to)].join(">"));

describe("flowPaths", () => {
  it("同一節點可由長短不同的路徑到達：兩條都列出，長的在前，且不列出可延伸的前綴", () => {
    const a = archOf([e("A", "C"), e("A", "B"), e("B", "C")]);
    const r = flowPaths(a, "A");
    expect(names(r.down)).toEqual(["A>B>C", "A>C"]);
    expect(r.up).toEqual([]);
  });

  it("上游路徑：由最上游走到 start，邊依流向排列，長的在前", () => {
    const a = archOf([e("X", "A"), e("W", "X"), e("Y", "A")]);
    const r = flowPaths(a, "A");
    expect(names(r.up)).toEqual(["W>X>A", "Y>A"]);
    expect(r.down).toEqual([]);
  });

  it("環 A→B→A 不重複走節點", () => {
    const a = archOf([e("A", "B"), e("B", "A"), e("B", "C")]);
    const r = flowPaths(a, "A");
    expect(names(r.down)).toEqual(["A>B>C"]);
    // 上游：B→A 是唯一進入 A 的邊，再往上 A 已在路徑中，所以停在 B
    expect(names(r.up)).toEqual(["B>A"]);
  });

  it("深度上限：最多 maxDepth 條邊", () => {
    const chain = archOf([e("1", "2"), e("2", "3"), e("3", "4"), e("4", "5")]);
    expect(names(flowPaths(chain, "1", { maxDepth: 2 }).down)).toEqual(["1>2>3"]);
    expect(names(flowPaths(chain, "1").down)).toEqual(["1>2>3>4>5"]);
  });

  it("條數上限：保留最長的前 maxPaths 條", () => {
    const a = archOf([e("S", "a"), e("a", "b"), e("S", "c"), e("S", "d")]);
    const r = flowPaths(a, "S", { maxPaths: 2 });
    expect(names(r.down)).toEqual(["S>a>b", "S>c"]);
  });

  it("預設深度 6、條數 8", () => {
    const chain = archOf(Array.from({ length: 9 }, (_, i) => e(`n${i}`, `n${i + 1}`)));
    expect(flowPaths(chain, "n0").down[0]).toHaveLength(6);
    const fan = archOf(Array.from({ length: 12 }, (_, i) => e("S", `t${i}`)));
    expect(flowPaths(fan, "S").down).toHaveLength(8);
  });

  it("沒有連線的節點回傳空", () => {
    expect(flowPaths(archOf([e("A", "B")]), "Z")).toEqual({ down: [], up: [] });
  });
});

describe("groupEdges", () => {
  it("同一對模組同 kind 的兩個方向合併成 mutual", () => {
    const g = groupEdges([e("A", "B"), e("B", "A"), e("A", "C")], "A");
    expect(g).toHaveLength(2);
    expect(g[0]).toMatchObject({ other: "B", kind: "import", mutual: true });
    expect(g[0].edges).toHaveLength(2);
    expect(g[1]).toMatchObject({ other: "C", kind: "import", mutual: false });
  });

  it("不同 kind 不合併，即使同一對模組", () => {
    const g = groupEdges([e("A", "B", "import"), e("B", "A", "http")], "A");
    expect(g).toHaveLength(2);
    expect(g.every((x) => !x.mutual)).toBe(true);
  });

  it("只有單向的上游邊 other 是來源端；與 selfId 無關的邊與自環被忽略", () => {
    const g = groupEdges([e("X", "A"), e("P", "Q"), e("A", "A")], "A");
    expect(g).toHaveLength(1);
    expect(g[0]).toMatchObject({ other: "X", mutual: false });
  });
});
