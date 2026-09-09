import { useState, type CSSProperties, type ReactNode } from "react";
import { hashLabelHue } from "./labelColor";

export function TaskLabelGroup({
  label,
  count,
  highlighted = false,
  children,
}: {
  label: string;
  count: number;
  /** True while a card is being dragged over this group within the same
   * status column (see ProjectBoard's `labelUnderPoint`/`dragOverGroupLabel`
   * — there is no native dragover to key off, same reasoning as the
   * column-level drop highlight). */
  highlighted?: boolean;
  children: ReactNode;
}) {
  const [collapsed, setCollapsed] = useState(false);
  return (
    <div
      className={`task-label-group${highlighted ? " task-label-group--drop-target" : ""}`}
      data-task-label-group={label}
      style={{ "--label-hue": hashLabelHue(label) } as CSSProperties}
    >
      <button
        type="button"
        className="task-label-group-header"
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
