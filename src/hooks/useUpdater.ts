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
  | { status: "error"; phase: "check" | "install"; message: string };

/**
 * Download progress as a whole percent, or null when it cannot be computed.
 *
 * `total` is null when the server omits Content-Length and 0 when it reports an
 * empty body; neither yields a usable percentage — 0/0 is NaN and n/0 is
 * Infinity, which `Math.min` clamps to a confident, wrong 100%. Both callers
 * render an indeterminate bar instead.
 *
 * Shared so the guard cannot drift between the modal and the About page.
 */
export function downloadPercent(state: UpdaterState): number | null {
  if (state.status !== "downloading" || state.total === null || state.total <= 0) return null;
  return Math.min(100, Math.round((state.downloaded / state.total) * 100));
}

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

/**
 * Publish interval when the server omits Content-Length. A 30MB download
 * publishes ~120 times, comparable to the one-percent rule below.
 */
const UNKNOWN_TOTAL_STEP = 256 * 1024;

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
  // Set once an install has been staged. Nothing clears it: the only exit from
  // `ready` is a relaunch, which restarts the process. Without this, a manual
  // check would overwrite `ready` and re-offer an update the user has already
  // downloaded.
  const stagedRef = useRef(false);

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
    // Never disturb an install in progress, and never discard one that has
    // already been staged and is waiting for the user to restart.
    if (installingRef.current || stagedRef.current) return;

    // Any check the user explicitly asked for must be allowed to surface its
    // result. There is no periodic background check — only the mount check
    // and the About page button — so this cannot resurrect a toast the user
    // dismissed without asking for anything.
    setDismissed(false);
    set({ status: "checking" });
    try {
      const update = (await check()) as PendingUpdate | null;
      if (!update) {
        pendingRef.current = null;
        setPendingVersion(null);
        set({ status: "none" });
        return;
      }
      // Nothing claims an update until the probe succeeds: a rejected probe
      // would otherwise light the TabBar dot while leaving the UI with an
      // `idle` state it renders as blank, and install() permanently inert.
      pendingRef.current = null;
      const supported = await invoke<boolean>("updater_supported");
      setPendingVersion(update.version);
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
      set(silent ? { status: "idle" } : { status: "error", phase: "check", message: errorMessage(e) });
    }
  }, [set]);

  const check_ = useCallback(() => runCheck(false), [runCheck]);

  const install = useCallback(async () => {
    const update = pendingRef.current;
    if (!update || installingRef.current) return;
    installingRef.current = true;

    let total: number | null = null;
    let downloaded = 0;
    let lastPublished = 0;
    set({ status: "downloading", version: update.version, downloaded: 0, total: null });

    try {
      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            total = event.data.contentLength ?? null;
            set({ status: "downloading", version: update.version, downloaded: 0, total });
            break;
          case "Progress": {
            downloaded += event.data.chunkLength;
            // Tauri streams ~8-16KB chunks, so a 30MB update fires thousands of
            // these, and every one re-renders the app tree from AppRoutes down.
            // Publish ~100 times instead: one percent when the size is known, a
            // fixed byte floor when it is not. The unknown-size case matters
            // most — the modal renders an indeterminate bar there and ignores
            // `downloaded` entirely, so extra updates buy literally nothing.
            const step = total !== null && total > 0 ? total / 100 : UNKNOWN_TOTAL_STEP;
            if (downloaded - lastPublished >= step || downloaded === total) {
              lastPublished = downloaded;
              set({ status: "downloading", version: update.version, downloaded, total });
            }
            break;
          }
          case "Finished":
            set({ status: "ready", version: update.version });
            break;
        }
      });
      stagedRef.current = true;
      pendingRef.current = null;
      set({ status: "ready", version: update.version });
    } catch (e) {
      set({ status: "error", phase: "install", message: errorMessage(e) });
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
