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

export interface FileContent {
  content: string;
  truncated: boolean;
}

/** Read a text file's content. Throws if binary or unreadable. */
export const readFile = (path: string): Promise<FileContent> =>
  invoke<FileContent>("pty_read_file", { path });

/** Overwrite (or create) a text file with the given UTF-8 content. */
export const writeTextFile = (path: string, content: string): Promise<void> =>
  invoke<void>("write_text_file", { path, content });

/**
 * Read any file and return its content as an ArrayBuffer.
 * Used by Doc Converter for files dropped via OS drag-and-drop.
 */
export async function readFileAsArrayBuffer(path: string): Promise<ArrayBuffer> {
  const b64: string = await invoke("read_file_as_bytes", { path });
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes.buffer;
}
