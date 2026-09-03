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
  sort_order: number;
  outcome: TaskOutcome | null;
  tab_id: string | null;
  transcript_path: string | null;
  error_message: string | null;
  created_at: string;
  dispatched_at: number | null;
  finished_at: number | null;
}

export interface TaskWithAttachments extends TaskRow {
  attachments: AttachmentRow[];
}

export interface TaskBoardConfig {
  max_concurrent: number;
  claude_command: string;
}

// ── Commands ────────────────────────────────────────────────────────────────

export const listTasks = (): Promise<TaskWithAttachments[]> => invoke("tasks_list");

export const createTask = (args: {
  title: string;
  body: string;
  project_dir: string;
  parallel_ok: boolean;
}): Promise<string> => invoke("tasks_create", { args });

export const cloneTask = (id: string): Promise<string> => invoke("tasks_clone", { id });

export const updateTask = (args: {
  id: string;
  title: string;
  body: string;
  project_dir: string;
  parallel_ok: boolean;
}): Promise<void> => invoke("tasks_update", { args });

export const moveTask = (id: string, to_status: TaskStatus, sort_order: number): Promise<void> =>
  invoke("tasks_move", { args: { id, to_status, sort_order } });

export const stopTask = (id: string): Promise<void> => invoke("tasks_stop", { id });

export const deleteTask = (id: string, close_tab: boolean): Promise<void> =>
  invoke("tasks_delete", { args: { id, close_tab } });

export const addAttachment = (
  id: string,
  filename: string,
  bytes: Uint8Array,
): Promise<AttachmentRow> =>
  invoke("tasks_add_attachment", { args: { id, filename, bytes: Array.from(bytes) } });

export const removeAttachment = (attachmentId: string): Promise<void> =>
  invoke("tasks_remove_attachment", { attachmentId });

export const readTranscript = (id: string): Promise<string> =>
  invoke("tasks_read_transcript", { id });

export const saveTranscript = (id: string, text: string): Promise<void> =>
  invoke("tasks_save_transcript", { id, text });

export const getTaskBoardConfig = (): Promise<TaskBoardConfig> => invoke("task_board_get_config");

export const setTaskBoardConfig = (value: TaskBoardConfig): Promise<void> =>
  invoke("task_board_set_config", { value });

/** Fires (no payload) after any task mutation or scheduler transition. */
export const onTasksUpdated = (cb: () => void): Promise<UnlistenFn> =>
  listen("tasks-updated", () => cb());
