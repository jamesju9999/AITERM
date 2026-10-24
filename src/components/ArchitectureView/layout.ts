import { layerRole, type Architecture, type Edge, type LayerRole, type Module } from "./model";

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
/** 橫向的共用底座標題列高度（比直向泳道矮，因為卡片是橫排）。 */
export const BAND_HEADER_H = 40;
/** 主流程、共用底座、支援週邊三區之間的垂直間隙（跨區連線走這裡）。 */
export const ZONE_GAP = 56;
/** 共用底座／支援週邊的區域小標題高度。 */
export const ZONE_CAPTION_H = 22;
/** 同欄連線向右凸出的距離。 */
export const EDGE_BULGE = 56;

export interface Pt { x: number; y: number }
/** zone 只有不在主流程區的卡片才有；沒標＝主流程。跨區的連線改走垂直方向。 */
export interface Rect { x: number; y: number; w: number; h: number; zone?: LayerRole }
export interface Lane { layerId: string; name: string; role: LayerRole; x: number; y: number; w: number; h: number; count: number }
/** 區域小標題的位置：只有共用底座與支援週邊會有。 */
export interface Zone { role: "shared" | "support"; x: number; y: number; w: number }
export interface Layout {
  nodes: Record<string, Rect>;
  columns: Lane[];
  zones: Zone[];
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

/**
 * 三區排版：主流程各層由左到右排成一排；共用底座（領域模型等）是主流程下方的帶狀區（寬度隨卡片數，最寬到主流程全寬）；
 * 支援週邊（建置、測試、報表…）在最下方另排一列。沒有共用／支援層時與單排完全相同。
 * columns 仍依 arch.layers 的順序，顏色與索引不受分區影響。
 */
export function layoutGraph(arch: Architecture): Layout {
  const nodes: Record<string, Rect> = {};
  const heightOf = (m: Module) => (isCompact(m) ? NODE_H_COMPACT : NODE_H);
  const rawRoles = arch.layers.map(layerRole);
  // 一層主流程都沒有時全部當主流程，免得畫面只剩週邊。
  const roles = rawRoles.includes("main") ? rawRoles : rawRoles.map((): LayerRole => "main");
  const subcols = arch.layers.map((l) => chunkColumns(arch.modules.filter((m) => m.layer === l.id)));
  const colH = (col: Module[]) => col.reduce((s, m) => s + heightOf(m), 0) + Math.max(0, col.length - 1) * ROW_GAP;
  const laneW = (i: number) => { const k = Math.max(1, subcols[i].length); return LANE_PAD * 2 + k * NODE_W + (k - 1) * SUBCOL_GAP; };
  const idxOf = (role: LayerRole) => arch.layers.map((_, i) => i).filter((i) => roles[i] === role);
  const rowH = (idx: number[]) => HEADER_H + Math.max(0, ...idx.flatMap((i) => subcols[i].map(colH))) + LANE_PAD;
  const rowW = (idx: number[]) => idx.reduce((s, i) => s + laneW(i), 0) + Math.max(0, idx.length - 1) * COL_GAP;

  const main = idxOf("main");
  const shared = idxOf("shared");
  const support = idxOf("support");
  const contentInnerW = Math.max(rowW(main), rowW(support));
  const lanes: Lane[] = new Array(arch.layers.length);
  const zones: Zone[] = [];

  /** 直向泳道一排：每條泳道自己往下排卡片，整排等高。 */
  const placeRow = (idx: number[], top: number, role: LayerRole): number => {
    const h = rowH(idx);
    let x = PAD;
    for (const i of idx) {
      const w = laneW(i);
      subcols[i].forEach((col, c) => {
        let y = top + HEADER_H;
        for (const m of col) {
          const mh = heightOf(m);
          nodes[m.id] = { x: x + LANE_PAD + c * (NODE_W + SUBCOL_GAP), y, w: NODE_W, h: mh, ...(role === "main" ? {} : { zone: role }) };
          y += mh + ROW_GAP;
        }
      });
      lanes[i] = { layerId: arch.layers[i].id, name: arch.layers[i].name, role, x, y: top, w, h, count: subcols[i].reduce((s, c) => s + c.length, 0) };
      x += w + COL_GAP;
    }
    return h;
  };

  let cursor = PAD;
  if (main.length > 0) cursor += placeRow(main, cursor, "main");

  if (shared.length > 0) {
    const fullW = Math.max(LANE_W, contentInnerW);
    // 帶狀區寬度只夠放自己的卡片排成一列（放不下才換行、最寬到主流程全寬），免得一張卡孤零零躺在整條空帶子裡。
    const bandWOf = (n: number) => Math.min(fullW, Math.max(LANE_W, LANE_PAD * 2 + n * NODE_W + (n - 1) * SUBCOL_GAP));
    cursor += ZONE_GAP;
    zones.push({ role: "shared", x: PAD, y: cursor, w: Math.max(...shared.map((i) => bandWOf(subcols[i].flat().length))) });
    cursor += ZONE_CAPTION_H;
    for (const i of shared) {
      const mods = subcols[i].flat();
      const bandW = bandWOf(mods.length);
      const perRow = Math.max(1, Math.floor((bandW - LANE_PAD * 2 + SUBCOL_GAP) / (NODE_W + SUBCOL_GAP)));
      const rows: Module[][] = [];
      for (let k = 0; k < mods.length; k += perRow) rows.push(mods.slice(k, k + perRow));
      let y = cursor + BAND_HEADER_H;
      for (const row of rows) {
        const rh = Math.max(...row.map(heightOf));
        row.forEach((m, c) => { nodes[m.id] = { x: PAD + LANE_PAD + c * (NODE_W + SUBCOL_GAP), y, w: NODE_W, h: heightOf(m), zone: "shared" }; });
        y += rh + ROW_GAP;
      }
      const h = BAND_HEADER_H + Math.max(0, y - ROW_GAP - (cursor + BAND_HEADER_H)) + LANE_PAD;
      lanes[i] = { layerId: arch.layers[i].id, name: arch.layers[i].name, role: "shared", x: PAD, y: cursor, w: bandW, h, count: mods.length };
      cursor += h + ROW_GAP;
    }
    cursor -= ROW_GAP;
  }

  if (support.length > 0) {
    cursor += ZONE_GAP;
    zones.push({ role: "support", x: PAD, y: cursor, w: Math.max(LANE_W, contentInnerW) });
    cursor += ZONE_CAPTION_H;
    cursor += placeRow(support, cursor, "support");
  }

  return {
    nodes,
    columns: lanes,
    zones,
    width: Math.max(PAD * 2, PAD * 2 + contentInnerW),
    height: cursor + PAD,
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

/** 真適合：整個內容縮進視窗（可低到 MIN_ZOOM）並置中。「適合視窗」鈕用。 */
export function trueFitView(contentW: number, contentH: number, viewW: number, viewH: number): View {
  const scale = fitScale(contentW, contentH, viewW, viewH);
  return { scale, tx: centerOffset(contentW * scale, viewW), ty: centerOffset(contentH * scale, viewH) };
}

/**
 * 自動（初次載入、換層級）視圖：真適合若小於 READABLE_SCALE 就停在 READABLE_SCALE，字才讀得出來；
 * 放不下的那一軸靠左上對齊（centerOffset 對溢出回 0），其餘靠使用者平移。
 */
export function initialView(contentW: number, contentH: number, viewW: number, viewH: number): View {
  const fit = fitScale(contentW, contentH, viewW, viewH);
  const scale = fit < READABLE_SCALE ? READABLE_SCALE : fit;
  return { scale, tx: centerOffset(contentW * scale, viewW), ty: centerOffset(contentH * scale, viewH) };
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
  if ((from.zone ?? "main") !== (to.zone ?? "main")) {
    // 跨區（主流程↔共用底座↔支援週邊）：區與區上下排列，從下緣出、上緣入。
    const down = to.y >= from.y;
    // 兩端的 x 取彼此最近的點（重疊就取重疊區中央），水平位移小，曲線才會幾乎垂直地接上卡片邊緣、箭頭才正。
    const m = Math.min(24, from.w / 4, to.w / 4);
    const lo = Math.max(from.x + m, to.x + m);
    const hi = Math.min(from.x + from.w - m, to.x + to.w - m);
    const near = (r: Rect, other: number) => Math.min(Math.max(other, r.x + m), r.x + r.w - m);
    const x1 = lo <= hi ? (lo + hi) / 2 : near(from, to.x + to.w / 2);
    const x2 = lo <= hi ? x1 : near(to, from.x + from.w / 2);
    const y1 = down ? from.y + from.h : from.y;
    const y2 = down ? to.y : to.y + to.h;
    const dy = Math.max(40, Math.abs(y2 - y1) / 2) * (down ? 1 : -1);
    return [{ x: x1, y: y1 }, { x: x1, y: y1 + dy }, { x: x2, y: y2 - dy }, { x: x2, y: y2 }];
  }
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

export const MIN_ZOOM = 0.2;
/** 自動視圖的易讀下限：真適合比這小就停在這個比例，其餘靠平移。 */
export const READABLE_SCALE = 0.55;
export const MAX_ZOOM = 2.5;
/** ＋／－按鈕每按一次的縮放倍率。 */
export const ZOOM_STEP = 1.25;
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
