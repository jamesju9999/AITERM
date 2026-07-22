import { invoke } from "@tauri-apps/api/core";
import type { ChatMessage } from "./ai";

export interface Notebook {
  id: string;
  name: string;
  folder_path: string;
  embed_provider_id: string | null;
  embed_model: string | null;
  embed_dim: number | null;
  last_synced_at: number | null;
  created_at: string;
}

export interface SyncSummary {
  indexed: number;
  failed: number;
  deleted: number;
}

export interface ChatSessionSummary {
  id: string;
  title: string;
  updated_at: string;
}

export interface ChatMessageRow {
  role: string;
  content: string;
  tool_calls_json: string | null;
  created_at: string;
}

export type KbSyncEvent =
  | { kind: "progress"; notebook_id: string; processed: number; total: number; current_file: string }
  | { kind: "done"; notebook_id: string; indexed: number; failed: number; deleted: number };

export type KbChatEvent =
  | { kind: "tool_call"; session_id: string; call_id: string; tool: string; args: Record<string, unknown> }
  | { kind: "tool_result"; session_id: string; call_id: string; content: string; truncated: boolean }
  | { kind: "text_delta"; session_id: string; delta: string }
  | { kind: "checkpoint"; session_id: string; number: number }
  | { kind: "clear_content"; session_id: string }
  | { kind: "done"; session_id: string }
  | { kind: "error"; session_id: string; message: string }
  | { kind: "fallback_mode"; session_id: string }
  | { kind: "token_count"; session_id: string; count: number; limit: number };

export const KB_SYNC_EVENT = "kb-sync-event";
export const KB_CHAT_EVENT = "kb-chat-event";

export function kbCreateNotebook(
  name: string,
  folderPath: string,
  embedProviderId?: string | null,
  embedModel?: string | null,
): Promise<Notebook> {
  return invoke<Notebook>("kb_create_notebook", {
    name,
    folderPath,
    embedProviderId: embedProviderId ?? null,
    embedModel: embedModel ?? null,
  });
}

export function kbListNotebooks(): Promise<Notebook[]> {
  return invoke<Notebook[]>("kb_list_notebooks");
}

export function kbDeleteNotebook(id: string): Promise<void> {
  return invoke<void>("kb_delete_notebook", { id });
}

export function kbSyncNotebook(notebookId: string): Promise<SyncSummary> {
  return invoke<SyncSummary>("kb_sync_notebook", { notebookId });
}

export function invokeKbChat(
  notebookId: string,
  messages: ChatMessage[],
  sessionId: string,
  chatSessionId: string,
  providerId?: string | null,
  locale: string = "zh-TW",
): Promise<void> {
  return invoke<void>("kb_chat", {
    notebookId,
    messages,
    sessionId,
    chatSessionId,
    providerId: providerId ?? null,
    locale,
  });
}

export function kbOpenDocument(notebookId: string, relPath: string): Promise<void> {
  return invoke<void>("kb_open_document", { notebookId, relPath });
}

export function kbCreateChatSession(notebookId: string, title: string): Promise<string> {
  return invoke<string>("kb_create_chat_session", { notebookId, title });
}

export function kbListChatSessions(notebookId: string): Promise<ChatSessionSummary[]> {
  return invoke<ChatSessionSummary[]>("kb_list_chat_sessions", { notebookId });
}

export function kbLoadChatSession(sessionId: string): Promise<ChatMessageRow[]> {
  return invoke<ChatMessageRow[]>("kb_load_chat_session", { sessionId });
}

export function kbDeleteChatSession(sessionId: string): Promise<void> {
  return invoke<void>("kb_delete_chat_session", { sessionId });
}
