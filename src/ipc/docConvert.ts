import { invoke } from "@tauri-apps/api/core";

/**
 * Convert a local file to Markdown, routed between anydoc and MarkItDown
 * per the user's doc_convert_engine setting. Resolves with the Markdown
 * string, rejects with an error message on failure.
 */
export function documentConvert(filePath: string, providerId?: string): Promise<string> {
  return invoke<string>("document_convert", { filePath, providerId: providerId ?? null });
}

/**
 * Open a native OS file picker filtered to supported document formats.
 * Resolves with the selected file path, or null if the user cancelled.
 */
export function documentConvertPickFile(): Promise<string | null> {
  return invoke<string | null>("document_convert_pick_file");
}
