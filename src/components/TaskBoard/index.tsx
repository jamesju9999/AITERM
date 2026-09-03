import { useCallback, useEffect, useRef, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import {
  listTasks,
  moveTask,
  onTasksUpdated,
  type TaskStatus,
  type TaskWithAttachments,
} from "../../ipc/tasks";
import { TaskCard } from "./TaskCard";
import { TaskColumn } from "./TaskColumn";
import { TaskEditorDialog } from "./TaskEditorDialog";
import { TranscriptDialog } from "./TranscriptDialog";
import "./index.css";

const COLUMNS: TaskStatus[] = ["planning", "queued", "running", "done"];

export function TaskBoardView() {
  const { t } = useLocale();
  const [tasks, setTasks] = useState<TaskWithAttachments[]>([]);
  const [editing, setEditing] = useState<TaskWithAttachments | "new" | null>(null);
  const [transcriptFor, setTranscriptFor] = useState<string | null>(null);
  const mounted = useRef(true);
  const draggingId = useRef<string | null>(null);

  const refresh = useCallback(async () => {
    const rows = await listTasks();
    if (mounted.current) setTasks(rows);
  }, []);

  useEffect(() => {
    mounted.current = true;
    void refresh();
    const un = onTasksUpdated(() => void refresh());
    return () => {
      mounted.current = false;
      void un.then((f) => f());
    };
  }, [refresh]);

  const colTitle = (s: TaskStatus) =>
    ({
      planning: t.board_col_planning,
      queued: t.board_col_queued,
      running: t.board_col_running,
      done: t.board_col_done,
    })[s];

  const byStatus = (s: TaskStatus) =>
    tasks.filter((x) => x.status === s).sort((a, b) => a.sort_order - b.sort_order);

  const handleDrop = useCallback(
    async (to: TaskStatus) => {
      const id = draggingId.current;
      draggingId.current = null;
      if (!id) return;
      const cardRow = tasks.find((x) => x.id === id);
      if (!cardRow || cardRow.status === to) return;
      const legal =
        (cardRow.status === "planning" && to === "queued") ||
        (cardRow.status === "queued" && to === "planning");
      if (!legal) return;
      const dest = tasks
        .filter((x) => x.status === to)
        .sort((a, b) => a.sort_order - b.sort_order);
      const sortOrder = dest.length ? dest[dest.length - 1].sort_order + 1 : 1;
      await moveTask(id, to, sortOrder);
      setTasks((prev) =>
        prev.map((x) => (x.id === id ? { ...x, status: to, sort_order: sortOrder } : x)),
      );
    },
    [tasks],
  );

  return (
    <div className="task-board">
      <div className="task-board-toolbar">
        <button className="task-board-new" onClick={() => setEditing("new")}>
          + {t.board_new_card}
        </button>
      </div>
      <div className="task-board-columns">
        {COLUMNS.map((s) => (
          <TaskColumn
            key={s}
            status={s}
            title={colTitle(s)}
            count={byStatus(s).length}
            onDropCard={() => void handleDrop(s)}
          >
            {byStatus(s).map((cardRow) => (
              <div
                key={cardRow.id}
                draggable={cardRow.status === "planning" || cardRow.status === "queued"}
                onDragStart={() => {
                  draggingId.current = cardRow.id;
                }}
                onDragEnd={() => {
                  draggingId.current = null;
                }}
              >
                <TaskCard
                  card={cardRow}
                  onEdit={() => setEditing(cardRow)}
                  onViewTranscript={() => setTranscriptFor(cardRow.id)}
                  onChanged={() => void refresh()}
                />
              </div>
            ))}
          </TaskColumn>
        ))}
      </div>

      {editing && (
        <TaskEditorDialog
          card={editing === "new" ? null : editing}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null);
            void refresh();
          }}
        />
      )}
      {transcriptFor && (
        <TranscriptDialog taskId={transcriptFor} onClose={() => setTranscriptFor(null)} />
      )}
    </div>
  );
}
