import type { ReactNode } from "react";

export function TaskColumn({
  status,
  title,
  count,
  highlighted = false,
  children,
}: {
  status: string;
  title: string;
  count: number;
  /** True while a card is being dragged over this column (see index.tsx's
   * mouse-event-based drag — there is no native dragover to key off). */
  highlighted?: boolean;
  children: ReactNode;
}) {
  return (
    <div
      className={`task-column${highlighted ? " task-column--drop-target" : ""}`}
      data-testid={`column-${status}`}
    >
      <div className="task-column-head">
        <span className="task-column-title">{title}</span>
        <span className="task-column-count">{count}</span>
      </div>
      <div className="task-column-body">{children}</div>
    </div>
  );
}
