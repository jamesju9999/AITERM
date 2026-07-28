import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch as processRelaunch } from "@tauri-apps/plugin-process";

export type UpdaterState =
  | { status: "idle" }
  | { status: "checking" }
  | { status: "none" }
  | { status: "available"; version: string; notes: string }
  | { status: "downloading"; version: string; downloaded: number; total: number | null }
  | { status: "ready"; version: string }
  | { status: "unsupported"; version: string }
  | { status: "error"; message: string };

export interface UpdaterApi {
  state: UpdaterState;
  /** True while an update exists, regardless of whether the modal was dismissed. */
  hasUpdate: boolean;
  dismissed: boolean;
  check: () => Promise<void>;
  install: () => Promise<void>;
  relaunch: () => Promise<void>;
  dismiss: () => void;
}

/** The plugin's Update object, narrowed to what we use. */
interface PendingUpdate {
  version: string;
  body?: string;
  downloadAndInstall: (onEvent: (event: DownloadEvent) => void) => Promise<void>;
}

type DownloadEvent =
  | { event: "Started"; data: { contentLength?: number } }
  | { event: "Progress"; data: { chunkLength: number } }
  | { event: "Finished" };

export function useUpdater(): UpdaterApi {
  const [state, setState] = useState<UpdaterState>({ status: "idle" });
  const [dismissed, setDismissed] = useState(false);
  const pendingRef = useRef<PendingUpdate | null>(null);

  // Tauri's async plugin calls can resolve after unmount; guard setState the
  // same way useAiChat does to avoid the React warning.
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  const set = useCallback((next: UpdaterState) => {
    if (mountedRef.current) setState(next);
  }, []);

  const runCheck = useCallback(async (silent: boolean) => {
    set({ status: "checking" });
    try {
      const update = (await check()) as PendingUpdate | null;
      if (!update) {
        set({ status: "none" });
        return;
      }
      pendingRef.current = update;
      setDismissed(false);
      // Gate before offering the one-click path — see commands/updater.rs.
      const supported = await invoke<boolean>("updater_supported");
      if (!supported) {
        set({ status: "unsupported", version: update.version });
        return;
      }
      set({ status: "available", version: update.version, notes: update.body ?? "" });
    } catch (e) {
      // The mount check is best-effort: an offline user should not see an error.
      set(silent ? { status: "idle" } : { status: "error", message: String(e) });
    }
  }, [set]);

  const check_ = useCallback(() => runCheck(false), [runCheck]);

  const install = useCallback(async () => {
    const update = pendingRef.current;
    if (!update) return;

    let total: number | null = null;
    let downloaded = 0;
    set({ status: "downloading", version: update.version, downloaded: 0, total: null });

    try {
      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            total = event.data.contentLength ?? null;
            set({ status: "downloading", version: update.version, downloaded: 0, total });
            break;
          case "Progress":
            downloaded += event.data.chunkLength;
            set({ status: "downloading", version: update.version, downloaded, total });
            break;
          case "Finished":
            set({ status: "ready", version: update.version });
            break;
        }
      });
      set({ status: "ready", version: update.version });
    } catch (e) {
      set({ status: "error", message: String(e) });
    }
  }, [set]);

  const relaunch = useCallback(async () => {
    await processRelaunch();
  }, []);

  const dismiss = useCallback(() => setDismissed(true), []);

  // Auto-check once on mount. The ref keeps StrictMode's double-invoke in dev
  // from firing two network requests.
  const autoCheckedRef = useRef(false);
  useEffect(() => {
    if (autoCheckedRef.current) return;
    autoCheckedRef.current = true;
    void runCheck(true);
  }, [runCheck]);

  const hasUpdate =
    state.status === "available" ||
    state.status === "downloading" ||
    state.status === "ready" ||
    state.status === "unsupported";

  return { state, hasUpdate, dismissed, check: check_, install, relaunch, dismiss };
}
