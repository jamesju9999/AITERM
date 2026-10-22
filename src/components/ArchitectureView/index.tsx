import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { nanoid } from "nanoid";
import { useLocale } from "../../contexts/LocaleContext";
import { archLoad, archReadFiles, archSave, archScan, type ScanResult } from "../../ipc/architecture";
import { abortAi, formatAiError, invokeAiComplete, type AiError } from "../../ipc/ai";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { ModelPickerButton } from "../ModelPickerButton";
import { ANALYZE_SYSTEM, buildAnalyzeMessage, mergeScanEdges, parseAnalysis, pickKeyFiles } from "./analyze";
import { buildBaseGraph, type Architecture, type Edge } from "./model";
import { buildDrillGraph, drillKey } from "./drill";
import { centerOffset, edgeLength, edgeMidpoint, edgePath, fitScale, layerColor, layoutGraph, NODE_H, NODE_W, spreadLabels, traceChain, zoomAt, type Layout, type View } from "./layout";
import { DetailPanel } from "./DetailPanel";
import "./index.css";

const PROVIDER_KEY = "aiterm_arch_provider";

/** 鑽取路徑上的一層：模組 id、它的檔案、顯示名稱（麵包屑用）。 */
interface DrillLevel { id: string; files: string[]; name: string }

/** 只含指定檔案、以及兩端都在其中的 import 的掃描結果，讓 AI 細分只看這個模組。 */
function subScanOf(scan: ScanResult, files: string[]): ScanResult {
  const own = new Set(files);
  return {
    ...scan,
    files: scan.files.filter((f) => own.has(f.path)),
    edges: scan.edges.filter((e) => own.has(e.from) && own.has(e.to)),
  };
}

/** 根層重新分析後，只保留仍存在的模組的 drills（鍵的第一段是根層模組 id）。 */
function pruneDrills(drills: Architecture["drills"], moduleIds: Set<string>): Architecture["drills"] {
  if (!drills) return undefined;
  const kept = Object.fromEntries(Object.entries(drills).filter(([k]) => moduleIds.has(k.split("\u0000")[0])));
  return Object.keys(kept).length > 0 ? kept : undefined;
}

/** 使用者要求減少動態時不渲染光點。jsdom 等環境沒有 matchMedia，查不到就當作沒開。 */
function prefersReducedMotion(): boolean {
  try { return !!window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches; } catch { return false; }
}

function samePath(a: Edge[] | null, b: Edge[]): boolean {
  return !!a && a.length === b.length && a.every((e, i) => e === b[i]);
}

/** 標籤互相錯開的最小垂直距離（標籤高約 18px）。 */
const LABEL_GAP = 20;
const DOTS_PER_EDGE = 2;

type Status = { kind: "idle" } | { kind: "busy"; msg: string } | { kind: "error"; msg: string } | { kind: "notice"; msg: string };

/** Tauri IPC 的錯誤常是字串或物件，String(e) 對物件只會得到 [object Object]。 */
function errText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  if (e && typeof e === "object") {
    const m = (e as { message?: unknown }).message;
    if (typeof m === "string") return m;
    try { return JSON.stringify(e); } catch { /* fall through */ }
  }
  return String(e);
}

