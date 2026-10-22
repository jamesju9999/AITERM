import { useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import { layerColor } from "./layout";
import type { Architecture } from "./model";

interface Props {
  arch: Architecture;
  selected: string | null;
  onSelect: (id: string) => void;
}

const FILES_COLLAPSED = 8;

export function DetailPanel({ arch, selected, onSelect }: Props) {
  const { t } = useLocale();
  // 記下「哪個模組展開了」，切換模組時自然回到收合，不必靠 effect 重設。
  const [expandedFor, setExpandedFor] = useState<string | null>(null);
  const showAll = expandedFor !== null && expandedFor === selected;

  const mod = arch.modules.find((m) => m.id === selected);
  if (!mod) {
    return (
      <aside className="arch-detail arch-detail-empty">
        <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
          <rect x="3" y="4" width="7" height="6" rx="1.5" /><rect x="14" y="14" width="7" height="6" rx="1.5" /><path d="M10 7h4a3 3 0 0 1 3 3v4" />
        </svg>
        <span>{t.arch_detail_empty}</span>
      </aside>
    );
  }

  const nameOf = (id: string) => arch.modules.find((m) => m.id === id)?.name ?? id;
  const up = arch.edges.filter((e) => e.to === mod.id);
  const down = arch.edges.filter((e) => e.from === mod.id);
  const layerIdx = arch.layers.findIndex((l) => l.id === mod.layer);
  const layerName = arch.layers[layerIdx]?.name ?? mod.layer;
  const color = layerColor(mod.layer, Math.max(0, layerIdx));
  const files = showAll ? mod.files : mod.files.slice(0, FILES_COLLAPSED);

  return (
    <aside className="arch-detail" style={{ ["--lc" as string]: color }}>
      <div className="arch-detail-head">
        <h3>{mod.name}</h3>
        <div className="arch-detail-layer"><i className="arch-dot" />{layerName}</div>
      </div>
      {mod.summary && <section className="arch-card"><h4>{t.arch_detail_summary}</h4><p>{mod.summary}</p></section>}
      {mod.tech.length > 0 && (
        <section className="arch-card"><h4>{t.arch_detail_tech}</h4>
          <div className="arch-chips">{mod.tech.map((x) => <span key={x} className="arch-chip">{x}</span>)}</div>
        </section>
      )}
      {mod.dataflow && <section className="arch-card"><h4>{t.arch_detail_dataflow}</h4><p>{mod.dataflow}</p></section>}
      {([[t.arch_detail_upstream, up, "from"], [t.arch_detail_downstream, down, "to"]] as const).map(([title, list, side]) =>
        list.length > 0 && (
          <section key={title} className="arch-card"><h4>{title} ({list.length})</h4>
            <ul className="arch-edge-list">
              {list.map((e, i) => (
                <li key={`${e.from}-${e.to}-${i}`} className={`arch-edge-card arch-edge-card-${e.source}`}>
                  <div className="arch-edge-card-head">
                    <button type="button" className="arch-link" onClick={() => onSelect(e[side])}>{nameOf(e[side])}</button>
                    <span className={`arch-kind arch-kind-${e.source}`}>{e.kind}</span>
                    {e.source === "ai" && (
                      <span className="arch-conf" title={t.arch_confidence}>
                        <span className="arch-conf-bar"><span style={{ width: `${Math.round(e.confidence * 100)}%` }} /></span>
                        {Math.round(e.confidence * 100)}%
                      </span>
                    )}
                  </div>
                  {e.note && <div className="arch-note">{e.note}</div>}
                  {e.evidence && <code className="arch-evidence" title={t.arch_edge_evidence}>{e.evidence.file}:{e.evidence.line}</code>}
                </li>
              ))}
            </ul>
          </section>
        ),
      )}
      <section className="arch-card"><h4>{t.arch_detail_files} ({mod.files.length})</h4>
        <ul className="arch-files">{files.map((f) => <li key={f}><code>{f}</code></li>)}</ul>
        {mod.files.length > FILES_COLLAPSED && (
          <button type="button" className="arch-link arch-files-toggle" onClick={() => setExpandedFor(showAll ? null : mod.id)}>
            {showAll ? t.arch_show_less : t.arch_show_all.replace("{n}", String(mod.files.length))}
          </button>
        )}
      </section>
    </aside>
  );
}
