// src/ipc/apiDocs.ts
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ── Types mirroring Rust structs ─────────────────────────────────────────────

export interface DocNode {
  title: string;
  href: string;
  items: DocNode[];
}

export interface KeepOptions {
  description: boolean;
  parameters: boolean;
  request_body: boolean;
  responses: boolean;
  code_samples: boolean;
}

export interface ExtractionOptions {
  url: string;
  pages: string[];
  output_dir: string;
  merge: boolean;
  keep: KeepOptions;
  cookies: string;
}

export interface AuthStatus {
  logged_in: boolean;
  account: string;
}

// ── Event payloads ───────────────────────────────────────────────────────────

export interface ApiDocsDetectedEvent {
  platform: string;
  confidence: string;
}

export interface ApiDocsProgressEvent {
  current: number;
  total: number;
  page: string;
}

export interface ApiDocsLogEvent {
  level: "info" | "warn" | "error";
  message: string;
}

export interface ApiDocsDoneEvent {
  files: string[];
}

// ── Commands ─────────────────────────────────────────────────────────────────

export function apiDocsDetect(url: string): Promise<string> {
  return invoke("api_docs_detect", { url });
}

export function apiDocsFetchTree(url: string): Promise<DocNode[]> {
  return invoke("api_docs_fetch_tree", { url });
}

export function apiDocsExtract(options: ExtractionOptions): Promise<void> {
  return invoke("api_docs_extract", { options });
}

export function apiDocsLogin(url: string): Promise<string> {
  return invoke("api_docs_login", { url });
}

export function apiDocsLogout(domain: string): Promise<void> {
  return invoke("api_docs_logout", { domain });
}

export function apiDocsAuthStatus(domain: string): Promise<AuthStatus> {
  return invoke("api_docs_auth_status", { domain });
}

// ── Event listeners ──────────────────────────────────────────────────────────

export function onApiDocsDetected(
  cb: (e: ApiDocsDetectedEvent) => void
): Promise<UnlistenFn> {
  return listen<ApiDocsDetectedEvent>("api-docs-detected", (ev) => cb(ev.payload));
}

export function onApiDocsProgress(
  cb: (e: ApiDocsProgressEvent) => void
): Promise<UnlistenFn> {
  return listen<ApiDocsProgressEvent>("api-docs-progress", (ev) => cb(ev.payload));
}

export function onApiDocsLog(
  cb: (e: ApiDocsLogEvent) => void
): Promise<UnlistenFn> {
  return listen<ApiDocsLogEvent>("api-docs-log", (ev) => cb(ev.payload));
}

export function onApiDocsDone(
  cb: (e: ApiDocsDoneEvent) => void
): Promise<UnlistenFn> {
  return listen<ApiDocsDoneEvent>("api-docs-done", (ev) => cb(ev.payload));
}

export const DEFAULT_KEEP_OPTIONS: KeepOptions = {
  description: true,
  parameters: true,
  request_body: true,
  responses: true,
  code_samples: true,
};
