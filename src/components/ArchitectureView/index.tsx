import { useCallback, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { nanoid } from "nanoid";
import { useLocale } from "../../contexts/LocaleContext";
import { archLoad, archReadFiles, archSave, archScan, type ScanResult } from "../../ipc/architecture";
import { formatAiError, invokeAiComplete, type AiError } from "../../ipc/ai";
import { ANALYZE_SYSTEM, buildAnalyzeMessage, mergeScanEdges, parseAnalysis, pickKeyFiles } from "./analyze";
import { buildBaseGraph, type Architecture } from "./model";
import { edgePath, layoutGraph, NODE_H, NODE_W, traceChain } from "./layout";
import { DetailPanel } from "./DetailPanel";
import "./index.css";

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
      setStatus(s.truncated ? { kind: "notice", msg: t.arch_truncated } : { kind: "idle" });
    } catch (e) {
      setStatus({ kind: "error", msg: t.arch_error.replace("{msg}", errText(e)) });
    }
  }, [t]);

  const pick = useCallback(async () => {
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir === "string") await load(dir);
  }, [load]);

  const analyze = useCallback(async () => {
    if (!root || !scan) return;
    setStatus({ kind: "busy", msg: t.arch_analyzing });
    try {
      const base = buildBaseGraph(scan);
      const files = await archReadFiles(root, pickKeyFiles(scan));
      const reply = await invokeAiComplete(
        [{ role: "user", content: buildAnalyzeMessage(scan, base, files) }],
        ANALYZE_SYSTEM,
        nanoid(),
      );
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
      const err = e as AiError;
      setStatus(err?.kind === "not_configured"
        ? { kind: "notice", msg: t.arch_no_ai }
        : { kind: "error", msg: t.arch_error.replace("{msg}", err?.kind ? formatAiError(err) : errText(e)) });
    }
  }, [root, scan, t]);

  const layout = useMemo(() => (arch ? layoutGraph(arch) : null), [arch]);
  const chain = useMemo(() => (arch && selected ? traceChain(arch, selected) : null), [arch, selected]);

  return (
    <div className="arch-view" style={{ display: isActive ? undefined : "none" }}>
      <header className="arch-toolbar">
        <button type="button" onClick={pick}>{t.arch_pick_folder}</button>
        {root && <span className="arch-root">{root}</span>}
        {root && <button type="button" onClick={() => load(root)}>{t.arch_rescan}</button>}
        {scan && <button type="button" onClick={analyze} disabled={status.kind === "busy"}>{t.arch_analyze}</button>}
        <span className="arch-legend"><i className="arch-line-solid" />{t.arch_legend_scan}<i className="arch-line-dash" />{t.arch_legend_ai}</span>
      </header>
      {status.kind !== "idle" && <div className={`arch-status arch-status-${status.kind}`}>{status.msg}</div>}
      {stale && <div className="arch-status arch-status-notice">{t.arch_stale}</div>}
      <div className="arch-body">
        <div className="arch-canvas">
          {arch && layout && (
            <div className="arch-stage" style={{ width: layout.width, height: layout.height }}>
              <svg width={layout.width} height={layout.height} className="arch-edges">
                {arch.edges.map((e, i) => {
                  const a = layout.nodes[e.from];
                  const b = layout.nodes[e.to];
                  if (!a || !b) return null;
                  const on = chain ? chain.edges.includes(e) : false;
                  return (
                    <path key={i} d={edgePath(a, b)} fill="none"
                      className={`arch-edge ${e.source === "ai" ? "arch-edge-ai" : ""} ${on ? "arch-edge-on" : ""} ${chain && !on ? "arch-edge-dim" : ""}`}>
                      <title>{e.note ?? e.kind}</title>
                    </path>
                  );
                })}
              </svg>
              {layout.columns.map((c) => (
                <div key={c.layerId} className="arch-col-title" style={{ left: c.x, width: NODE_W }}>{c.name}</div>
              ))}
              {arch.modules.map((m) => {
                const p = layout.nodes[m.id];
                if (!p) return null;
                const dim = chain && !chain.modules.has(m.id);
                return (
                  <button key={m.id} type="button" title={m.id}
                    className={`arch-node ${selected === m.id ? "arch-node-sel" : ""} ${dim ? "arch-node-dim" : ""}`}
                    style={{ left: p.x, top: p.y, width: NODE_W, height: NODE_H }}
                    onClick={() => setSelected(m.id)}>
                    {m.name}
                  </button>
                );
              })}
            </div>
          )}
        </div>
        {arch && <DetailPanel arch={arch} selected={selected} onSelect={setSelected} />}
      </div>
    </div>
  );
}
