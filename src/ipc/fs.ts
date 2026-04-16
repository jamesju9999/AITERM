import { invoke } from "@tauri-apps/api/core";

export interface DirEntry {
  name: string;
  path: string;
  is_dir: boolean;
  size: number | null;
}

/** Get the current working directory of a PTY session. */
export const getSessionCwd = (id: string): Promise<string | null> =>
  invoke<string | null>("pty_get_cwd", { id });

/** List directory children. Pass an empty path to list the session's CWD. */
export const listDirectory = (id: string, path: string = ""): Promise<DirEntry[]> =>
  invoke<DirEntry[]>("pty_list_dir", { id, path });
