// src/ipc/design.ts
import { invoke } from '@tauri-apps/api/core';
import type { ChatMessage } from './ai';

export interface DesignSession {
  id: string;
  title: string;
  current_proposal_draft: string | null;
  current_spec_draft: string | null;
  current_sdd_draft: string | null;
  current_plan_draft: string | null;
  context_summary: string | null;
  status: 'draft' | 'proposal_approved' | 'spec_approved' | 'sdd_approved' | 'approved';
}

export interface DesignChatReply {
  content: string;
}

/**
 * Advance the design session to the next workflow stage.
 */
export async function designAdvanceStage(
  sessionId: string, 
  nextStatus: DesignSession['status']
): Promise<boolean> {
  return invoke('design_advance_stage', { sessionId, nextStatus });
}

/**
 * Start a new design session.
 */
export async function designStartSession(title: string): Promise<string> {
  return invoke('design_start_session', { title });
}

/**
 * Load a design session by ID.
 */
export async function designLoadSession(id: string): Promise<DesignSession> {
  return invoke('design_load_session', { id: id });
}

/**
 * List all available design sessions.
 */
export async function designListSessions(): Promise<DesignSession[]> {
  return invoke('design_list_sessions');
}

/**
 * List all messages in a design session.
 */
export async function designListMessages(sessionId: string): Promise<ChatMessage[]> {
  return invoke('design_list_messages', { sessionId });
}

/**
 * Delete a design session and its messages.
 */
export async function designDeleteSession(sessionId: string): Promise<boolean> {
  return invoke('design_delete_session', { sessionId });
}

/**
 * Send a message to AI within a design session.
 * Supports streaming via 'ai-stream' event.
 */
export async function designChat(
  sessionId: string, 
  messages: ChatMessage[], 
  providerId?: string
): Promise<DesignChatReply> {
  return invoke('design_chat', { sessionId, messages, providerId: providerId ?? null });
}

/**
 * Update a draft in the design session (Spec, Architecture, or Plan).
 * MUST match Rust params exactly (Tauri automatically converts camelCase to snake_case).
 */
export async function designUpdateDraft(
  sessionId: string, 
  field: 'proposal' | 'spec' | 'sdd' | 'plan',
  content: string
): Promise<boolean> {
  return invoke('design_update_draft', { sessionId, field, content });
}

/**
 * Save draft content to a file on the local filesystem.
 */
export async function designSaveFile(
  filePath: string,
  content: string
): Promise<boolean> {
  return invoke('design_save_file', { filePath, content });
}