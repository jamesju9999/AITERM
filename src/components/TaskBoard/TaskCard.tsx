import type { TaskWithAttachments } from "../../ipc/tasks";

export function TaskCard({ card }: {
  card: TaskWithAttachments;
  onEdit: () => void;
  onViewTranscript: () => void;
  onChanged: () => void;
}) {
  return <div className="task-card" data-testid={`card-${card.id}`}>{card.title}</div>;
}
