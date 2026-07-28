import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

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
    const update = fakeUpdate({
      downloadAndInstall: vi.fn(async (onEvent: (e: unknown) => void) => {
        onEvent({ event: "Started", data: { contentLength: 1000 } });
        onEvent({ event: "Progress", data: { chunkLength: 400 } });
        onEvent({ event: "Progress", data: { chunkLength: 600 } });
        onEvent({ event: "Finished" });
      }),
    });
    checkMock.mockResolvedValue(update);
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("available"));

    await act(async () => { await result.current.install(); });

    expect(result.current.state).toEqual({ status: "ready", version: "1.2.0" });
  });

  it("treats a missing contentLength as an unknown total", async () => {
    let seenTotal: number | null | undefined;
    const update = fakeUpdate({
      downloadAndInstall: vi.fn(async (onEvent: (e: unknown) => void) => {
        onEvent({ event: "Started", data: {} });
        onEvent({ event: "Progress", data: { chunkLength: 10 } });
      }),
    });
    checkMock.mockResolvedValue(update);
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("available"));

    await act(async () => {
      const p = result.current.install();
      if (result.current.state.status === "downloading") seenTotal = result.current.state.total;
      await p;
    });

    expect(seenTotal ?? null).toBeNull();
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

  it("relaunch() delegates to the process plugin", async () => {
    checkMock.mockResolvedValue(null);
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("none"));

    await act(async () => { await result.current.relaunch(); });

    expect(relaunchMock).toHaveBeenCalledTimes(1);
  });
});
