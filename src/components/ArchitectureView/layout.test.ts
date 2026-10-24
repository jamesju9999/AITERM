import { describe, expect, it } from "vitest";
import {
  layoutGraph, spreadLabels, clampZoom, zoomAt, centerOffset, edgePath, edgeMidpoint, edgeLength, fitScale, initialView, trueFitView, READABLE_SCALE, layerColor, neighborhood, isCompact,
  NODE_W, NODE_H, BAND_HEADER_H, ZONE_GAP, ZONE_CAPTION_H, NODE_H_COMPACT, MAX_ROWS_PER_COL, SUBCOL_GAP, COL_GAP, ROW_GAP, PAD, LANE_PAD, LANE_W, HEADER_H, EDGE_BULGE, MIN_ZOOM, LAYER_PALETTE,
  type Rect,
} from "./layout";
import type { Architecture, Edge, Module } from "./model";

const arch: Architecture = {
  layers: [{ id: "a", name: "A" }, { id: "b", name: "B" }],
  modules: [
    { id: "m1", layer: "a", name: "m1", files: [], tech: [], summary: "", dataflow: "" },
    { id: "m2", layer: "a", name: "m2", files: [], tech: [], summary: "", dataflow: "" },
    { id: "m3", layer: "b", name: "m3", files: [], tech: [], summary: "", dataflow: "" },
  ],
  edges: [
    { from: "m1", to: "m3", kind: "http", source: "ai", confidence: 0.8 },
    { from: "m2", to: "m3", kind: "import", source: "scan", confidence: 1 },
  ],
  meta: { scannedAt: "", gitHead: null, analyzed: true, dropped: 0 },
};

describe("layoutGraph", () => {
  it("每層一條泳道、卡片在泳道內由上往下排列", () => {
    const l = layoutGraph(arch);
    const laneX0 = PAD;
    expect(l.nodes.m1).toEqual({ x: laneX0 + LANE_PAD, y: PAD + HEADER_H, w: NODE_W, h: NODE_H_COMPACT });
    expect(l.nodes.m2).toEqual({ x: laneX0 + LANE_PAD, y: PAD + HEADER_H + NODE_H_COMPACT + ROW_GAP, w: NODE_W, h: NODE_H_COMPACT });
    expect(l.nodes.m3.x).toBe(PAD + LANE_W + COL_GAP + LANE_PAD);
    expect(l.columns).toHaveLength(2);
    expect(l.columns[0]).toMatchObject({ layerId: "a", x: laneX0, y: PAD, w: LANE_W, count: 2 });
    expect(l.columns[1].x).toBe(PAD + LANE_W + COL_GAP);
  });

  it("所有泳道等高，並完整包住最長那一欄；畫布寬高含外距", () => {
    const l = layoutGraph(arch);
    const laneH = HEADER_H + 2 * NODE_H_COMPACT + ROW_GAP + LANE_PAD;
    expect(l.columns.every((c) => c.h === laneH)).toBe(true);
    expect(l.nodes.m2.y + l.nodes.m2.h + LANE_PAD).toBe(l.columns[0].y + laneH);
    expect(l.width).toBe(PAD * 2 + 2 * LANE_W + COL_GAP);
    expect(l.height).toBe(PAD * 2 + laneH);
  });
});

