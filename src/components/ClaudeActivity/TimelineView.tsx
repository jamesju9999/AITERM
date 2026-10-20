import { useMemo } from "react";
import type { ActivityNode } from "../../ipc/claudeActivity";
import { useLocale } from "../../contexts/LocaleContext";
import { formatDuration } from "./format";
import { buildTimeline } from "./timeline";

interface Props {
  nodes: ActivityNode[];
  now: number;
  selectedId: string | null;
  onSelect: (id: string | null) => void;
}

/** 時間軸：每個 subagent 一條橫條，看平行度與誰最慢。 */
export function TimelineView({ nodes, now, selectedId, onSelect }: Props) {
  const { t } = useLocale();
  const { rows, spanMs } = useMemo(() => buildTimeline(nodes, now), [nodes, now]);
  return (
    <div className="aiterm-cact__scroll aiterm-cact__timeline">
      <div className="aiterm-cact__timeline-span">{t.claude_activity_timeline_span(formatDuration(spanMs))}</div>
      {rows.map(({ node, depth, left, width }) => {
        const title = node.description ?? t.claude_activity_untitled;
        return (
          <button
            key={node.id}
            type="button"
            className={`aiterm-cact__trow${selectedId === node.id ? " is-selected" : ""}`}
            data-node-id={node.id}
            aria-pressed={selectedId === node.id}
            title={title}
            onClick={() => onSelect(selectedId === node.id ? null : node.id)}
          >
            <span className="aiterm-cact__trow-label" style={{ paddingLeft: depth * 10 }}>{title}</span>
            <span className="aiterm-cact__track">
              <span
                className={`aiterm-cact__bar aiterm-cact__bar--${node.status}`}
                data-testid={`bar-${node.id}`}
                style={{ left: `${left}%`, width: `${width}%` }}
              />
            </span>
          </button>
        );
      })}
    </div>
  );
}
