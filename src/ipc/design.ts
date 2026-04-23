// src/ipc/design.ts
import { invoke } from '@tauri-apps/api/core';

export interface DesignSession {
  id: string;
  title: string;
  current_spec_draft: string | null;
  current_sdd_draft: string | null;
  current_plan_draft: string | null;
  context_summary: string | null;
  status: 'draft' | 'review' | 'approved';
}

/**
 * Start a new design session.
 * @param title The initial title for the session.
 * @returns The ID of the created session.
 */
export async function designStartSession(title: string): Promise<string> {
  return invoke('design_start_session', { title });
}

/**
 * Load a design session by ID.
 * @param id The session ID.
 */
export async function designLoadSession(id: string): Promise<DesignSession> {
  return invoke('design_load_session', { id });
}

/**
 * List all available design sessions.
 */
export async function designListSessions(): Promise<DesignSession[]> {
  return invoke('design_list_sessions');
}