describe("initialView / trueFitView", () => {
  it("READABLE_SCALE 介於 MIN_ZOOM 與 1 之間", () => {
    expect(READABLE_SCALE).toBeGreaterThan(MIN_ZOOM);
    expect(READABLE_SCALE).toBeLessThan(1);
  });
  it("真適合比易讀下限大時，自動視圖就是適合視窗並置中", () => {
    const v = initialView(1000, 300, 800, 600);
    expect(v.scale).toBeCloseTo(0.8);
    expect(v.tx).toBeCloseTo(centerOffset(1000 * 0.8, 800));
    expect(v.ty).toBeCloseTo(centerOffset(300 * 0.8, 600));
    expect(v.ty).toBeGreaterThan(0);
  });
  it("真適合小於易讀下限：縮放卡在 READABLE_SCALE，內容比視窗高／寬時靠左上、不置中", () => {
    const w = 4000, h = 2000;
    expect(fitScale(w, h, 800, 600)).toBeLessThan(READABLE_SCALE);
    const v = initialView(w, h, 800, 600);
    expect(v).toEqual({ scale: READABLE_SCALE, tx: 0, ty: 0 });
  });
  it("卡在下限但某一軸放得下時，該軸仍置中", () => {
    const v = initialView(4000, 300, 800, 600); // 高 300*0.55=165 < 600
    expect(v.scale).toBe(READABLE_SCALE);
    expect(v.tx).toBe(0);
    expect(v.ty).toBeCloseTo(centerOffset(300 * READABLE_SCALE, 600));
    expect(v.ty).toBeGreaterThan(0);
  });
  it("量不到視窗時為 1", () => {
    expect(initialView(4000, 2000, 0, 0).scale).toBe(1);
  });
  it("trueFitView 可低於易讀下限（到 MIN_ZOOM），並置中", () => {
    const v = trueFitView(4000, 300, 800, 600);
    expect(v.scale).toBeCloseTo(0.2);
    expect(v.scale).toBeLessThan(READABLE_SCALE);
    expect(v.tx).toBeCloseTo(0);
    expect(v.ty).toBeCloseTo(centerOffset(300 * 0.2, 600));
  });
});

describe("fitScale", () => {
  it("寬高都放得下時不縮放", () => {
    expect(fitScale(800, 500, 1000, 600)).toBe(1);
    expect(fitScale(1000, 600, 1000, 600)).toBe(1);
  });
  it("太寬時依寬度比例縮小", () => {
    expect(fitScale(1000, 100, 800, 600)).toBeCloseTo(0.8);
  });
  it("太高時依高度比例縮小（只有寬度的舊行為會把底部切掉）", () => {
    expect(fitScale(400, 2000, 800, 1000)).toBeCloseTo(0.5);
  });
  it("寬高都超出時取較小者；不低於 MIN_ZOOM", () => {
    expect(fitScale(1000, 2000, 800, 1000)).toBeCloseTo(0.5);
    expect(fitScale(1000, 6000, 800, 600)).toBe(MIN_ZOOM);
    expect(MIN_ZOOM).toBe(0.2);
  });
  it("量不到尺寸時回傳 1", () => {
    expect(fitScale(1000, 500, 0, 600)).toBe(1);
    expect(fitScale(1000, 500, 800, 0)).toBe(1);
    expect(fitScale(0, 500, 800, 600)).toBe(1);
  });
});

describe("layerColor", () => {
  it("已知分層有固定顏色，彼此不同", () => {
    const ids = ["frontend", "transport", "backend", "data", "other"];
    const cs = ids.map((id, i) => layerColor(id, i));
    expect(new Set(cs).size).toBe(ids.length);
    expect(layerColor("frontend", 9)).toBe(layerColor("frontend", 0));
  });
  it("未知分層依序號取調色盤並循環", () => {
    expect(layerColor("x", 0)).toBe(layerColor("y", 0));
    expect(layerColor("x", 1)).not.toBe(layerColor("x", 0));
    expect(layerColor("x", LAYER_PALETTE.length)).toBe(layerColor("x", 0));
  });
});

const R = (x: number, y: number, h = NODE_H): Rect => ({ x, y, w: NODE_W, h });

