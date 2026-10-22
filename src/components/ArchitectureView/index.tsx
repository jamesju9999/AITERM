import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { nanoid } from "nanoid";
import { useLocale } from "../../contexts/LocaleContext";
import { archLoad, archReadFiles, archSave, archScan, type ScanResult } from "../../ipc/architecture";
import { abortAi, formatAiError, invokeAiComplete, type AiError } from "../../ipc/ai";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { ModelPickerButton } from "../ModelPickerButton";
import { ANALYZE_SYSTEM, buildAnalyzeMessage, mergeScanEdges, parseAnalysis, pickKeyFiles } from "./analyze";
import { buildBaseGraph, type Architecture } from "./model";
import { edgeMidpoint, edgePath, fitScale, layerColor, layoutGraph, NODE_H, NODE_W, traceChain } from "./layout";
import { DetailPanel } from "./DetailPanel";
import "./index.css";

const PROVIDER_KEY = "aiterm_arch_provider";

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
  const [stale, setStale] = useState(false);
  const [status, setStatus] = useState<Status>({ kind: "idle" });
  const [analyzing, setAnalyzing] = useState(false);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [providerId, setProviderId] = useState("");
  const [elapsed, setElapsed] = useState(0);
  const [hoverEdge, setHoverEdge] = useState<number | null>(null);
  const [viewportW, setViewportW] = useState(0);
  const streamIdRef = useRef<string | null>(null);
  const runRef = useRef(0);
  const canvasRef = useRef<HTMLDivElement>(null);

  const load = useCallback(async (dir: string) => {
    setRoot(dir);
    setSelected(null);
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

  const analyze = useCallback(async () => {
    if (!root || !scan) return;
    const run = ++runRef.current;
    const streamId = nanoid();
    streamIdRef.current = streamId;
    setElapsed(0);
    setAnalyzing(true);
    setStatus({ kind: "idle" });
    try {
      const base = buildBaseGraph(scan);
      const files = await archReadFiles(root, pickKeyFiles(scan));
      const reply = await invokeAiComplete(
        [{ role: "user", content: buildAnalyzeMessage(scan, base, files) }],
        ANALYZE_SYSTEM,
        streamId,
        providerId || undefined,
      );
      if (run !== runRef.current) return; // 已取消
      const parsed = reply.content ? parseAnalysis(reply.content, scan) : null;
      if (!parsed) {
        setStatus({ kind: "error", msg: t.arch_error.replace("{msg}", "AI 回應不是有效的架構 JSON") });
        return;
      }
      const merged = mergeScanEdges(parsed, scan);
      setArch(merged);
      setStale(false);
      await archSave(root, JSON.stringify(merged));
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
    const ro = new ResizeObserver((entries) => setViewportW(entries[0]?.contentRect.width ?? 0));
    ro.observe(el);
    return () => ro.disconnect();
  }, [arch]);

  const layout = useMemo(() => (arch ? layoutGraph(arch) : null), [arch]);
  const chain = useMemo(() => (arch && selected ? traceChain(arch, selected) : null), [arch, selected]);

  const scale = layout ? fitScale(layout.width, viewportW) : 1;
  const layerIndex = (id: string) => Math.max(0, arch?.layers.findIndex((l) => l.id === id) ?? 0);
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
      {status.kind !== "idle" && <div className={`arch-status arch-status-${status.kind}`}>{status.msg}</div>}
      {stale && <div className="arch-status arch-status-notice">{t.arch_stale}</div>}
      <div className="arch-body">
        <div className="arch-canvas" ref={canvasRef}>
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
          {arch && layout && (
            <div className="arch-stage-wrap" style={{ width: layout.width * scale, height: layout.height * scale }}>
              <div className="arch-stage" style={{ width: layout.width, height: layout.height, transform: scale < 1 ? `scale(${scale})` : undefined }}>
                {layout.columns.map((c, i) => (
                  <div key={c.layerId} className="arch-lane" style={{ left: c.x, top: c.y, width: c.w, height: c.h, ["--lc" as string]: layerColor(c.layerId, i) }}>
                    <div className="arch-lane-head">
                      <i className="arch-dot" />
                      <span className="arch-lane-name">{c.name}</span>
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
                  {arch.edges.map((e, i) => {
                    const a = layout.nodes[e.from];
                    const b = layout.nodes[e.to];
                    if (!a || !b) return null;
                    const on = chain ? chain.edges.includes(e) : false;
                    const d = edgePath(a, b);
                    return (
                      <g key={i} onMouseEnter={() => setHoverEdge(i)} onMouseLeave={() => setHoverEdge(null)}>
                        {on && <path d={d} fill="none" className="arch-edge-glow" filter="url(#arch-glow)" />}
                        <path d={d} fill="none" markerEnd={`url(#${on ? "arch-arrow-on" : "arch-arrow"})`}
                          className={`arch-edge ${e.source === "ai" ? "arch-edge-ai" : ""} ${on ? "arch-edge-on" : ""} ${chain && !on ? "arch-edge-dim" : ""}`}>
                          <title>{e.note ?? e.kind}</title>
                        </path>
                        <path d={d} fill="none" className="arch-edge-hit" />
                      </g>
                    );
                  })}
                </svg>
                {arch.modules.map((m) => {
                  const p = layout.nodes[m.id];
                  if (!p) return null;
                  const dim = chain && !chain.modules.has(m.id);
                  return (
                    <button key={m.id} type="button" title={m.id} aria-label={m.name}
                      className={`arch-node ${selected === m.id ? "arch-node-sel" : ""} ${dim ? "arch-node-dim" : ""}`}
                      style={{ left: p.x, top: p.y, width: NODE_W, height: NODE_H, ["--lc" as string]: layerColor(m.layer, layerIndex(m.layer)) }}
                      onClick={() => setSelected(m.id)}>
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
                {arch.edges.map((e, i) => {
                  const a = layout.nodes[e.from];
                  const b = layout.nodes[e.to];
                  if (!a || !b) return null;
                  if (!(chain?.edges.includes(e) || hoverEdge === i)) return null;
                  const m = edgeMidpoint(a, b);
                  return <span key={`pill-${i}`} className="arch-edge-pill" style={{ left: m.x, top: m.y }}>{e.kind}</span>;
                })}
              </div>
            </div>
          )}
        </div>
        {arch && <DetailPanel arch={arch} selected={selected} onSelect={setSelected} />}
      </div>
    </div>
  );
}
