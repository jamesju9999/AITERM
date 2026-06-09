import { invoke } from "@tauri-apps/api/core";

/**
 * Convert a local file to Markdown using MarkItDown (Python backend).
 * Resolves with the Markdown string, rejects with an error message on failure.
 */
export function markitdownConvert(filePath: string): Promise<string> {
  return invoke<string>("markitdown_convert", { filePath });
}

/**
 * Open a native OS file picker filtered to supported document formats.
 * Resolves with the selected file path, or null if the user cancelled.
 */
export function markitdownPickFile(): Promise<string | null> {
  return invoke<string | null>("markitdown_pick_file");
}
