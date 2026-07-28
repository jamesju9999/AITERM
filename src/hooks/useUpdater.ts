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

/** Matches the convention in FileViewer.tsx and useAiChat.ts. */
function errorMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export function useUpdater(): UpdaterApi {
  const [state, setState] = useState<UpdaterState>({ status: "idle" });
  const [dismissed, setDismissed] = useState(false);
  // Tracks "an update exists" independently of the transient checking/error
  // states. Deriving hasUpdate from `state` made the TabBar dot blink off during
  // a manual re-check, and disappear entirely if that re-check failed offline.
  const [pendingVersion, setPendingVersion] = useState<string | null>(null);

  // Only populated once the update is confirmed installable. Leaving it null is
  // what makes install() a no-op on install types the updater cannot service.
  const pendingRef = useRef<PendingUpdate | null>(null);
  // Stops a double-click starting two concurrent installs (which would race on
  // the same install target), and stops check() from stomping a live download.
  // Must be a ref: install()'s closure would see a stale `state`.
  const installingRef = useRef(false);

  // Guard against setState after unmount (Tauri async invoke race), same as
  // useAiChat does.
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  const set = useCallback((next: UpdaterState) => {
    if (mountedRef.current) setState(next);
  }, []);

  const runCheck = useCallback(async (silent: boolean) => {
    // Never disturb an install in progress: the checking/available transitions
    // would stomp on `downloading` and swap pendingRef out from under it.
    if (installingRef.current) return;

    set({ status: "checking" });
    try {
      const update = (await check()) as PendingUpdate | null;
      if (!update) {
        pendingRef.current = null;
        setPendingVersion(null);
        set({ status: "none" });
        return;
      }
      // Held back until the gate passes — see commands/updater.rs.
      pendingRef.current = null;
      setPendingVersion(update.version);
      setDismissed(false);
      const supported = await invoke<boolean>("updater_supported");
      if (!supported) {
        set({ status: "unsupported", version: update.version });
        return;
      }
      pendingRef.current = update;
      set({ status: "available", version: update.version, notes: update.body ?? "" });
    } catch (e) {
      // The mount check is best-effort: an offline user should not see an error.
      // pendingVersion is deliberately left alone so a failed re-check does not
      // erase an update we already know about.
      set(silent ? { status: "idle" } : { status: "error", message: errorMessage(e) });
    }
  }, [set]);

  const check_ = useCallback(() => runCheck(false), [runCheck]);

  const install = useCallback(async () => {
    const update = pendingRef.current;
    if (!update || installingRef.current) return;
    installingRef.current = true;

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
      set({ status: "error", message: errorMessage(e) });
    } finally {
      installingRef.current = false;
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

  return {
    state,
    hasUpdate: pendingVersion !== null,
    dismissed,
    check: check_,
    install,
    relaunch,
    dismiss,
  };
}
