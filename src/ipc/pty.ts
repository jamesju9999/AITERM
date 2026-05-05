import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  ptyDataEvent,
  type PtyDataPayload,
} from "./events";

export interface PtySize {
  rows: number;
  cols: number;
}

export function createPty(size: PtySize): Promise<string> {
  return invoke<string>("pty_create", { size });
}

export function writePty(id: string, data: string): Promise<void> {
  return invoke<void>("pty_write", { id, data });
}

export function resizePty(id: string, size: PtySize): Promise<void> {
  return invoke<void>("pty_resize", { id, size });
}

export function closePty(id: string): Promise<void> {
  return invoke<void>("pty_close", { id });
}

/** Return the last ~4 KiB of ANSI-stripped output for the session, or null. */
export function getPtyRecentOutput(id: string): Promise<string | null> {
  return invoke<string | null>("pty_get_recent_output", { id });
}

/** Return the shell type for a session ("pwsh", "cmd", "bash", or "unknown"). */
export function getPtyShellType(id: string): Promise<string | null> {
  return invoke<string | null>("pty_get_shell_type", { id });
}

/**
 * Subscribe to PTY output for a given session. Returns an unlisten function.
 * The callback receives already-decoded bytes as a Uint8Array.
 */
export async function onPtyData(
  sessionId: string,
  onChunk: (bytes: Uint8Array) => void,
): Promise<UnlistenFn> {
  return listen<PtyDataPayload>(ptyDataEvent(sessionId), (event) => {
    const binary = atob(event.payload.base64);
    const out = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
    onChunk(out);
  });
}
