import type { DragEvent, ReactNode } from "react";

export function TaskColumn({
  status,
  title,
  count,
  onDropCard,
  children,
}: {
  status: string;
  title: string;
  count: number;
  onDropCard: () => void;
  children: ReactNode;
}) {
  return (
    <div
      className="task-column"
      data-testid={`column-${status}`}
      onDragOver={(e: DragEvent) => e.preventDefault()}
      onDrop={(e: DragEvent) => {
        e.preventDefault();
        onDropCard();
      }}
    >
      <div className="task-column-head">
        <span className="task-column-title">{title}</span>
        <span className="task-column-count">{count}</span>
      </div>
      <div className="task-column-body">{children}</div>
    </div>
  );
}
