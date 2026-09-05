import { useCallback, useEffect, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import { createPortal } from "react-dom";

import { useLocale } from "../../contexts/LocaleContext";
import {
  listTasks,
  markTaskDone,
  moveTask,
  onTasksUpdated,
  type TaskStatus,
  type TaskWithAttachments,
} from "../../ipc/tasks";
import { setRunningTaskTabs } from "../../lib/runningTaskTabRegistry";
import { TaskCard } from "./TaskCard";
import { TaskColumn } from "./TaskColumn";
import { TaskEditorDialog } from "./TaskEditorDialog";
import { TranscriptDialog } from "./TranscriptDialog";
import { tryUpgradeTranscript } from "./transcriptUpgrade";

const COLUMNS: TaskStatus[] = ["planning", "queued", "running", "done"];

/** Below this many pixels of movement, a mousedown+mouseup is a click, not a
 * drag — same threshold TabBar's own (proven-working) drag uses. */
const DRAG_THRESHOLD_PX = 4;

interface DragState {
  id: string;
  startX: number;
  startY: number;
  started: boolean;
}

export function ProjectBoard({ projectId }: { projectId: string }) {
  const { t } = useLocale();
  const [tasks, setTasks] = useState<TaskWithAttachments[]>([]);
  const [editing, setEditing] = useState<TaskWithAttachments | "new" | null>(null);
  const [transcriptFor, setTranscriptFor] = useState<string | null>(null);
  const mounted = useRef(true);
  /** Previous fetch's id→status snapshot, so refresh() can tell a genuine
   * "just transitioned into done" apart from "was already done on a
   * previous fetch" (including the very first load — an already-done task
   * from a prior session must NOT be treated as freshly completed). */
  const lastStatusRef = useRef<Map<string, TaskStatus>>(new Map());
  const dragRef = useRef<DragState | null>(null);
  /** Which column's `data-testid` the cursor is currently over while
   * dragging, for the drop-target highlight. Not the drop decision itself
   * (that's read fresh from `elementFromPoint` on mouseup). */
  const [dragOverStatus, setDragOverStatus] = useState<TaskStatus | null>(null);
  /** id of the card currently being dragged, purely for the fade-out visual
   * (mousedown alone isn't "dragging" yet — only once the threshold is
   * crossed). Without this the interaction gave no feedback at all, which
   * made it look broken even once the drag mechanism itself worked. */
  const [draggingCardId, setDraggingCardId] = useState<string | null>(null);
  /** Live cursor position while dragging, in viewport coordinates. Drives a
   * "ghost" card rendered via a portal into document.body (see the render
   * below) — NOT a transform on the card's own wrapper. Every column has
   * `overflow: hidden`, so a translated-in-place card got clipped the
   * instant it crossed its own column's edge into a neighbor; a portal
   * sibling of every column has no such ancestor to be clipped by. */
  const [dragPointer, setDragPointer] = useState<{ x: number; y: number } | null>(null);

  const refresh = useCallback(async () => {
    const rows = await listTasks(projectId);
    if (!mounted.current) return;
    const previous = lastStatusRef.current;
    const next = new Map<string, TaskStatus>();
    for (const row of rows) {
      next.set(row.id, row.status);
      const wasDone = previous.get(row.id) === "done";
      const justFinished = previous.has(row.id) && !wasDone && row.status === "done";
      if (justFinished) {
        void tryUpgradeTranscript(projectId, row.id, row.tab_id);
      }
    }
    lastStatusRef.current = next;
    setTasks(rows);
  }, [projectId]);

  useEffect(() => {
    mounted.current = true;
    void refresh();
    const un = onTasksUpdated(() => void refresh());
    return () => {
      mounted.current = false;
      void un.then((f) => f());
    };
  }, [refresh]);

  // Keeps TerminalView's close guard aware of which tabs currently belong
  // to a `running` task — see runningTaskTabRegistry.ts for why this lives
  // in a shared module instead of TaskBoard registering its own close
  // guard directly (a tab can only have ONE registered guard; this tab
  // already has TerminalView's own busy/mission guard on it).
  useEffect(() => {
    setRunningTaskTabs(
      tasks.filter((x) => x.status === "running" && x.tab_id).map((x) => x.tab_id as string),
    );
  }, [tasks]);

  const colTitle = (s: TaskStatus) =>
    ({
      planning: t.board_col_planning,
      queued: t.board_col_queued,
      running: t.board_col_running,
      done: t.board_col_done,
    })[s];

  const byStatus = (s: TaskStatus) =>
    tasks.filter((x) => x.status === s).sort((a, b) => a.sort_order - b.sort_order);

  // Single source of truth for "can this card legally be dropped on this
  // column", shared by handleDrop (what actually happens on release) and
  // the drag-over highlight below (what visually invites a drop while
  // hovering) — so a column can never light up as a valid target and then
  // silently reject the drop, which was the actual bug this fixed.
  const isLegalDropTarget = useCallback((cardRow: TaskWithAttachments, to: TaskStatus): boolean => {
    if (cardRow.status === to) return false;
    if (cardRow.status === "running" && to === "done" && cardRow.interactive) return true;
    return (
      (cardRow.status === "planning" && to === "queued") ||
      (cardRow.status === "queued" && to === "planning")
    );
  }, []);

  const handleDrop = useCallback(
    async (id: string, to: TaskStatus) => {
      const cardRow = tasks.find((x) => x.id === id);
      if (!cardRow || !isLegalDropTarget(cardRow, to)) return;

      if (cardRow.status === "running" && to === "done" && cardRow.interactive) {
        await markTaskDone(projectId, id);
        return; // finish flow (incl. the outcome badge) arrives via the
                 // existing tasks-updated listener, same as auto-completion.
      }

      const dest = tasks
        .filter((x) => x.status === to)
        .sort((a, b) => a.sort_order - b.sort_order);
      const sortOrder = dest.length ? dest[dest.length - 1].sort_order + 1 : 1;
      await moveTask(projectId, id, to, sortOrder);
      setTasks((prev) =>
        prev.map((x) => (x.id === id ? { ...x, status: to, sort_order: sortOrder } : x)),
      );
    },
    [tasks, isLegalDropTarget, projectId],
  );

  // Card drag-to-move: deliberately NOT native HTML5 drag-and-drop
  // (draggable/dragstart/dragover/drop). Tauri's window-level
  // `dragDropEnabled` (default true, not overridden in tauri.conf.json)
  // intercepts any native OS drag session before those DOM events ever
  // fire — the same reason TabBar's own tab-reorder drag uses plain mouse
  // events instead. Mirrors that exact mechanism: mousedown arms a pending
  // drag, mousemove past a small threshold "starts" it and tracks which
  // column is under the cursor via `elementFromPoint` (there is no native
  // dragover to listen to), mouseup reads the column under the release
  // point and commits the move.
  const statusUnderPoint = useCallback((x: number, y: number): TaskStatus | null => {
    const el = document.elementFromPoint(x, y);
    const testId = el?.closest("[data-testid^='column-']")?.getAttribute("data-testid");
    const status = testId?.replace("column-", "");
    return status && (COLUMNS as string[]).includes(status) ? (status as TaskStatus) : null;
  }, []);

  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      const st = dragRef.current;
      if (!st) return;
      if (!st.started) {
        if (Math.hypot(e.clientX - st.startX, e.clientY - st.startY) < DRAG_THRESHOLD_PX) return;
        st.started = true;
        setDraggingCardId(st.id);
      }
      setDragPointer({ x: e.clientX, y: e.clientY });
      const hovered = statusUnderPoint(e.clientX, e.clientY);
      const draggedCard = tasks.find((x) => x.id === st.id);
      setDragOverStatus(hovered && draggedCard && isLegalDropTarget(draggedCard, hovered) ? hovered : null);
    };
    const onUp = (e: MouseEvent) => {
      const st = dragRef.current;
      dragRef.current = null;
      setDragOverStatus(null);
      setDraggingCardId(null);
      setDragPointer(null);
      if (!st?.started) return;
      const to = statusUnderPoint(e.clientX, e.clientY);
      if (to) void handleDrop(st.id, to);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [handleDrop, statusUnderPoint, tasks, isLegalDropTarget]);

  const handleCardMouseDown = (e: ReactMouseEvent<HTMLDivElement>, cardRow: TaskWithAttachments) => {
    if (e.button !== 0) return;
    const draggable =
      cardRow.status === "planning" ||
      cardRow.status === "queued" ||
      (cardRow.status === "running" && cardRow.interactive);
    if (!draggable) return;
    dragRef.current = { id: cardRow.id, startX: e.clientX, startY: e.clientY, started: false };
  };

  return (
    <div className="task-board-inner">
      <div className="task-board-toolbar">
        <button className="task-board-new" onClick={() => setEditing("new")}>
          + {t.board_new_card}
        </button>
      </div>
      <div className="task-board-columns">
        {COLUMNS.map((s) => (
          <TaskColumn key={s} status={s} title={colTitle(s)} count={byStatus(s).length} highlighted={dragOverStatus === s}>
            {byStatus(s).map((cardRow) => {
              const draggableCard =
                cardRow.status === "planning" ||
                cardRow.status === "queued" ||
                (cardRow.status === "running" && cardRow.interactive);
              const isDragging = draggingCardId === cardRow.id;
              const classes = ["task-card-drag-wrap"];
              if (draggableCard) classes.push("task-card-drag-wrap--draggable");
              if (isDragging) classes.push("task-card-drag-wrap--dragging");
              return (
                <div
                  key={cardRow.id}
                  data-task-drag-id={cardRow.id}
                  className={classes.join(" ")}
                  onMouseDown={(e) => handleCardMouseDown(e, cardRow)}
                >
                  <TaskCard
                    projectId={projectId}
                    card={cardRow}
                    onEdit={() => setEditing(cardRow)}
                    onViewTranscript={() => setTranscriptFor(cardRow.id)}
                    onChanged={() => void refresh()}
                  />
                </div>
              );
            })}
          </TaskColumn>
        ))}
      </div>

      {editing && (
        <TaskEditorDialog
          projectId={projectId}
          card={editing === "new" ? null : editing}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null);
            void refresh();
          }}
        />
      )}
      {transcriptFor &&
        (() => {
          const transcriptCard = tasks.find((x) => x.id === transcriptFor);
          if (!transcriptCard) return null;
          return (
            <TranscriptDialog
              projectId={projectId}
              taskId={transcriptFor}
              body={transcriptCard.body}
              onClose={() => setTranscriptFor(null)}
            />
          );
        })()}

      {draggingCardId &&
        dragPointer &&
        (() => {
          const draggedCard = tasks.find((x) => x.id === draggingCardId);
          if (!draggedCard) return null;
          return createPortal(
            <div
              className="task-card-ghost"
              data-testid="task-drag-ghost"
              style={{ left: `${dragPointer.x}px`, top: `${dragPointer.y}px` }}
            >
              {/* 渲染真的 TaskCard，不是另外手刻一份簡化版——這樣拖曳中看到的
                  內容（徽章/狀態色條/頭像/按鈕）永遠跟靜止的卡片一致，日後
                  改卡片也不會漏改這裡。回呼給 no-op：.task-card-ghost 是
                  pointer-events: none，這些按鈕點不到。 */}
              <TaskCard
                projectId={projectId}
                card={draggedCard}
                onEdit={() => {}}
                onViewTranscript={() => {}}
                onChanged={() => {}}
              />
            </div>,
            document.body,
          );
        })()}
    </div>
  );
}
