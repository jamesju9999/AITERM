import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
const listenMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

import { useNotebooks } from "./useNotebooks";

const NB1 = {
  id: "nb-1", name: "Docs", folder_path: "/tmp/docs",
  embed_provider_id: "ollama-local", embed_model: "nomic-embed-text",
  embed_dim: null, last_synced_at: null, created_at: "2026-01-01",
};

beforeEach(() => {
  invokeMock.mockReset();
  listenMock.mockReset();
  listenMock.mockResolvedValue(vi.fn());
});

describe("useNotebooks", () => {
  it("loads notebooks on mount", async () => {
    invokeMock.mockResolvedValueOnce([NB1]);
    const { result } = renderHook(() => useNotebooks());

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.notebooks).toEqual([NB1]);
    expect(invokeMock).toHaveBeenCalledWith("kb_list_notebooks");
  });

  it("create() calls kb_create_notebook then refreshes the list", async () => {
    invokeMock.mockResolvedValueOnce([]); // initial load
    const { result } = renderHook(() => useNotebooks());
    await waitFor(() => expect(result.current.loading).toBe(false));

    invokeMock.mockResolvedValueOnce(NB1); // kb_create_notebook
    invokeMock.mockResolvedValueOnce([NB1]); // refresh after create

    await act(async () => {
      await result.current.create("Docs", "/tmp/docs", "ollama-local", "nomic-embed-text");
    });

    expect(result.current.notebooks).toEqual([NB1]);
  });

  it("remove() calls kb_delete_notebook then refreshes the list", async () => {
    invokeMock.mockResolvedValueOnce([NB1]); // initial load
    const { result } = renderHook(() => useNotebooks());
    await waitFor(() => expect(result.current.notebooks).toEqual([NB1]));

    invokeMock.mockResolvedValueOnce(undefined); // kb_delete_notebook
    invokeMock.mockResolvedValueOnce([]); // refresh after delete

    await act(async () => {
      await result.current.remove("nb-1");
    });

    expect(result.current.notebooks).toEqual([]);
  });

  it("sync() tracks progress events scoped to the syncing notebook", async () => {
    invokeMock.mockResolvedValueOnce([NB1]); // initial load
    const { result } = renderHook(() => useNotebooks());
    await waitFor(() => expect(result.current.notebooks).toEqual([NB1]));

    let capturedCallback: ((e: { payload: unknown }) => void) | null = null;
    listenMock.mockImplementationOnce((_event: string, cb: typeof capturedCallback) => {
      capturedCallback = cb;
      return Promise.resolve(vi.fn());
    });

    // Keep the sync invoke pending so we can observe progress before it resolves.
    let resolveSync: (v: { indexed: number; failed: number; deleted: number }) => void = () => {};
    invokeMock.mockImplementationOnce(
      () =>
        new Promise((r) => {
          resolveSync = r;
        }),
    );

    let syncPromise: Promise<unknown> = Promise.resolve();
    act(() => {
      syncPromise = result.current.sync("nb-1");
    });

    await waitFor(() => expect(capturedCallback).not.toBeNull());

    act(() => {
      capturedCallback?.({
        payload: { kind: "progress", notebook_id: "nb-1", processed: 3, total: 10, current_file: "a.pdf" },
      });
    });

    await waitFor(() => expect(result.current.syncProgress?.processed).toBe(3));

    invokeMock.mockResolvedValueOnce([NB1]); // refresh after sync
    await act(async () => {
      resolveSync({ indexed: 10, failed: 0, deleted: 0 });
      await syncPromise;
    });

    expect(result.current.syncingId).toBeNull();
  });
});
