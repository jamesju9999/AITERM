import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { StrictMode } from "react";

const checkMock = vi.fn();
const relaunchMock = vi.fn();
const invokeMock = vi.fn();

vi.mock("@tauri-apps/plugin-updater", () => ({
  check: (...args: unknown[]) => checkMock(...args),
}));
vi.mock("@tauri-apps/plugin-process", () => ({
  relaunch: (...args: unknown[]) => relaunchMock(...args),
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { useUpdater } from "./useUpdater";

/** Minimal stand-in for the plugin's Update object. */
function fakeUpdate(overrides: Record<string, unknown> = {}) {
  return {
    version: "1.2.0",
    body: "Bug fixes",
    downloadAndInstall: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };
}

/**
 * A `downloadAndInstall` stub the test drives event-by-event.
 *
 * The hook's intermediate `downloading` states are only observable if React can
 * re-render between events, so the stub parks on a promise the test resolves at
 * the end rather than running to completion synchronously.
 */
function deferredDownload() {
  let onEvent!: (e: unknown) => void;
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  const fn = vi.fn(async (handler: (e: unknown) => void) => {
    onEvent = handler;
    await gate;
  });
  return { fn, emit: (e: unknown) => onEvent(e), release: () => release() };
}

beforeEach(() => {
  checkMock.mockReset();
  relaunchMock.mockReset();
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(true); // updater_supported
});

describe("useUpdater", () => {
  it("reports 'none' when the endpoint has no newer version", async () => {
    checkMock.mockResolvedValue(null);
    const { result } = renderHook(() => useUpdater());

    await waitFor(() => expect(result.current.state.status).toBe("none"));
    expect(result.current.hasUpdate).toBe(false);
  });

  it("reports 'available' with version and notes on a supported install", async () => {
    checkMock.mockResolvedValue(fakeUpdate());
    const { result } = renderHook(() => useUpdater());

    await waitFor(() => expect(result.current.state.status).toBe("available"));
    expect(result.current.state).toEqual({
      status: "available",
      version: "1.2.0",
      notes: "Bug fixes",
    });
    expect(result.current.hasUpdate).toBe(true);
    expect(invokeMock).toHaveBeenCalledWith("updater_supported");
  });

  it("reports 'unsupported' when updater_supported is false (.deb install)", async () => {
    checkMock.mockResolvedValue(fakeUpdate());
    invokeMock.mockResolvedValue(false);
    const { result } = renderHook(() => useUpdater());

    await waitFor(() => expect(result.current.state.status).toBe("unsupported"));
    expect(result.current.state).toEqual({ status: "unsupported", version: "1.2.0" });
    // Still counts as an update so the TabBar dot and the manual link show up.
    expect(result.current.hasUpdate).toBe(true);
  });

  it("falls back to idle when the automatic mount check fails", async () => {
    checkMock.mockRejectedValue(new Error("offline"));
    const { result } = renderHook(() => useUpdater());

    await waitFor(() => expect(result.current.state.status).toBe("idle"));
  });

  it("surfaces the error when a manual check fails", async () => {
    checkMock.mockResolvedValue(null);
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("none"));

    checkMock.mockRejectedValue(new Error("offline"));
    await act(async () => { await result.current.check(); });

    expect(result.current.state.status).toBe("error");
  });

  it("tracks download progress and ends in 'ready'", async () => {
    const download = deferredDownload();
    checkMock.mockResolvedValue(fakeUpdate({ downloadAndInstall: download.fn }));
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("available"));

    let installed: Promise<void> | undefined;
    await act(async () => { installed = result.current.install(); });

    // Before any event arrives the total is not yet known.
    expect(result.current.state).toEqual({
      status: "downloading", version: "1.2.0", downloaded: 0, total: null,
    });

    await act(async () => { download.emit({ event: "Started", data: { contentLength: 1000 } }); });
    expect(result.current.state).toEqual({
      status: "downloading", version: "1.2.0", downloaded: 0, total: 1000,
    });
    // The TabBar dot must stay lit through the download.
    expect(result.current.hasUpdate).toBe(true);

    await act(async () => { download.emit({ event: "Progress", data: { chunkLength: 400 } }); });
    expect(result.current.state).toEqual({
      status: "downloading", version: "1.2.0", downloaded: 400, total: 1000,
    });

    // Accumulates rather than overwrites.
    await act(async () => { download.emit({ event: "Progress", data: { chunkLength: 600 } }); });
    expect(result.current.state).toEqual({
      status: "downloading", version: "1.2.0", downloaded: 1000, total: 1000,
    });

    await act(async () => {
      download.emit({ event: "Finished" });
      download.release();
      await installed;
    });
    expect(result.current.state).toEqual({ status: "ready", version: "1.2.0" });
  });

  it("throttles progress updates to about one percent", async () => {
    const download = deferredDownload();
    checkMock.mockResolvedValue(fakeUpdate({ downloadAndInstall: download.fn }));
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("available"));

    let installed: Promise<void> | undefined;
    await act(async () => { installed = result.current.install(); });
    await act(async () => { download.emit({ event: "Started", data: { contentLength: 10_000 } }); });

    // 10 bytes of 10000 is 0.1% — below the threshold, so nothing is published.
    await act(async () => { download.emit({ event: "Progress", data: { chunkLength: 10 } }); });
    expect(result.current.state).toEqual({
      status: "downloading", version: "1.2.0", downloaded: 0, total: 10_000,
    });

    // Crossing a full percent publishes the accumulated total, not just the chunk.
    await act(async () => { download.emit({ event: "Progress", data: { chunkLength: 200 } }); });
    expect(result.current.state).toEqual({
      status: "downloading", version: "1.2.0", downloaded: 210, total: 10_000,
    });

    // 9950 of 10000: over the threshold, publishes normally.
    await act(async () => { download.emit({ event: "Progress", data: { chunkLength: 9740 } }); });
    expect(result.current.state).toEqual({
      status: "downloading", version: "1.2.0", downloaded: 9950, total: 10_000,
    });

    // The last 50 bytes are under the 100-byte threshold, but completing the
    // download publishes regardless so the bar reaches 100%.
    await act(async () => { download.emit({ event: "Progress", data: { chunkLength: 50 } }); });
    expect(result.current.state).toEqual({
      status: "downloading", version: "1.2.0", downloaded: 10_000, total: 10_000,
    });

    await act(async () => {
      download.emit({ event: "Finished" });
      download.release();
      await installed;
    });
  });

  it("throttles by bytes when the total size is unknown", async () => {
    const download = deferredDownload();
    checkMock.mockResolvedValue(fakeUpdate({ downloadAndInstall: download.fn }));
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("available"));

    let installed: Promise<void> | undefined;
    await act(async () => { installed = result.current.install(); });
    await act(async () => { download.emit({ event: "Started", data: {} }); });

    // Below the byte floor. The indeterminate bar ignores `downloaded` anyway,
    // so publishing here would be pure re-render cost.
    await act(async () => { download.emit({ event: "Progress", data: { chunkLength: 10 } }); });
    expect(result.current.state).toEqual({
      status: "downloading", version: "1.2.0", downloaded: 0, total: null,
    });

    await act(async () => { download.emit({ event: "Progress", data: { chunkLength: 300_000 } }); });
    expect(result.current.state).toEqual({
      status: "downloading", version: "1.2.0", downloaded: 300_010, total: null,
    });

    await act(async () => {
      download.emit({ event: "Finished" });
      download.release();
      await installed;
    });
  });

  it("treats a missing contentLength as an unknown total", async () => {
    const download = deferredDownload();
    checkMock.mockResolvedValue(fakeUpdate({ downloadAndInstall: download.fn }));
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("available"));

    let installed: Promise<void> | undefined;
    await act(async () => { installed = result.current.install(); });

    // Servers are not obliged to send Content-Length; total stays null so the UI
    // can fall back to an indeterminate bar instead of dividing by zero.
    await act(async () => { download.emit({ event: "Started", data: {} }); });
    expect(result.current.state).toEqual({
      status: "downloading", version: "1.2.0", downloaded: 0, total: null,
    });

    await act(async () => { download.emit({ event: "Progress", data: { chunkLength: 300_000 } }); });
    expect(result.current.state).toEqual({
      status: "downloading", version: "1.2.0", downloaded: 300_000, total: null,
    });

    await act(async () => {
      download.emit({ event: "Finished" });
      download.release();
      await installed;
    });
  });

  it("surfaces install failures as an error", async () => {
    const update = fakeUpdate({
      downloadAndInstall: vi.fn().mockRejectedValue(new Error("signature mismatch")),
    });
    checkMock.mockResolvedValue(update);
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("available"));

    await act(async () => { await result.current.install(); });

    expect(result.current.state.status).toBe("error");
    if (result.current.state.status === "error") {
      expect(result.current.state.message).toContain("signature mismatch");
    }
  });

  it("dismiss() hides the modal but keeps hasUpdate true", async () => {
    checkMock.mockResolvedValue(fakeUpdate());
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("available"));

    act(() => { result.current.dismiss(); });

    expect(result.current.dismissed).toBe(true);
    expect(result.current.hasUpdate).toBe(true);
  });

  it("re-shows a dismissed modal when a later check finds a new update", async () => {
    checkMock.mockResolvedValue(fakeUpdate());
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("available"));

    act(() => { result.current.dismiss(); });
    expect(result.current.dismissed).toBe(true);

    checkMock.mockResolvedValue(fakeUpdate({ version: "1.3.0" }));
    await act(async () => { await result.current.check(); });

    expect(result.current.dismissed).toBe(false);
    expect(result.current.state).toEqual({
      status: "available", version: "1.3.0", notes: "Bug fixes",
    });
  });

  it("relaunch() delegates to the process plugin", async () => {
    checkMock.mockResolvedValue(null);
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("none"));

    await act(async () => { await result.current.relaunch(); });

    expect(relaunchMock).toHaveBeenCalledTimes(1);
  });

  it("checks only once on mount under StrictMode double-invoke", async () => {
    checkMock.mockResolvedValue(fakeUpdate());
    const { result } = renderHook(() => useUpdater(), { wrapper: StrictMode });

    await waitFor(() => expect(result.current.state.status).toBe("available"));
    expect(checkMock).toHaveBeenCalledTimes(1);
  });

  it("ignores a second install() while one is already running", async () => {
    const download = deferredDownload();
    checkMock.mockResolvedValue(fakeUpdate({ downloadAndInstall: download.fn }));
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("available"));

    let first: Promise<void> | undefined;
    await act(async () => { first = result.current.install(); });
    await act(async () => { await result.current.install(); });

    // Two concurrent installs would race on the same install target.
    expect(download.fn).toHaveBeenCalledTimes(1);

    await act(async () => {
      download.emit({ event: "Finished" });
      download.release();
      await first;
    });
  });

  it("ignores check() while an install is in flight", async () => {
    const download = deferredDownload();
    checkMock.mockResolvedValue(fakeUpdate({ downloadAndInstall: download.fn }));
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("available"));

    let installed: Promise<void> | undefined;
    await act(async () => { installed = result.current.install(); });
    await act(async () => { download.emit({ event: "Started", data: { contentLength: 100 } }); });

    checkMock.mockResolvedValue(fakeUpdate({ version: "9.9.9" }));
    await act(async () => { await result.current.check(); });

    // The download must be untouched: no "checking" flicker, no swap to 9.9.9.
    expect(result.current.state).toEqual({
      status: "downloading", version: "1.2.0", downloaded: 0, total: 100,
    });

    await act(async () => {
      download.emit({ event: "Finished" });
      download.release();
      await installed;
    });
    expect(result.current.state).toEqual({ status: "ready", version: "1.2.0" });
  });

  it("install() does nothing on an install the updater cannot service", async () => {
    const update = fakeUpdate();
    checkMock.mockResolvedValue(update);
    invokeMock.mockResolvedValue(false);
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("unsupported"));

    await act(async () => { await result.current.install(); });

    expect(update.downloadAndInstall).not.toHaveBeenCalled();
    expect(result.current.state).toEqual({ status: "unsupported", version: "1.2.0" });
  });

  it("keeps hasUpdate true when a manual re-check fails", async () => {
    checkMock.mockResolvedValue(fakeUpdate());
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("available"));

    checkMock.mockRejectedValue(new Error("offline"));
    await act(async () => { await result.current.check(); });

    expect(result.current.state.status).toBe("error");
    // The update we already know about must not disappear from the TabBar.
    expect(result.current.hasUpdate).toBe(true);
  });

  it("allows a retry after a failed install", async () => {
    const downloadAndInstall = vi.fn()
      .mockRejectedValueOnce(new Error("boom"))
      .mockResolvedValue(undefined);
    checkMock.mockResolvedValue(fakeUpdate({ downloadAndInstall }));
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("available"));

    await act(async () => { await result.current.install(); });
    expect(result.current.state.status).toBe("error");

    // The reentrancy flag must be released on failure, or every later install
    // and check would be a silent no-op until the app restarts.
    await act(async () => { await result.current.install(); });

    expect(downloadAndInstall).toHaveBeenCalledTimes(2);
    expect(result.current.state).toEqual({ status: "ready", version: "1.2.0" });
  });

  it("clears the pending update when a later check finds none", async () => {
    const update = fakeUpdate();
    checkMock.mockResolvedValue(update);
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("available"));

    checkMock.mockResolvedValue(null);
    await act(async () => { await result.current.check(); });

    expect(result.current.state.status).toBe("none");
    expect(result.current.hasUpdate).toBe(false);

    // The stale update object must not remain installable.
    await act(async () => { await result.current.install(); });
    expect(update.downloadAndInstall).not.toHaveBeenCalled();
  });

  it("refuses to re-check once an update has been staged", async () => {
    checkMock.mockResolvedValue(fakeUpdate());
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("available"));

    await act(async () => { await result.current.install(); });
    expect(result.current.state).toEqual({ status: "ready", version: "1.2.0" });

    const callsBefore = checkMock.mock.calls.length;
    await act(async () => { await result.current.check(); });

    // Re-checking would overwrite `ready` and re-offer an already-downloaded
    // update, asking the user to fetch the same bytes twice.
    expect(checkMock).toHaveBeenCalledTimes(callsBefore);
    expect(result.current.state).toEqual({ status: "ready", version: "1.2.0" });
  });

  it("does not claim an update when the support probe fails", async () => {
    checkMock.mockResolvedValue(fakeUpdate());
    invokeMock.mockRejectedValue(new Error("command not found"));
    const { result } = renderHook(() => useUpdater());

    await waitFor(() => expect(result.current.state.status).toBe("idle"));
    // A lit dot over an `idle` state renders as a blank About panel.
    expect(result.current.hasUpdate).toBe(false);
  });
});
