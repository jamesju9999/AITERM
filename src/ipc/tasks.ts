import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ── Types (mirror Rust tasks/store.rs) ───────────────────────────────────────

export type TaskStatus = "planning" | "queued" | "running" | "done";
export type TaskOutcome = "success" | "failed" | "cancelled";

export interface AttachmentRow {
  id: string;
  task_id: string;
  filename: string;
  stored_path: string;
}

export interface TaskRow {
  id: string;
  title: string;
  body: string;
  project_dir: string;
  status: TaskStatus;
  parallel_ok: boolean;
  interactive: boolean;
  sort_order: number;
  outcome: TaskOutcome | null;
  tab_id: string | null;
  transcript_path: string | null;
  error_message: string | null;
  created_at: string;
  dispatched_at: number | null;
  finished_at: number | null;
  /** AI 履行摘要（工作報告用）。只有跑過的卡片會有。 */
  ai_summary: string | null;
  /** 封存時間（Unix 秒）。有值代表已經從看板上收起來，資料仍完整保留。 */
  archived_at: number | null;
}

export interface TaskWithAttachments extends TaskRow {
  attachments: AttachmentRow[];
}

export interface TaskBoardConfig {
  max_concurrent: number;
  claude_command: string;
}

// ── Commands ────────────────────────────────────────────────────────────────

export const listTasks = (projectId: string): Promise<TaskWithAttachments[]> =>
  invoke("tasks_list", { projectId });

export const createTask = (
  projectId: string,
  args: {
    title: string;
    body: string;
    project_dir: string;
    parallel_ok: boolean;
    interactive: boolean;
  },
): Promise<string> => invoke("tasks_create", { projectId, args });

export const cloneTask = (projectId: string, id: string): Promise<string> =>
  invoke("tasks_clone", { projectId, id });

export const updateTask = (
  projectId: string,
  args: {
    id: string;
    title: string;
    body: string;
    project_dir: string;
    parallel_ok: boolean;
    interactive: boolean;
  },
): Promise<void> => invoke("tasks_update", { projectId, args });

export const moveTask = (
  projectId: string,
  id: string,
  to_status: TaskStatus,
  sort_order: number,
): Promise<void> => invoke("tasks_move", { projectId, args: { id, to_status, sort_order } });

export const stopTask = (projectId: string, id: string): Promise<void> =>
  invoke("tasks_stop", { projectId, id });

export const markTaskDone = (projectId: string, id: string): Promise<void> =>
  invoke("tasks_mark_done", { projectId, id });

export const deleteTask = (
  projectId: string,
  id: string,
  close_tab: boolean,
): Promise<void> => invoke("tasks_delete", { projectId, args: { id, close_tab } });

export const addAttachment = (
  projectId: string,
  id: string,
  filename: string,
  bytes: Uint8Array,
): Promise<AttachmentRow> =>
  invoke("tasks_add_attachment", {
    projectId,
    args: { id, filename, bytes: Array.from(bytes) },
  });

export const removeAttachment = (projectId: string, attachmentId: string): Promise<void> =>
  invoke("tasks_remove_attachment", { projectId, attachmentId });

export const readTranscript = (projectId: string, id: string): Promise<string> =>
  invoke("tasks_read_transcript", { projectId, id });

export const saveTranscript = (projectId: string, id: string, text: string): Promise<void> =>
  invoke("tasks_save_transcript", { projectId, id, text });

export const setSummary = (projectId: string, id: string, summary: string): Promise<void> =>
  invoke("tasks_set_summary", { projectId, taskId: id, summary });

/** 把一張已完成的卡片從看板上收起來。 */
export const archiveTask = (projectId: string, taskId: string): Promise<void> =>
  invoke("tasks_archive", { projectId, taskId });

/** 把封存的卡片放回「已完成」欄。 */
export const unarchiveTask = (projectId: string, taskId: string): Promise<void> =>
  invoke("tasks_unarchive", { projectId, taskId });

/** 一次收走整個「已完成」欄，回傳實際封存了幾張。 */
export const archiveDoneTasks = (projectId: string): Promise<number> =>
  invoke("tasks_archive_done", { projectId });

/** 鏡射 Rust 的 `commands::tasks::ArchivePage`。 */
export interface ArchivePage {
  rows: TaskRow[];
  /** 符合搜尋條件的總數，分頁靠它算頁數。 */
  total: number;
}

/**
 * 封存清單的一頁，新封存的在前。`query` 空字串代表不過濾。
 *
 * 過濾與分頁都在 SQL 做——封存清單是唯一被設計成會無限成長的地方。
 */
export const listArchivedTasks = (
  projectId: string,
  query: string,
  limit: number,
  offset: number,
): Promise<ArchivePage> =>
  invoke("tasks_list_archived", { projectId, query, limit, offset });

export const getTaskBoardConfig = (): Promise<TaskBoardConfig> => invoke("task_board_get_config");

export const setTaskBoardConfig = (value: TaskBoardConfig): Promise<void> =>
  invoke("task_board_set_config", { value });

/** Fires (no payload) after any task mutation or scheduler transition. */
export const onTasksUpdated = (cb: () => void): Promise<UnlistenFn> =>
  listen("tasks-updated", () => cb());