describe("edgePath", () => {
  const A = R(100, 100);
  it("不同欄且目標在右：來源右緣中點 → 目標左緣中點", () => {
    const B = R(100 + LANE_W + COL_GAP, 300);
    const d = edgePath(A, B);
    expect(d.startsWith(`M ${A.x + A.w} ${A.y + A.h / 2} C`)).toBe(true);
    expect(d.endsWith(`${B.x} ${B.y + B.h / 2}`)).toBe(true);
  });
  it("目標在左邊：來源左緣 → 目標右緣", () => {
    const B = R(A.x - LANE_W - COL_GAP, 300);
    const d = edgePath(A, B);
    expect(d.startsWith(`M ${A.x} ${A.y + A.h / 2} C`)).toBe(true);
    expect(d.endsWith(`${B.x + B.w} ${B.y + B.h / 2}`)).toBe(true);
  });
  it("同一欄：兩端都在右緣，並向右凸出到泳道間隙而不穿過卡片", () => {
    const B = R(A.x, A.y + A.h + ROW_GAP);
    const d = edgePath(A, B);
    const right = A.x + A.w;
    expect(d.startsWith(`M ${right} ${A.y + A.h / 2} C`)).toBe(true);
    expect(d.endsWith(`${right} ${B.y + B.h / 2}`)).toBe(true);
    const nums = d.match(/-?\d+(\.\d+)?/g)!.map(Number);
    const xs = nums.filter((_, i) => i % 2 === 0);
    expect(Math.max(...xs)).toBe(right + EDGE_BULGE);
    expect(xs.every((x) => x >= right)).toBe(true);
  });
  it("兩端高度不同（精簡卡 vs 完整卡）：各自取自己的垂直中點", () => {
    const compact = R(100, 100, NODE_H_COMPACT);
    const full = R(100 + LANE_W + COL_GAP, 300, NODE_H);
    const d = edgePath(compact, full);
    expect(d.startsWith(`M ${compact.x + NODE_W} ${100 + NODE_H_COMPACT / 2} C`)).toBe(true);
    expect(d.endsWith(`${full.x} ${300 + NODE_H / 2}`)).toBe(true);
  });
});

describe("edgeMidpoint", () => {
  it("不同欄：落在兩端點正中", () => {
    const A = R(100, 100);
    const B = R(100 + LANE_W + COL_GAP, 300, NODE_H_COMPACT);
    const m = edgeMidpoint(A, B);
    expect(m.x).toBeCloseTo((A.x + A.w + B.x) / 2);
    expect(m.y).toBeCloseTo((A.y + A.h / 2 + B.y + B.h / 2) / 2);
  });
  it("同一欄：貝茲中點在右緣外 0.75 倍凸出量", () => {
    const A = R(100, 100);
    const B = R(100, 100 + NODE_H + ROW_GAP);
    const m = edgeMidpoint(A, B);
    expect(m.x).toBeCloseTo(A.x + A.w + 0.75 * EDGE_BULGE);
    expect(m.y).toBeCloseTo((A.y + A.h / 2 + B.y + B.h / 2) / 2);
  });
});

describe("edgeLength", () => {
  it("不同欄：至少是兩端點直線距離，且隨距離增加", () => {
    const A = R(100, 100);
    const near = R(100 + LANE_W + COL_GAP, 100);
    const far = R(100 + 2 * (LANE_W + COL_GAP), 100);
    expect(edgeLength(A, near)).toBeGreaterThanOrEqual(near.x - (A.x + A.w));
    expect(edgeLength(A, far)).toBeGreaterThan(edgeLength(A, near));
  });
  it("同欄：含向右凸出的來回，長度大於 2 倍凸出量", () => {
    expect(edgeLength(R(100, 100), R(100, 100 + NODE_H + ROW_GAP))).toBeGreaterThan(2 * EDGE_BULGE);
  });
});

