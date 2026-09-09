import { useState, type CSSProperties, type ReactNode } from "react";
import { hashLabelHue } from "./labelColor";

export function TaskLabelGroup({
  label,
  count,
  children,
}: {
  label: string;
  count: number;
  children: ReactNode;
}) {
  const [collapsed, setCollapsed] = useState(false);
  return (
    <div className="task-label-group">
      <button
        type="button"
        className="task-label-group-header"
        style={{ "--label-hue": hashLabelHue(label) } as CSSProperties}
        onClick={() => setCollapsed((c) => !c)}
      >
        <span className="task-label-group-caret">{collapsed ? "▸" : "▾"}</span>
        <span className="task-label-chip">{label}</span>
        <span className="task-label-group-count">({count})</span>
      </button>
      {!collapsed && <div className="task-label-group-body">{children}</div>}
    </div>
  );
}
