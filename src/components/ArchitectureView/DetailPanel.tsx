import { useLocale } from "../../contexts/LocaleContext";
import type { Architecture } from "./model";

interface Props {
  arch: Architecture;
  selected: string | null;
  onSelect: (id: string) => void;
}

export function DetailPanel({ arch, selected, onSelect }: Props) {
  const { t } = useLocale();
  const mod = arch.modules.find((m) => m.id === selected);
  if (!mod) return <aside className="arch-detail arch-detail-empty">{t.arch_detail_empty}</aside>;

  const nameOf = (id: string) => arch.modules.find((m) => m.id === id)?.name ?? id;
  const up = arch.edges.filter((e) => e.to === mod.id);
  const down = arch.edges.filter((e) => e.from === mod.id);
  const layerName = arch.layers.find((l) => l.id === mod.layer)?.name ?? mod.layer;

  return (
    <aside className="arch-detail">
      <h3>{mod.name}</h3>
      <div className="arch-detail-layer">{layerName}</div>
      {mod.summary && <section><h4>{t.arch_detail_summary}</h4><p>{mod.summary}</p></section>}
      {mod.tech.length > 0 && (
        <section><h4>{t.arch_detail_tech}</h4>
          <div className="arch-chips">{mod.tech.map((x) => <span key={x} className="arch-chip">{x}</span>)}</div>
        </section>
      )}
      {mod.dataflow && <section><h4>{t.arch_detail_dataflow}</h4><p>{mod.dataflow}</p></section>}
      {([[t.arch_detail_upstream, up, "from"], [t.arch_detail_downstream, down, "to"]] as const).map(([title, list, side]) =>
        list.length > 0 && (
          <section key={title}><h4>{title}</h4>
            <ul>
              {list.map((e, i) => (
                <li key={`${e.from}-${e.to}-${i}`}>
                  <button type="button" onClick={() => onSelect(e[side])}>{nameOf(e[side])}</button>
                  <span className={`arch-kind arch-kind-${e.source}`}>{e.kind}</span>
                  {e.source === "ai" && <span className="arch-conf">{t.arch_confidence} {Math.round(e.confidence * 100)}%</span>}
                  {e.note && <div className="arch-note">{e.note}</div>}
                  {e.evidence && <code className="arch-evidence">{t.arch_edge_evidence}：{e.evidence.file}:{e.evidence.line}</code>}
                </li>
              ))}
            </ul>
          </section>
        ),
      )}
      <section><h4>{t.arch_detail_files} ({mod.files.length})</h4>
        <ul className="arch-files">{mod.files.slice(0, 50).map((f) => <li key={f}><code>{f}</code></li>)}</ul>
      </section>
    </aside>
  );
}
