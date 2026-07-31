import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";
import { usePythonEnvGate } from "./usePythonEnvGate";

const ensure = vi.fn();
const status = vi.fn();
const listeners: Array<(e: { payload: { level: string; message: string } }) => void> = [];

vi.mock("../../ipc/pythonEnv", () => ({
  pythonEnvEnsure: (p: string) => ensure(p),
  pythonEnvStatus: () => status(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (_name: string, cb: (e: { payload: { level: string; message: string } }) => void) => {
    listeners.push(cb);
    return Promise.resolve(() => {});
  },
}));

describe("usePythonEnvGate", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    listeners.length = 0;
  });

  it("reports ready and returns true when ensure succeeds", async () => {
    ensure.mockResolvedValue(undefined);
    const { result } = renderHook(() => usePythonEnvGate());

    let ok: boolean | undefined;
    await act(async () => {
      ok = await result.current.ensureProfile("doc_core");
    });

    expect(ok).toBe(true);
    expect(result.current.state).toBe("ready");
  });

  it("shows the guidance card when the failure is a missing Python, not a failed install", async () => {
    // Decided from status(), not by matching words in the error string.
    ensure.mockRejectedValue("無法取得 Python：network unreachable");
    status.mockResolvedValue({
      uvAvailable: true,
      pythonVersion: null,
      installed: [],
      venvPath: "/data/python-env",
      userInterpreter: null,
    });
    const { result } = renderHook(() => usePythonEnvGate());

    await act(async () => {
      await result.current.ensureProfile("doc_core");
    });

    expect(result.current.state).toBe("missing");
  });

  it("shows the broken-install card when uv itself is unusable, not the missing-Python card", async () => {
    // uvAvailable: false means the bundled uv binary is gone or unusable — no
    // "install it for me" button can fix that, so it must not collapse into
    // "missing" the way a plain missing-Python failure does.
    //
    // This combination is one the backend can actually produce: `status()`'s
    // uvAvailable checks the executable bit (not just that the file exists),
    // so a uv binary that lost its execute permission (a copy that didn't
    // preserve modes, a restrictive ACL) reports uvAvailable: false at the
    // same time ensure() fails to spawn it with a permission error.
    ensure.mockRejectedValue("uv 無法執行：permission denied");
    status.mockResolvedValue({
      uvAvailable: false,
      pythonVersion: null,
      installed: [],
      venvPath: "/data/python-env",
      userInterpreter: null,
    });
    const { result } = renderHook(() => usePythonEnvGate());

    await act(async () => {
      await result.current.ensureProfile("doc_core");
    });

    expect(result.current.state).toBe("broken");
  });

  it("shows a plain failure when Python exists but the install broke", async () => {
    ensure.mockRejectedValue("安裝 doc_core 相依套件失敗：…");
    status.mockResolvedValue({
      uvAvailable: true,
      pythonVersion: "3.12.13",
      installed: [],
      venvPath: "/data/python-env",
      userInterpreter: null,
    });
    const { result } = renderHook(() => usePythonEnvGate());

    await act(async () => {
      await result.current.ensureProfile("doc_core");
    });

    expect(result.current.state).toBe("failed");
    expect(result.current.error).toContain("相依套件失敗");
  });

  it("accumulates log lines from python-env-log and marks non-info lines as errors", async () => {
    ensure.mockResolvedValue(undefined);
    const { result } = renderHook(() => usePythonEnvGate());
    await waitFor(() => expect(listeners.length).toBe(1));

    act(() => {
      listeners[0]({ payload: { level: "info", message: "Resolved 36 packages" } });
      listeners[0]({ payload: { level: "warn", message: "could not fetch wheel" } });
    });

    expect(result.current.lines).toEqual([
      { text: "Resolved 36 packages", isError: false },
      { text: "could not fetch wheel", isError: true },
    ]);
  });
});
