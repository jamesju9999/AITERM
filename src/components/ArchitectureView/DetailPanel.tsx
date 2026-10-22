import { useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import type { ScanResult } from "../../ipc/architecture";
import { externalRefs } from "./drill";
import { groupEdges, type GroupedEdge } from "./flow";
import { layerColor } from "./layout";
import type { Architecture } from "./model";
import { buildOverview } from "./overview";

interface Props {
  arch: Architecture;
  selected: string | null;
  onSelect: (id: string) => void;
  /** 目前圖所屬專案的掃描結果；有它才能算外部引用與總覽。 */
  scan?: ScanResult | null;
  /** 往下鑽。單一檔案的節點不會顯示展開鈕。 */
  onExpand?: (id: string) => void;
}

const FILES_COLLAPSED = 8;

export function DetailPanel({ arch, selected, onSelect, scan, onExpand }: Props) {
  const { t } = useLocale();
  // 記下「哪個模組展開了」，切換模組時自然回到收合，不必靠 effect 重設。
  const [expandedFor, setExpandedFor] = useState<string | null>(null);
  const showAll = expandedFor !== null && expandedFor === selected;

  const mod = arch.modules.find((m) => m.id === selected);
  if (!mod) {
    const ov = scan ? buildOverview(scan, arch) : null;
    const maxLang = Math.max(1, ...(ov?.languages.map((l) => l.count) ?? []));
    return (
      <aside className={`arch-detail ${ov ? "" : "arch-detail-empty"}`}>
        <div className="arch-detail-hint">
          <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
            <rect x="3" y="4" width="7" height="6" rx="1.5" /><rect x="14" y="14" width="7" height="6" rx="1.5" /><path d="M10 7h4a3 3 0 0 1 3 3v4" />
          </svg>
          <span>{t.arch_detail_empty}</span>
        </div>
        {ov && (
          <>
            <h3 className="arch-ov-title">{t.arch_ov_title}</h3>
            <div className="arch-ov-counts">{t.arch_ov_counts.replace("{m}", String(ov.moduleCount)).replace("{e}", String(ov.edgeCount))}</div>
            {ov.languages.length > 0 && (
              <section className="arch-card"><h4>{t.arch_ov_langs}</h4>
                <ul className="arch-bars">
                  {ov.languages.map((l) => (
                    <li key={l.lang} className="arch-bar-row">
                      <span className="arch-bar-name">{l.lang}</span>
                      <span className="arch-bar"><span style={{ width: `${Math.round((l.count / maxLang) * 100)}%` }} /></span>
                      <span className="arch-bar-n">{l.count}</span>
                    </li>
                  ))}
                </ul>
              </section>
            )}
            {ov.tech.length > 0 && (
              <section className="arch-card"><h4>{t.arch_ov_tech}</h4>
                <div className="arch-chips">{ov.tech.map((x) => <span key={x.name} className="arch-chip">{x.name}</span>)}</div>
              </section>
            )}
            {ov.deps.length > 0 && (
              <section className="arch-card"><h4>{t.arch_ov_deps}</h4>
                <div className="arch-chips">{ov.deps.map((x) => <span key={x} className="arch-chip">{x}</span>)}</div>
              </section>
            )}
          </>
        )}
      </aside>
    );
  }

  const nameOf = (id: string) => arch.modules.find((m) => m.id === id)?.name ?? id;
  const groups = groupEdges(arch.edges, mod.id);
  const up = groups.filter((g) => !g.mutual && g.edges[0].to === mod.id);
  const down = groups.filter((g) => !g.mutual && g.edges[0].from === mod.id);
  const mutual = groups.filter((g) => g.mutual);
  const ext = scan ? externalRefs(scan, mod.files) : null;
  const layerIdx = arch.layers.findIndex((l) => l.id === mod.layer);
  const layerName = arch.layers[layerIdx]?.name ?? mod.layer;
  const color = layerColor(mod.layer, Math.max(0, layerIdx));
  const files = showAll ? mod.files : mod.files.slice(0, FILES_COLLAPSED);

  return (
    <aside className="arch-detail" style={{ ["--lc" as string]: color }}>
      <div className="arch-detail-head">
        <h3>{mod.name}</h3>
        <div className="arch-detail-layer"><i className="arch-dot" />{layerName}</div>
        {onExpand && mod.files.length > 1 && (
          <button type="button" className="arch-btn arch-btn-small arch-expand" onClick={() => onExpand(mod.id)}>{t.arch_expand}</button>
        )}
      </div>
      {ext && (ext.inbound > 0 || ext.outbound > 0) && (
        <div className="arch-ext">
          <span>{t.arch_ext_in.replace("{n}", String(ext.inbound))}</span>
          <span>{t.arch_ext_out.replace("{n}", String(ext.outbound))}</span>
        </div>
      )}
      {mod.summary && <section className="arch-card"><h4>{t.arch_detail_summary}</h4><p>{mod.summary}</p></section>}
      {mod.tech.length > 0 && (
        <section className="arch-card"><h4>{t.arch_detail_tech}</h4>
          <div className="arch-chips">{mod.tech.map((x) => <span key={x} className="arch-chip">{x}</span>)}</div>
        </section>
      )}
      {mod.dataflow && <section className="arch-card"><h4>{t.arch_detail_dataflow}</h4><p>{mod.dataflow}</p></section>}
      {([[t.arch_mutual, mutual], [t.arch_detail_upstream, up], [t.arch_detail_downstream, down]] as [string, GroupedEdge[]][]).map(([title, list]) =>
        list.length > 0 && (
          <section key={title} className="arch-card"><h4>{title} ({list.length})</h4>
            <ul className="arch-edge-list">
              {list.map((g) => (
                <li key={`${g.other}-${g.kind}`} className={`arch-edge-card arch-edge-card-${g.edges[0].source}`}>
                  <div className="arch-edge-card-head">
                    {g.mutual && <span className="arch-mutual" aria-hidden="true">↔</span>}
                    <button type="button" className="arch-link" onClick={() => onSelect(g.other)}>{nameOf(g.other)}</button>
                    <span className={`arch-kind arch-kind-${g.edges[0].source}`}>{g.kind}</span>
                  </div>
                  {g.edges.map((e, i) => (
                    <div key={i} className="arch-edge-line">
                      {g.mutual && <span className="arch-dir">{e.from === mod.id ? "→" : "←"}</span>}
                      {e.source === "ai" && (
                        <span className="arch-conf" title={t.arch_confidence}>
                          <span className="arch-conf-bar"><span style={{ width: `${Math.round(e.confidence * 100)}%` }} /></span>
                          {Math.round(e.confidence * 100)}%
                        </span>
                      )}
                      {e.note && <div className="arch-note">{e.note}</div>}
                      {e.evidence && <code className="arch-evidence" title={t.arch_edge_evidence}>{e.evidence.file}:{e.evidence.line}</code>}
                    </div>
                  ))}
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
