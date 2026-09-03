import { useCallback, useEffect, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";

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

/** Below this many pixels of movement, a mousedown+mouseup is a click, not a
 * drag — same threshold TabBar's own (proven-working) drag uses. */
const DRAG_THRESHOLD_PX = 4;

interface DragState {
  id: string;
  startX: number;
  startY: number;
  started: boolean;
}

export function TaskBoardView() {
  const { t } = useLocale();
  const [tasks, setTasks] = useState<TaskWithAttachments[]>([]);
  const [editing, setEditing] = useState<TaskWithAttachments | "new" | null>(null);
  const [transcriptFor, setTranscriptFor] = useState<string | null>(null);
  const mounted = useRef(true);
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
  /** Cursor delta from the drag's start point, applied as a translate() on
   * the dragged card so it visibly leaves its column and follows the mouse
   * — a static fade-in-place wasn't legible feedback that anything was
   * happening. */
  const [dragOffset, setDragOffset] = useState<{ dx: number; dy: number } | null>(null);
  // DEBUG-TEMP: on-screen diagnostic overlay while root-causing a real drag
  // bug live in the running app (no way to script a real OS mouse drag from
  // this environment to self-verify). Remove once the drag is confirmed
  // working.
  const [debugLog, setDebugLog] = useState<string[]>([]);
  const pushDebug = useCallback((msg: string) => {
    setDebugLog((prev) => [...prev.slice(-9), `${new Date().toISOString().slice(11, 23)} ${msg}`]);
  }, []);

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
    async (id: string, to: TaskStatus) => {
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
    let moveEventCount = 0;
    let lastLoggedStatus: TaskStatus | null | "unset" = "unset";
    const onMove = (e: MouseEvent) => {
      const st = dragRef.current;
      if (!st) return;
      moveEventCount++;
      if (!st.started) {
        const dist = Math.hypot(e.clientX - st.startX, e.clientY - st.startY);
        if (dist < DRAG_THRESHOLD_PX) return;
        st.started = true;
        setDraggingCardId(st.id);
        pushDebug(`threshold crossed after ${moveEventCount} move events (dist=${dist.toFixed(0)}px) → started=true`);
      }
      setDragOffset({ dx: e.clientX - st.startX, dy: e.clientY - st.startY });
      const over = statusUnderPoint(e.clientX, e.clientY);
      if (over !== lastLoggedStatus) {
        lastLoggedStatus = over;
        const el = document.elementFromPoint(e.clientX, e.clientY);
        pushDebug(`over changed → ${over ?? "null"} (elementFromPoint tag=${el?.tagName ?? "null"}.${(el as HTMLElement | null)?.className ?? ""})`);
      }
      setDragOverStatus(over);
    };
    const onUp = (e: MouseEvent) => {
      const st = dragRef.current;
      dragRef.current = null;
      setDragOverStatus(null);
      setDraggingCardId(null);
      setDragOffset(null);
      if (!st?.started) {
        pushDebug(`mouseup: drag never started (armed=${!!st}, moveEvents=${moveEventCount})`);
        return;
      }
      const to = statusUnderPoint(e.clientX, e.clientY);
      pushDebug(`mouseup at (${e.clientX},${e.clientY}) → resolved status=${to ?? "null"}${to ? " → calling moveTask" : " → NOT calling moveTask"}`);
      if (to) void handleDrop(st.id, to);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [handleDrop, statusUnderPoint, pushDebug]);

  const handleCardMouseDown = (e: ReactMouseEvent<HTMLDivElement>, cardRow: TaskWithAttachments) => {
    pushDebug(`mousedown id=${cardRow.id} status=${cardRow.status} button=${e.button}`);
    if (e.button !== 0) return;
    if (cardRow.status !== "planning" && cardRow.status !== "queued") {
      pushDebug(`mousedown ignored: status ${cardRow.status} not draggable`);
      return;
    }
    dragRef.current = { id: cardRow.id, startX: e.clientX, startY: e.clientY, started: false };
    pushDebug(`armed drag for ${cardRow.id} at (${e.clientX},${e.clientY})`);
  };

  return (
    <div className="task-board">
      <div className="task-board-toolbar">
        <button className="task-board-new" onClick={() => setEditing("new")}>
          + {t.board_new_card}
        </button>
      </div>
      <div className="task-board-columns">
        {COLUMNS.map((s) => (
          <TaskColumn key={s} status={s} title={colTitle(s)} count={byStatus(s).length} highlighted={dragOverStatus === s}>
            {byStatus(s).map((cardRow) => {
              const draggableCard = cardRow.status === "planning" || cardRow.status === "queued";
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
                  style={
                    isDragging && dragOffset
                      ? { transform: `translate(${dragOffset.dx}px, ${dragOffset.dy}px)` }
                      : undefined
                  }
                >
                  <TaskCard
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

      {/* DEBUG-TEMP: remove once the drag bug is confirmed fixed live. */}
      <div
        style={{
          position: "fixed",
          right: 8,
          bottom: 8,
          width: 480,
          maxHeight: 220,
          overflowY: "auto",
          background: "rgba(0,0,0,0.85)",
          color: "#0f0",
          fontFamily: "monospace",
          fontSize: 11,
          padding: 8,
          borderRadius: 6,
          zIndex: 999,
          whiteSpace: "pre-wrap",
          pointerEvents: "none",
        }}
      >
        <div style={{ color: "#fff", marginBottom: 4 }}>DEBUG (temporary) — drag event log:</div>
        {debugLog.length === 0 ? "(no events yet — try dragging a card)" : debugLog.join("\n")}
      </div>
    </div>
  );
}
