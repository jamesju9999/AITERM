import type { Architecture, Edge, Module } from "./model";

export const NODE_W = 200;
export const NODE_H = 112;
/** 精簡卡高度：一行名稱＋檔案數徽章。 */
export const NODE_H_COMPACT = 44;
/** 泳道內每個子欄最多幾張卡；超過就往右多開一欄。 */
export const MAX_ROWS_PER_COL = 10;
/** 同一泳道內子欄之間的間隙（同欄連線的凸出弧線走這裡）。 */
export const SUBCOL_GAP = 40;
/** 泳道與泳道之間的間隙（同欄連線的凸出弧線就走這裡）。 */
export const COL_GAP = 64;
export const ROW_GAP = 14;
export const PAD = 24;
/** 泳道內側留白（卡片與泳道邊框的距離）。 */
export const LANE_PAD = 14;
/** 泳道標題列高度（含與第一張卡片之間的空隙）。 */
export const HEADER_H = 54;
export const LANE_W = NODE_W + LANE_PAD * 2;
/** 同欄連線向右凸出的距離。 */
export const EDGE_BULGE = 56;

export interface Pt { x: number; y: number }
export interface Rect { x: number; y: number; w: number; h: number }
export interface Lane { layerId: string; name: string; x: number; y: number; w: number; h: number; count: number }
export interface Layout {
  nodes: Record<string, Rect>;
  columns: Lane[];
  width: number;
  height: number;
}

/** 沒有摘要也沒有技術標籤的模組（多半是單一檔案）用精簡卡：一行名稱＋檔案數。 */
export function isCompact(m: Module): boolean {
  return !m.summary && m.tech.length === 0;
}

/** 把一層的模組切成幾個子欄：每欄最多 MAX_ROWS_PER_COL 張，欄數相同時盡量平均。 */
function chunkColumns<T>(items: T[]): T[][] {
  if (items.length === 0) return [];
  const k = Math.ceil(items.length / MAX_ROWS_PER_COL);
  const per = Math.ceil(items.length / k);
  return Array.from({ length: k }, (_, i) => items.slice(i * per, (i + 1) * per)).filter((c) => c.length > 0);
}

export function layoutGraph(arch: Architecture): Layout {
  const nodes: Record<string, Rect> = {};
  const heightOf = (m: Module) => (isCompact(m) ? NODE_H_COMPACT : NODE_H);
  const subcols = arch.layers.map((l) => chunkColumns(arch.modules.filter((m) => m.layer === l.id)));
  const colH = (col: Module[]) => col.reduce((s, m) => s + heightOf(m), 0) + Math.max(0, col.length - 1) * ROW_GAP;
  const innerH = Math.max(0, ...subcols.flat().map(colH));
  const laneH = HEADER_H + innerH + LANE_PAD;
  let x = PAD;
  const columns: Lane[] = arch.layers.map((layer, i) => {
    const cols = subcols[i];
    const k = Math.max(1, cols.length);
    const w = LANE_PAD * 2 + k * NODE_W + (k - 1) * SUBCOL_GAP;
    cols.forEach((col, c) => {
      let y = PAD + HEADER_H;
      for (const m of col) {
        const h = heightOf(m);
        nodes[m.id] = { x: x + LANE_PAD + c * (NODE_W + SUBCOL_GAP), y, w: NODE_W, h };
        y += h + ROW_GAP;
      }
    });
    const lane: Lane = { layerId: layer.id, name: layer.name, x, y: PAD, w, h: laneH, count: cols.reduce((s, c) => s + c.length, 0) };
    x += w + COL_GAP;
    return lane;
  });
  return {
    nodes,
    columns,
    width: Math.max(PAD * 2, x - COL_GAP + PAD),
    height: PAD * 2 + laneH,
  };
}

/**
 * 內容放不進可視區時等比縮小：寬、高各算一個比例取較小者，夾在 [MIN_ZOOM, 1]。
 * 只看寬度的舊版會讓很高的圖底部被切掉；量不到尺寸時不縮。
 */
export function fitScale(contentW: number, contentH: number, viewW: number, viewH: number): number {
  if (contentW <= 0 || contentH <= 0 || viewW <= 0 || viewH <= 0) return 1;
  return Math.min(1, Math.max(MIN_ZOOM, Math.min(viewW / contentW, viewH / contentH)));
}

const FIXED_LAYER_COLORS: Record<string, string> = {
  frontend: "#38bdf8",
  transport: "#a78bfa",
  backend: "#34d399",
  data: "#fbbf24",
  other: "#94a3b8",
};
export const LAYER_PALETTE = ["#f472b6", "#fb923c", "#2dd4bf", "#818cf8", "#a3e635", "#f87171"];