// ---- 稠密圖：45 個節點、一個 hub、一條離 hub 兩步的鏈 ----
const mkMod = (id: string, layer: string, full = false): Module => ({
  id, layer, name: id, files: [`${id}.ts`], tech: full ? ["React"] : [], summary: full ? "有摘要" : "", dataflow: "",
});
const dense = (() => {
  const modules: Module[] = [];
  for (let i = 0; i < 30; i++) modules.push(mkMod(`a${i}`, "a", i % 10 === 0));
  for (let i = 0; i < 12; i++) modules.push(mkMod(`b${i}`, "b"));
  modules.push(mkMod("hub", "b", true), mkMod("x1", "c"), mkMod("x2", "c"));
  const edges: Edge[] = [];
  const e = (from: string, to: string): Edge => ({ from, to, kind: "import", source: "scan", confidence: 1 });
  for (let i = 0; i < 6; i++) edges.push(e(`a${i}`, "hub"));      // 6 個上游
  for (let i = 0; i < 4; i++) edges.push(e("hub", `b${i}`));       // 4 個下游
  edges.push(e("hub", "x1"), e("x1", "x2"), e("b0", "x2"));          // 鏈：hub→x1→x2，x2 離 hub 兩步
  for (let i = 10; i < 30; i++) edges.push(e(`a${i}`, `a${i - 1}`)); // 其他雜邊（不碰 hub）
  const arch: Architecture = {
    layers: [{ id: "a", name: "A" }, { id: "b", name: "B" }, { id: "c", name: "C" }],
    modules, edges,
    meta: { scannedAt: "", gitHead: null, analyzed: false, dropped: 0 },
  };
  return arch;
})();

describe("neighborhood", () => {
  it("只含自己、直接鄰居，以及碰到自己的連線（一跳）", () => {
    const n = neighborhood(dense, "hub");
    expect([...n.modules].sort()).toEqual(["a0", "a1", "a2", "a3", "a4", "a5", "b0", "b1", "b2", "b3", "hub", "x1"]);
    expect(n.edges).toHaveLength(11);
    expect(n.edges.every((x) => x.from === "hub" || x.to === "hub")).toBe(true);
  });
  it("傳遞閉包上的節點（兩步以外）不在內", () => {
    const n = neighborhood(dense, "hub");
    expect(n.modules.has("x2")).toBe(false);
    expect(n.modules.has("a12")).toBe(false);
  });
  it("葉節點只有自己與兩個鄰居", () => {
    const n = neighborhood(dense, "x2");
    expect([...n.modules].sort()).toEqual(["b0", "x1", "x2"]);
    expect(n.edges).toHaveLength(2);
  });
  it("不存在的 id：只回傳自己、沒有連線", () => {
    const n = neighborhood(dense, "nope");
    expect([...n.modules]).toEqual(["nope"]);
    expect(n.edges).toEqual([]);
  });
});

