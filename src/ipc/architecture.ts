import { invoke } from "@tauri-apps/api/core";

export interface ScanFile { path: string; lang: string; lines: number }
export interface ScanEdge { from: string; to: string }
export interface ScanResult {
  root: string;
  files: ScanFile[];
  edges: ScanEdge[];
  deps: string[];
  truncated: boolean;
  git_head: string | null;
}

export const archScan = (root: string) => invoke<ScanResult>("arch_scan", { root });
export const archLoad = (root: string) => invoke<string | null>("arch_load", { root });
export const archSave = (root: string, json: string) => invoke<void>("arch_save", { root, json });
export const archReadFiles = (root: string, paths: string[]) =>
  invoke<[string, string][]>("arch_read_files", { root, paths });

export interface FilePreview { content: string; truncated: boolean; total_bytes: number }
export const archReadFile = (root: string, path: string) => invoke<FilePreview>("arch_read_file", { root, path });
export const archOpenPath = (root: string, path: string) => invoke<void>("arch_open_path", { root, path });