/** 分層的固定代表色；AI 自訂的分層依序號從調色盤取。 */
export function layerColor(layerId: string, index: number): string {
  return FIXED_LAYER_COLORS[layerId] ?? LAYER_PALETTE[((index % LAYER_PALETTE.length) + LAYER_PALETTE.length) % LAYER_PALETTE.length];
}

function edgeGeom(from: Rect, to: Rect): [Pt, Pt, Pt, Pt] {
  const y1 = from.y + from.h / 2;
  const y2 = to.y + to.h / 2;
  if (from.x === to.x) {
    // 同欄：兩端都從右緣出入，向右凸出到泳道間隙，不會穿過同欄其他卡片。
    const x = from.x + from.w;
    return [{ x, y: y1 }, { x: x + EDGE_BULGE, y: y1 }, { x: x + EDGE_BULGE, y: y2 }, { x, y: y2 }];
  }
  const fwd = to.x > from.x;
  const x1 = fwd ? from.x + from.w : from.x;
  const x2 = fwd ? to.x : to.x + to.w;
  const dx = Math.max(40, Math.abs(x2 - x1) / 2) * (fwd ? 1 : -1);
  return [{ x: x1, y: y1 }, { x: x1 + dx, y: y1 }, { x: x2 - dx, y: y2 }, { x: x2, y: y2 }];
}

/** 不同欄：側邊到側邊的貝茲曲線；同欄：向右凸出的弧線。 */
export function edgePath(from: Rect, to: Rect): string {
  const [a, b, c, d] = edgeGeom(from, to);
  return `M ${a.x} ${a.y} C ${b.x} ${b.y} ${c.x} ${c.y} ${d.x} ${d.y}`;
}

/** 曲線 t=0.5 的位置，用來放連線標籤。 */
export function edgeMidpoint(from: Rect, to: Rect): Pt {
  const [a, b, c, d] = edgeGeom(from, to);
  return { x: (a.x + 3 * b.x + 3 * c.x + d.x) / 8, y: (a.y + 3 * b.y + 3 * c.y + d.y) / 8 };
}

/** 曲線長度的近似（控制點折線長度與弦長的平均），光點動畫時長用。 */
export function edgeLength(from: Rect, to: Rect): number {
  const pts = edgeGeom(from, to);
  const d = (p: Pt, q: Pt) => Math.hypot(q.x - p.x, q.y - p.y);
  return (d(pts[0], pts[1]) + d(pts[1], pts[2]) + d(pts[2], pts[3]) + d(pts[0], pts[3])) / 2;
}

/** 某模組的一跳鄰域：它自己、直接相連的模組，以及碰到它的連線。傳遞閉包只靠資料流路徑顯示。 */
export function neighborhood(arch: Architecture, id: string): { modules: Set<string>; edges: Edge[] } {
  const modules = new Set<string>([id]);
  const edges: Edge[] = [];
  for (const e of arch.edges) {
    if (e.from !== id && e.to !== id) continue;
    edges.push(e);
    modules.add(e.from);
    modules.add(e.to);
  }
  return { modules, edges };
}

export interface LabelPt { id: string; x: number; y: number }

/**
 * 重疊的連線標籤沿 y 往下錯開到至少 minDist。
 * 取捨：標籤寬約高的 3 倍，所以只有 |dx| < 3*minDist 才算會互相遮住；
 * 依 (y, 輸入順序) 由上往下處理，最上面的不動；回傳順序與輸入一致。
 */
export function spreadLabels(points: LabelPt[], minDist: number): LabelPt[] {
  const order = points.map((_, i) => i).sort((a, b) => points[a].y - points[b].y || a - b);
  const placed: LabelPt[] = [];
  const out: LabelPt[] = new Array(points.length);
  for (const i of order) {
    const p = points[i];
    let y = p.y;
    for (const q of placed) {
      if (Math.abs(q.x - p.x) < minDist * 3 && y - q.y < minDist) y = q.y + minDist;
    }
    const np = { ...p, y };
    placed.push(np);
    out[i] = np;
  }
  return out;
}

export const MIN_ZOOM = 0.3;
export const MAX_ZOOM = 2.5;
export interface View { scale: number; tx: number; ty: number }

export function clampZoom(z: number): number {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, z));
}

/** 以 (cx,cy)（視窗座標）為中心縮放；變換為 translate(tx,ty) scale(s)，游標下的內容點保持不動。 */
export function zoomAt(view: View, factor: number, cx: number, cy: number): View {
  const scale = clampZoom(view.scale * factor);
  const k = scale / view.scale;
  return { scale, tx: cx - (cx - view.tx) * k, ty: cy - (cy - view.ty) * k };
}

/** 內容矮於視窗時的垂直置中位移；否則 0。 */
export function centerOffset(contentH: number, viewportH: number): number {
  return Math.max(0, (viewportH - contentH) / 2);
}