describe("稠密圖版面", () => {
  const l = layoutGraph(dense);
  it("沒有摘要與技術標籤的是精簡卡，其餘是完整卡", () => {
    expect(isCompact(dense.modules.find((m) => m.id === "a1")!)).toBe(true);
    expect(isCompact(dense.modules.find((m) => m.id === "a0")!)).toBe(false);
    expect(l.nodes.a1.h).toBe(NODE_H_COMPACT);
    expect(l.nodes.a0.h).toBe(NODE_H);
    expect(l.nodes.hub.h).toBe(NODE_H);
    expect(NODE_H_COMPACT).toBeLessThan(NODE_H / 2);
  });
  it("每個泳道內，同一子欄最多 MAX_ROWS_PER_COL 張卡", () => {
    for (const lane of l.columns) {
      const byX = new Map<number, number>();
      for (const m of dense.modules.filter((m) => m.layer === lane.layerId)) byX.set(l.nodes[m.id].x, (byX.get(l.nodes[m.id].x) ?? 0) + 1);
      expect(Math.max(...byX.values())).toBeLessThanOrEqual(MAX_ROWS_PER_COL);
    }
    const subcols = (layer: string) => new Set(dense.modules.filter((m) => m.layer === layer).map((m) => l.nodes[m.id].x)).size;
    expect([subcols("a"), subcols("b"), subcols("c")]).toEqual([3, 2, 1]);
  });
  it("泳道依子欄數加寬，所有卡片都在自己泳道內，相鄰泳道不重疊", () => {
    const laneA = l.columns[0];
    expect(laneA.w).toBe(LANE_PAD * 2 + 3 * NODE_W + 2 * SUBCOL_GAP);
    expect(l.columns[1].w).toBe(LANE_PAD * 2 + 2 * NODE_W + SUBCOL_GAP);
    expect(l.columns[2].w).toBe(LANE_W);
    for (const lane of l.columns) {
      for (const m of dense.modules.filter((m) => m.layer === lane.layerId)) {
        const r = l.nodes[m.id];
        expect(r.x).toBeGreaterThanOrEqual(lane.x + LANE_PAD);
        expect(r.x + r.w).toBeLessThanOrEqual(lane.x + lane.w - LANE_PAD);
        expect(r.y).toBeGreaterThanOrEqual(lane.y + HEADER_H);
        expect(r.y + r.h).toBeLessThanOrEqual(lane.y + lane.h - LANE_PAD);
      }
    }
    expect(l.columns[1].x).toBe(laneA.x + laneA.w + COL_GAP);
    expect(l.width).toBe(PAD * 2 + l.columns.reduce((s, c) => s + c.w, 0) + 2 * COL_GAP);
  });
  it("同一子欄內的卡片不互相重疊", () => {
    const rects = Object.values(l.nodes);
    for (const a of rects) for (const b of rects) {
      if (a === b || a.x !== b.x) continue;
      expect(a.y + a.h + ROW_GAP <= b.y || b.y + b.h + ROW_GAP <= a.y).toBe(true);
    }
  });
  it("所有泳道等高；整體高度遠低於「全部疊一欄用完整卡」", () => {
    expect(new Set(l.columns.map((c) => c.h)).size).toBe(1);
    expect(l.height).toBeLessThan(PAD * 2 + HEADER_H + 30 * (NODE_H + ROW_GAP));
  });
});

describe("spreadLabels", () => {
  it("三個幾乎同一點的標籤沿 y 錯開至少 minDist，順序與 id 不變", () => {
    const out = spreadLabels([
      { id: "a", x: 100, y: 50 },
      { id: "b", x: 101, y: 51 },
      { id: "c", x: 100, y: 50.5 },
    ], 20);
    expect(out.map((p) => p.id)).toEqual(["a", "b", "c"]);
    const ys = [...out].sort((p, q) => p.y - q.y).map((p) => p.y);
    expect(ys[1] - ys[0]).toBeGreaterThanOrEqual(20);
    expect(ys[2] - ys[1]).toBeGreaterThanOrEqual(20);
    // 最上面那個不動（只往下推）
    expect(Math.min(...ys)).toBe(50);
  });

  it("相同位置時依輸入順序由上往下排（穩定）", () => {
    const out = spreadLabels([{ id: "a", x: 0, y: 0 }, { id: "b", x: 0, y: 0 }, { id: "c", x: 0, y: 0 }], 10);
    expect(out.map((p) => p.y)).toEqual([0, 10, 20]);
  });

  it("彼此夠遠的標籤原封不動（含 x 差很遠但 y 相同者）", () => {
    const pts = [{ id: "a", x: 0, y: 0 }, { id: "b", x: 1000, y: 0 }, { id: "c", x: 0, y: 100 }];
    expect(spreadLabels(pts, 20)).toEqual(pts);
  });

  it("不改動輸入陣列", () => {
    const pts = [{ id: "a", x: 0, y: 0 }, { id: "b", x: 0, y: 0 }];
    spreadLabels(pts, 10);
    expect(pts[1].y).toBe(0);
  });
});

describe("clampZoom", () => {
  it("限制在 MIN_ZOOM–MAX_ZOOM", () => {
    expect(clampZoom(0.01)).toBe(MIN_ZOOM);
    expect(clampZoom(9)).toBe(2.5);
    expect(clampZoom(1.2)).toBe(1.2);
  });
});

