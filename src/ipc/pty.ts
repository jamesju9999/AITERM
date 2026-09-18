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

export function createPty(
  size: PtySize,
  cwd?: string,
  claudeBridge?: boolean,
): Promise<string> {
  return invoke<string>("pty_create", {
    size,
    cwd: cwd ?? null,
    claudeBridge: claudeBridge ?? false,
  });
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

/**
 * Writes a pasted/dropped file's bytes to a real file on disk and returns
 * its path. A `File` from a clipboard paste has no usable filesystem path
 * (unlike OS drag-and-drop), so there's nothing for a program in the PTY to
 * open unless we materialize the bytes ourselves.
 */
export function writePastedFile(name: string, base64Data: string): Promise<string> {
  return invoke<string>("write_pasted_file", { name, base64Data });
}

/** Return the last ~4 KiB of ANSI-stripped output for the session, or null. */
export function getPtyRecentOutput(id: string): Promise<string | null> {
  return invoke<string | null>("pty_get_recent_output", { id });
}

/** Return the shell type for a session ("pwsh", "cmd", "bash", or "unknown"). */
export function getPtyShellType(id: string): Promise<string | null> {
  return invoke<string | null>("pty_get_shell_type", { id });
}

/** 要求對指定 session 提權。回傳 false 代表使用者在 UAC 對話框按了取消。 */
export function elevatePty(id: string): Promise<boolean> {
  return invoke<boolean>("pty_elevate", { id });
}

/** 主動檢查最近一次指令是否像是權限不足失敗（前端在偵測到指令結束時呼叫）。 */
export function checkPermissionDenied(id: string): Promise<boolean> {
  return invoke<boolean>("pty_check_permission_denied", { id });
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