export function ArchitectureView({ isActive }: { isActive: boolean }) {
  const { t } = useLocale();
  const [root, setRoot] = useState<string | null>(null);
  const [scan, setScan] = useState<ScanResult | null>(null);
  const [arch, setArch] = useState<Architecture | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [drillPath, setDrillPath] = useState<DrillLevel[]>([]);
  const [stale, setStale] = useState(false);
  const [status, setStatus] = useState<Status>({ kind: "idle" });
  const [analyzing, setAnalyzing] = useState(false);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [providerId, setProviderId] = useState("");
  const [elapsed, setElapsed] = useState(0);
  const [hoverEdge, setHoverEdge] = useState<number | null>(null);
  const [viewport, setViewport] = useState({ w: 0, h: 0 });
  // 使用者手動縮放／平移的結果；綁定產生它的 layout，換圖（鑽取、重新分析）就自動回到適合視窗。
  const [manual, setManual] = useState<{ layout: Layout; view: View } | null>(null);
  // 目前選的資料流路徑；綁定當時的圖與選取，換選取或換層級就自動失效。
  const [pathPick, setPathPick] = useState<{ path: Edge[]; view: Architecture; selected: string } | null>(null);
  const [panning, setPanning] = useState(false);
  const streamIdRef = useRef<string | null>(null);
  const runRef = useRef(0);
  const canvasRef = useRef<HTMLDivElement>(null);
  const archRef = useRef<Architecture | null>(null);
  const viewRef = useRef<View>({ scale: 1, tx: 0, ty: 0 });
  const layoutRef = useRef<Layout | null>(null);
  const dragCleanupRef = useRef<(() => void) | null>(null);

  const load = useCallback(async (dir: string) => {
    setRoot(dir);
    setSelected(null);
    setDrillPath([]);
    setStatus({ kind: "busy", msg: t.arch_scanning });
    try {
      const s = await archScan(dir);
      setScan(s);
      const cached = await archLoad(dir);
      if (cached) {
        const parsed = JSON.parse(cached) as Architecture;
        setArch(parsed);
        setStale(parsed.meta.gitHead !== s.git_head);
      } else {
        setArch(buildBaseGraph(s));
        setStale(false);
      }
      setStatus(s.truncated ? { kind: "notice", msg: t.arch_truncated_n.replace("{n}", String(s.files.length)) } : { kind: "idle" });
    } catch (e) {
      setStatus({ kind: "error", msg: t.arch_error.replace("{msg}", errText(e)) });
    }
  }, [t]);

  const pick = useCallback(async () => {
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir === "string") await load(dir);
  }, [load]);

  // 可選的模型：上次記住的 → 預設供應商 → 清單第一個。記住的 id 若已被刪除要退回預設。
  useEffect(() => {
    let alive = true;
    void listProviders().then((list) => {
      if (!alive) return;
      setProviders(list);
      let remembered: string | null = null;
      try { remembered = localStorage.getItem(PROVIDER_KEY); } catch { /* 沒有儲存空間就用預設 */ }
      const fallback = list.find((p) => p.is_default)?.id ?? list[0]?.id ?? "";
      setProviderId(remembered && list.some((p) => p.id === remembered) ? remembered : fallback);
    }).catch(() => { /* 取不到清單就維持預設供應商 */ });
    return () => { alive = false; };
  }, []);

  const changeProvider = useCallback((id: string) => {
    setProviderId(id);
    try { localStorage.setItem(PROVIDER_KEY, id); } catch { /* 記不住不影響使用 */ }
  }, []);

  /**
   * 根層分析與模組細分共用：選供應商、已等時間、取消、錯誤處理都在這裡。
   * target 是要送給 AI 的掃描範圍；apply 把驗證過的結果併進目前的整份架構並回傳新的整份，
   * 這裡負責顯示與存檔。
   */
  const runAnalysis = useCallback(async (target: ScanResult, apply: (merged: Architecture, current: Architecture | null) => Architecture) => {
    if (!root || !scan) return;
    const run = ++runRef.current;
    const streamId = nanoid();
    streamIdRef.current = streamId;
    setElapsed(0);
    setAnalyzing(true);
    setStatus({ kind: "idle" });
    try {
      const base = buildBaseGraph(target);
      const files = await archReadFiles(root, pickKeyFiles(target));
      const reply = await invokeAiComplete(
        [{ role: "user", content: buildAnalyzeMessage(target, base, files) }],
        ANALYZE_SYSTEM,
        streamId,
        providerId || undefined,
      );
      if (run !== runRef.current) return; // 已取消
      const parsed = reply.content ? parseAnalysis(reply.content, target) : null;
      if (!parsed) {
        setStatus({ kind: "error", msg: t.arch_error.replace("{msg}", "AI 回應不是有效的架構 JSON") });
        return;
      }
      const next = apply(mergeScanEdges(parsed, target), archRef.current);
      setArch(next);
      setStale(next.meta.gitHead !== scan.git_head);
      await archSave(root, JSON.stringify(next));
      setStatus(parsed.meta.dropped > 0 ? { kind: "notice", msg: t.arch_dropped.replace("{n}", String(parsed.meta.dropped)) } : { kind: "idle" });
    } catch (e) {
      if (run !== runRef.current) return; // 取消造成的錯誤不顯示
      const err = e as AiError;
      setStatus(err?.kind === "not_configured"
        ? { kind: "notice", msg: t.arch_no_ai }
        : { kind: "error", msg: t.arch_error.replace("{msg}", err?.kind ? formatAiError(err) : errText(e)) });
    } finally {
      if (run === runRef.current) setAnalyzing(false);
    }
  }, [root, scan, t, providerId]);

  const analyze = useCallback(() => {
    if (!scan) return Promise.resolve();
    return runAnalysis(scan, (merged, current) => {
      // 根圖被取代：舊 drills 只留模組 id 仍存在的；鑽取位置可能失效，回到根層。
      setDrillPath([]);
      setSelected(null);
      const drills = pruneDrills(current?.drills, new Set(merged.modules.map((m) => m.id)));
      return drills ? { ...merged, drills } : merged;
    });
  }, [scan, runAnalysis]);

  const analyzeDrill = useCallback(() => {
    const level = drillPath[drillPath.length - 1];
    if (!scan || !level) return Promise.resolve();
    const key = drillKey(drillPath.map((d) => d.id));
    return runAnalysis(subScanOf(scan, level.files), (merged, current) => {
      if (!current) return merged;
      return { ...current, drills: { ...current.drills, [key]: merged } };
    });
  }, [scan, drillPath, runAnalysis]);

  const cancelAnalyze = useCallback(() => {
    runRef.current++;
    setAnalyzing(false);
    setStatus({ kind: "idle" });
    const id = streamIdRef.current;
    if (id) abortAi(id).catch(() => { /* 取消本身失敗也不打擾使用者 */ });
  }, []);

  useEffect(() => {
    if (!analyzing) return;
    const timer = setInterval(() => setElapsed((n) => n + 1), 1000);
    return () => clearInterval(timer);
  }, [analyzing]);

  useEffect(() => {
    const el = canvasRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver((entries) => {
      const r = entries[0]?.contentRect;
      setViewport({ w: r?.width ?? 0, h: r?.height ?? 0 });
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [arch]);

  useEffect(() => { archRef.current = arch; }, [arch]);

  const drillKeyNow = drillKey(drillPath.map((d) => d.id));
  // 目前顯示的圖：根圖，或鑽取層（AI 細分結果優先，否則由掃描即時產生）。
  const drilledFiles = drillPath.length > 0 ? drillPath[drillPath.length - 1].files : null;
  const savedDrill = arch?.drills?.[drillKeyNow];
  const built = useMemo(() => (drilledFiles && scan ? buildDrillGraph(scan, drilledFiles) : null), [scan, drilledFiles]);
  const view = !arch ? null : !drilledFiles ? arch : savedDrill ?? built?.arch ?? null;
  const hiddenCount = drilledFiles && !savedDrill ? built?.hidden ?? 0 : 0;
  const hasAiDrill = !!savedDrill;

  const goTo = useCallback((depth: number) => {
    setDrillPath((p) => p.slice(0, depth));
    setSelected(null);
    setHoverEdge(null);
  }, []);
  const enter = useCallback((id: string) => {
    const m = view?.modules.find((x) => x.id === id);
    if (!m || m.files.length <= 1) return; // 單一檔案不能再鑽
    setDrillPath((p) => [...p, { id: m.id, files: m.files, name: m.name }]);
    setSelected(null);
    setHoverEdge(null);
  }, [view]);

  const layout = useMemo(() => (view ? layoutGraph(view) : null), [view]);
  const chain = useMemo(() => (view && selected ? traceChain(view, selected) : null), [view, selected]);

  const fit: View = layout
    ? (() => { const s = fitScale(layout.width, viewport.w); return { scale: s, tx: 0, ty: centerOffset(layout.height * s, viewport.h) }; })()
    : { scale: 1, tx: 0, ty: 0 };
  const cur: View = manual && manual.layout === layout ? manual.view : fit;
  const activePath = pathPick && pathPick.view === view && pathPick.selected === selected ? pathPick.path : null;
  const hl = useMemo(() => {
    if (activePath) {
      const modules = new Set<string>();
      for (const e of activePath) { modules.add(e.from); modules.add(e.to); }
      return { edges: new Set<Edge>(activePath), modules };
    }
    return chain ? { edges: new Set<Edge>(chain.edges), modules: chain.modules } : null;
  }, [activePath, chain]);
  const reduced = prefersReducedMotion();
  const pickPath = useCallback((p: Edge[]) => {
    if (!view || !selected) return;
    setPathPick((cur0) => (cur0 && cur0.view === view && cur0.selected === selected && samePath(cur0.path, p) ? null : { path: p, view, selected }));
  }, [view, selected]);
  const fitView = useCallback(() => setManual(null), []);
  useEffect(() => { viewRef.current = cur; layoutRef.current = layout; });
  useEffect(() => () => dragCleanupRef.current?.(), []);

  // Ctrl/⌘＋滾輪以游標為中心縮放；一般滾輪維持捲動。要 preventDefault 就必須是非 passive 的原生監聽。
  useEffect(() => {
    const el = canvasRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey && !e.metaKey) return;
      const layoutNow = layoutRef.current;
      if (!layoutNow) return;
      e.preventDefault();
      const r = el.getBoundingClientRect();
      const factor = Math.exp(-e.deltaY * 0.0015);
      setManual({ layout: layoutNow, view: zoomAt(viewRef.current, factor, e.clientX - r.left + el.scrollLeft, e.clientY - r.top + el.scrollTop) });
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  const startPan = (e: React.MouseEvent) => {
    if (e.button !== 0 || !layoutRef.current) return;
    if ((e.target as Element).closest(".arch-node, .arch-fit, .arch-empty")) return;
    const layoutNow = layoutRef.current;
    const start = { x: e.clientX, y: e.clientY, view: viewRef.current };
    const move = (m: MouseEvent) => setManual({ layout: layoutNow, view: { ...start.view, tx: start.view.tx + m.clientX - start.x, ty: start.view.ty + m.clientY - start.y } });
    const stop = () => { window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", stop); dragCleanupRef.current = null; setPanning(false); };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", stop);
    dragCleanupRef.current = stop;
    setPanning(true);
  };

  const labelPts = useMemo(() => {
    if (!view || !layout) return [];
    const pts: { id: string; x: number; y: number }[] = [];
    view.edges.forEach((e, i) => {
      const a = layout.nodes[e.from];
      const b = layout.nodes[e.to];
      if (!a || !b || !(hl?.edges.has(e) || hoverEdge === i)) return;
      const m = edgeMidpoint(a, b);
      pts.push({ id: String(i), x: m.x, y: m.y });
    });
    return spreadLabels(pts, LABEL_GAP);
  }, [view, layout, hl, hoverEdge]);
  const layerIndex = (id: string) => Math.max(0, view?.layers.findIndex((l) => l.id === id) ?? 0);
  const mmss = `${Math.floor(elapsed / 60)}:${String(elapsed % 60).padStart(2, "0")}`;
  const pickIcon = (
    <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.4" aria-hidden="true">
      <rect x="3" y="4" width="7" height="6" rx="1.5" /><rect x="14" y="4" width="7" height="6" rx="1.5" /><rect x="8.5" y="14" width="7" height="6" rx="1.5" />
      <path d="M6.5 10v2a2 2 0 0 0 2 2M17.5 10v2a2 2 0 0 1-2 2" />
    </svg>
  );

  return (
    <div className="arch-view" style={{ display: isActive ? undefined : "none" }}>
      {root && (
        <header className="arch-toolbar">
          <button type="button" className="arch-btn" onClick={pick} disabled={analyzing}>{t.arch_pick_folder}</button>
          <span className="arch-root" title={root}>{root}</span>
          <button type="button" className="arch-btn" onClick={() => load(root)} disabled={analyzing}>{t.arch_rescan}</button>
          {scan && providers.length > 0 && (
            <ModelPickerButton providers={providers} selectedId={providerId} onChange={changeProvider} disabled={analyzing} />
          )}
          {scan && !analyzing && (
            <button type="button" className="arch-btn arch-btn-primary" onClick={analyze} disabled={status.kind === "busy"}>{t.arch_analyze}</button>
          )}
          {analyzing && (
            <span className="arch-analyzing" role="status">
              <i className="arch-spinner" />
              <span>{`${t.arch_analyzing} ${mmss}`}</span>
              <button type="button" className="arch-btn arch-btn-small" onClick={cancelAnalyze}>{t.arch_cancel}</button>
            </span>
          )}
          <span className="arch-legend"><i className="arch-line-solid" />{t.arch_legend_scan}<i className="arch-line-dash" />{t.arch_legend_ai}</span>
        </header>
      )}
      {drillPath.length > 0 && (
        <nav className="arch-crumbs" aria-label={t.arch_crumb_nav}>
          <button type="button" className="arch-crumb" onClick={() => goTo(0)}>{t.arch_crumb_root}</button>
          {drillPath.map((d, i) => (
            <span key={`${i}-${d.id}`} className="arch-crumb-item">
              <span className="arch-crumb-sep" aria-hidden="true">›</span>
              {i < drillPath.length - 1
                ? <button type="button" className="arch-crumb" onClick={() => goTo(i + 1)}>{d.name}</button>
                : <span className="arch-crumb-cur" aria-current="page">{d.name}</span>}
            </span>
          ))}
          {scan && !analyzing && (
            <button type="button" className="arch-btn arch-btn-small" onClick={analyzeDrill} disabled={status.kind === "busy"}>
              {hasAiDrill ? t.arch_drill_reanalyze : t.arch_drill_analyze}
            </button>
          )}
        </nav>
      )}
      {hiddenCount > 0 && <div className="arch-status arch-status-notice">{t.arch_hidden_n.replace("{n}", String(hiddenCount))}</div>}
      {status.kind !== "idle" && <div className={`arch-status arch-status-${status.kind}`}>{status.msg}</div>}
      {stale && <div className="arch-status arch-status-notice">{t.arch_stale}</div>}
      <div className="arch-body">
        <div className="arch-canvas-wrap">
        <div className={`arch-canvas ${panning ? "arch-panning" : ""}`} ref={canvasRef} onMouseDown={startPan}>
          {!root && (
            <div className="arch-empty">
              <div className="arch-empty-card">
                <div className="arch-empty-icon">{pickIcon}</div>
                <h2>{t.arch_tab}</h2>
                <p>{t.new_arch_desc}</p>
                <button type="button" className="arch-btn arch-btn-primary arch-btn-big" onClick={pick}>{t.arch_pick_folder}</button>
              </div>
            </div>
          )}
          {view && layout && (
            <div className="arch-stage-wrap" style={{ width: layout.width * cur.scale + Math.max(0, cur.tx), height: layout.height * cur.scale + Math.max(0, cur.ty) }}>
              <div className="arch-stage" style={{ width: layout.width, height: layout.height, transform: `translate(${cur.tx}px, ${cur.ty}px) scale(${cur.scale})` }}>
                {layout.columns.map((c, i) => (
                  <div key={c.layerId} className="arch-lane" style={{ left: c.x, top: c.y, width: c.w, height: c.h, ["--lc" as string]: layerColor(c.layerId, i) }}>
                    <div className="arch-lane-head">
                      <i className="arch-dot" />
                      <span className="arch-lane-name" title={c.name}>{c.name}</span>
                      <span className="arch-lane-count">{c.count}</span>
                    </div>
                  </div>
                ))}
                <svg width={layout.width} height={layout.height} className="arch-edges">
                  <defs>
                    <marker id="arch-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto-start-reverse"><path d="M 0 1 L 9 5 L 0 9 z" className="arch-arrow" /></marker>
                    <marker id="arch-arrow-on" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto-start-reverse"><path d="M 0 1 L 9 5 L 0 9 z" className="arch-arrow arch-arrow-on" /></marker>
                    <filter id="arch-glow" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="4" /></filter>
                  </defs>
                  {view.edges.map((e, i) => {
                    const a = layout.nodes[e.from];
                    const b = layout.nodes[e.to];
                    if (!a || !b) return null;
                    const on = hl ? hl.edges.has(e) : false;
                    const d = edgePath(a, b);
                    const dur = Math.min(4, Math.max(1, edgeLength(a, b) / 160));
                    return (
                      <g key={i} onMouseEnter={() => setHoverEdge(i)} onMouseLeave={() => setHoverEdge(null)}>
                        {on && <path d={d} fill="none" className="arch-edge-glow" filter="url(#arch-glow)" />}
                        <path d={d} fill="none" markerEnd={`url(#${on ? "arch-arrow-on" : "arch-arrow"})`} data-on={on ? "true" : "false"}
                          className={`arch-edge ${e.source === "ai" ? "arch-edge-ai" : ""} ${on ? "arch-edge-on" : ""} ${hl && !on ? "arch-edge-dim" : ""}`}>
                          <title>{e.note ?? e.kind}</title>
                        </path>
                        <path d={d} fill="none" className="arch-edge-hit" />
                        {on && !reduced && Array.from({ length: DOTS_PER_EDGE }, (_, k) => (
                          <circle key={k} r="3.5" className="arch-flow-dot">
                            <animateMotion path={d} dur={`${dur.toFixed(2)}s`} begin={k === 0 ? "0s" : `-${((dur * k) / DOTS_PER_EDGE).toFixed(2)}s`} repeatCount="indefinite" />
                          </circle>
                        ))}
                      </g>
                    );
                  })}
                </svg>
                {view.modules.map((m) => {
                  const p = layout.nodes[m.id];
                  if (!p) return null;
                  const dim = hl && !hl.modules.has(m.id);
                  return (
                    <button key={m.id} type="button" title={m.name} aria-label={m.name}
                      className={`arch-node ${selected === m.id ? "arch-node-sel" : ""} ${dim ? "arch-node-dim" : ""}`}
                      style={{ left: p.x, top: p.y, width: NODE_W, height: NODE_H, ["--lc" as string]: layerColor(m.layer, layerIndex(m.layer)) }}
                      onClick={() => setSelected(m.id)} onDoubleClick={() => enter(m.id)}>
                      <span className="arch-node-head">
                        <span className="arch-node-name">{m.name}</span>
                        <span className="arch-node-files">{t.arch_files_count.replace("{n}", String(m.files.length))}</span>
                      </span>
                      {m.summary && <span className="arch-node-sum">{m.summary}</span>}
                      <span className="arch-node-foot">
                        {m.tech.slice(0, 2).map((x) => <span key={x} className="arch-chip arch-chip-sm">{x}</span>)}
                      </span>
                    </button>
                  );
                })}
                {labelPts.map((l) => (
                  <span key={`pill-${l.id}`} className="arch-edge-pill" style={{ left: l.x, top: l.y }}>{view.edges[Number(l.id)].kind}</span>
                ))}
              </div>
            </div>
          )}
        </div>
        {view && layout && <button type="button" className="arch-btn arch-fit" onClick={fitView}>{t.arch_fit}</button>}
        </div>
        {view && <DetailPanel arch={view} selected={selected} onSelect={setSelected} scan={scan} onExpand={enter} activePath={activePath} onPickPath={pickPath} />}
      </div>
    </div>
  );
}