describe("zoomAt", () => {
  it("以游標為中心縮放：游標下的內容座標不變", () => {
    const v = { scale: 1, tx: 10, ty: 20 };
    const r = zoomAt(v, 2, 110, 220);
    expect(r.scale).toBe(2);
    // 內容座標 (110-10)/1 = 100 → 縮放後仍在 110
    expect(110 - r.tx).toBeCloseTo(100 * r.scale);
    expect(220 - r.ty).toBeCloseTo(200 * r.scale);
  });

  it("縮放結果被夾在範圍內，且夾住後游標下內容座標仍不變", () => {
    const r = zoomAt({ scale: 2, tx: 0, ty: 0 }, 10, 50, 80);
    expect(r.scale).toBe(2.5);
    expect(50 - r.tx).toBeCloseTo((50 / 2) * 2.5);
    expect(80 - r.ty).toBeCloseTo((80 / 2) * 2.5);
    const lo = zoomAt({ scale: 0.4, tx: 5, ty: 5 }, 0.1, 0, 0);
    expect(lo.scale).toBe(MIN_ZOOM);
  });
});

describe("centerOffset", () => {
  it("內容矮於視窗時回傳垂直置中的位移，否則 0", () => {
    expect(centerOffset(200, 500)).toBe(150);
    expect(centerOffset(500, 500)).toBe(0);
    expect(centerOffset(800, 500)).toBe(0);
  });
});

