// src/ipc/design.test.ts
import { describe, it, expect, vi } from 'vitest';
import { designStartSession, designLoadSession, designListSessions } from './design';
import { invoke } from '@tauri-apps/api/core';

// Mock the Tauri invoke function
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

describe('Design IPC Wrappers', () => {
  it('designStartSession invokes the backend command with title', async () => {
    vi.mocked(invoke).mockResolvedValue('session-id-123');
    
    const id = await designStartSession('New App');
    
    expect(invoke).toHaveBeenCalledWith('design_start_session', { title: 'New App' });
    expect(id).toBe('session-id-123');
  });

  it('designLoadSession fetches a specific session', async () => {
    const mockSession = {
      id: 's1',
      title: 'T1',
      current_spec_draft: null,
      current_sdd_draft: null,
      current_plan_draft: null,
      context_summary: null,
      status: 'draft',
    };
    vi.mocked(invoke).mockResolvedValue(mockSession);
    
    const session = await designLoadSession('s1');
    
    expect(invoke).toHaveBeenCalledWith('design_load_session', { id: 's1' });
    expect(session.title).toBe('T1');
  });

  it('designListSessions returns all sessions', async () => {
    vi.mocked(invoke).mockResolvedValue([{ id: 's1', title: 'T1', status: 'draft' }]);
    
    const sessions = await designListSessions();
    
    expect(invoke).toHaveBeenCalledWith('design_list_sessions');
    expect(sessions).toHaveLength(1);
    expect(sessions[0].id).toBe('s1');
  });
});