describe("三區排版（主流程／共用底座／支援週邊）", () => {
  const mod = (id: string, layer: string): Module => ({ id, layer, name: id, files: [], tech: [], summary: "", dataflow: "" });
  const zoned: Architecture = {
    layers: [
      { id: "fe", name: "前端" }, { id: "be", name: "後端" },
      { id: "dom", name: "領域模型" }, { id: "bld", name: "建置工具" },
    ],
    modules: [mod("f1", "fe"), mod("b1", "be"), mod("b2", "be"), mod("d1", "dom"), mod("d2", "dom"), mod("t1", "bld"), mod("t2", "bld"), mod("t3", "bld")],
    edges: [],
    meta: { scannedAt: "", gitHead: null, analyzed: true, dropped: 0 },
  };
  it("columns 仍依 layers 順序；角色與區域標題都帶出來", () => {
    const l = layoutGraph(zoned);
    expect(l.columns.map((c) => [c.layerId, c.role])).toEqual([["fe", "main"], ["be", "main"], ["dom", "shared"], ["bld", "support"]]);
    expect(l.zones.map((z) => z.role)).toEqual(["shared", "support"]);
  });
  it("主流程泳道高度只看主流程自己的卡片，不被週邊那條拉高", () => {
    const l = layoutGraph(zoned);
    const mainH = HEADER_H + 2 * NODE_H_COMPACT + ROW_GAP + LANE_PAD;
    expect(l.columns[0].h).toBe(mainH);
    expect(l.columns[1].h).toBe(mainH);
  });
  it("共用底座在主流程正下方，寬度只夠卡片橫排（不超過主流程全寬）；支援週邊在最下面", () => {
    const l = layoutGraph(zoned);
    const mainBottom = PAD + l.columns[0].h;
    const dom = l.columns[2];
    expect(dom.x).toBe(PAD);
    expect(dom.w).toBe(LANE_PAD * 2 + 2 * NODE_W + SUBCOL_GAP);
    expect(dom.w).toBeLessThanOrEqual(l.columns[1].x + l.columns[1].w - PAD);
    expect(dom.y).toBe(mainBottom + ZONE_GAP + ZONE_CAPTION_H);
    expect(l.zones[0].y).toBe(mainBottom + ZONE_GAP);
    expect(l.nodes.d1.y).toBe(dom.y + BAND_HEADER_H);
    expect(l.nodes.d2.y).toBe(l.nodes.d1.y);
    expect(l.nodes.d2.x).toBe(l.nodes.d1.x + NODE_W + SUBCOL_GAP);
    expect(l.nodes.d1.zone).toBe("shared");
    const bld = l.columns[3];
    expect(bld.y).toBeGreaterThan(dom.y + dom.h);
    expect(l.nodes.t1.zone).toBe("support");
    expect(l.nodes.f1.zone).toBeUndefined();
    expect(l.height).toBe(bld.y + bld.h + PAD);
  });
  it("沒有共用／支援層時與單排相同：zones 為空", () => {
    expect(layoutGraph(arch).zones).toEqual([]);
  });
  it("一層主流程都沒有時全部當主流程，畫面不會只剩週邊", () => {
    const l = layoutGraph({ ...zoned, layers: [{ id: "dom", name: "領域模型" }, { id: "bld", name: "建置工具" }], modules: zoned.modules.filter((m) => m.layer === "dom" || m.layer === "bld") });
    expect(l.columns.every((c) => c.role === "main")).toBe(true);
    expect(l.zones).toEqual([]);
  });
  it("跨區連線從下緣出、上緣入（垂直走線）；同區仍是側邊到側邊", () => {
    const l = layoutGraph(zoned);
    const nums = (d: string) => d.match(/-?\d+(\.\d+)?/g)!.map(Number);
    const down = nums(edgePath(l.nodes.b1, l.nodes.d1));
    expect(down[1]).toBe(l.nodes.b1.y + l.nodes.b1.h);
    expect(down[down.length - 1]).toBe(l.nodes.d1.y);
    const up = nums(edgePath(l.nodes.d1, l.nodes.f1));
    expect(up[1]).toBe(l.nodes.d1.y);
    const side = edgePath(l.nodes.f1, l.nodes.b1);
    expect(side.startsWith(`M ${l.nodes.f1.x + NODE_W} `)).toBe(true);
  });
  it("跨區連線兩端落在卡片寬度之內，且水平位移不超過兩卡中心的距離（曲線才接得正）", () => {
    const l = layoutGraph(zoned);
    for (const [from, to] of [[l.nodes.b1, l.nodes.d1], [l.nodes.d1, l.nodes.f1]]) {
      const n = edgePath(from, to).match(/-?\d+(\.\d+)?/g)!.map(Number);
      const [x1, x2] = [n[0], n[n.length - 2]];
      expect(x1).toBeGreaterThanOrEqual(from.x);
      expect(x1).toBeLessThanOrEqual(from.x + from.w);
      expect(x2).toBeGreaterThanOrEqual(to.x);
      expect(x2).toBeLessThanOrEqual(to.x + to.w);
      expect(Math.abs(x2 - x1)).toBeLessThanOrEqual(Math.abs(to.x + to.w / 2 - (from.x + from.w / 2)));
    }
  });
});

describe("共用底座寬度", () => {
  const mod = (id: string): Module => ({ id, layer: "dom", name: id, files: [], tech: [], summary: "", dataflow: "" });
  const mk = (n: number): Architecture => ({
    layers: [{ id: "fe", name: "前端" }, { id: "dom", name: "領域模型" }],
    modules: [{ ...mod("f"), layer: "fe" }, ...Array.from({ length: n }, (_, i) => mod(`d${i}`))],
    edges: [], meta: { scannedAt: "", gitHead: null, analyzed: true, dropped: 0 },
  });
  it("一張卡時最窄只有一條泳道寬", () => {
    expect(layoutGraph(mk(1)).columns[1].w).toBe(LANE_W);
  });
  it("卡片多到放不進主流程寬度時換行，寬度不超過主流程全寬", () => {
    const l = layoutGraph(mk(5));
    expect(l.columns[1].w).toBe(l.width - PAD * 2);
    expect(l.nodes.d4.y).toBeGreaterThan(l.nodes.d0.y);
  });
});
