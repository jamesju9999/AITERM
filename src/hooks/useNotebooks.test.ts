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

const NB2 = {
  id: "nb-2", name: "More Docs", folder_path: "/tmp/more-docs",
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

    await waitFor(() => expect(result.current.syncProgressById["nb-1"]?.processed).toBe(3));

    invokeMock.mockResolvedValueOnce([NB1]); // refresh after sync
    await act(async () => {
      resolveSync({ indexed: 10, failed: 0, deleted: 0 });
      await syncPromise;
    });

    expect(result.current.syncingIds.has("nb-1")).toBe(false);
  });

  it("syncing two notebooks at once keeps their progress and completion independent", async () => {
    invokeMock.mockResolvedValueOnce([NB1, NB2]); // initial load
    const { result } = renderHook(() => useNotebooks());
    await waitFor(() => expect(result.current.notebooks).toEqual([NB1, NB2]));

    const callbacks: Record<string, (e: { payload: unknown }) => void> = {};
    listenMock.mockImplementation((_event: string, cb: (e: { payload: unknown }) => void) => {
      // Each sync() call registers its own listener; stash the latest one so
      // both notebooks' progress events can be delivered independently below.
      const pending = Object.keys(callbacks).length === 0 ? "nb-1" : "nb-2";
      callbacks[pending] = cb;
      return Promise.resolve(vi.fn());
    });

    const resolvers: Record<string, (v: { indexed: number; failed: number; deleted: number }) => void> = {};
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd !== "kb_sync_notebook") return Promise.resolve([NB1, NB2]); // refresh calls
      return new Promise((r) => {
        const id = Object.keys(resolvers).length === 0 ? "nb-1" : "nb-2";
        resolvers[id] = r;
      });
    });

    let syncA: Promise<unknown> = Promise.resolve();
    act(() => {
      syncA = result.current.sync("nb-1");
    });
    await waitFor(() => expect(callbacks["nb-1"]).toBeDefined());

    let syncB: Promise<unknown> = Promise.resolve();
    act(() => {
      syncB = result.current.sync("nb-2");
    });
    await waitFor(() => expect(callbacks["nb-2"]).toBeDefined());

    // Both notebooks should be tracked as syncing at once.
    expect(result.current.syncingIds.has("nb-1")).toBe(true);
    expect(result.current.syncingIds.has("nb-2")).toBe(true);

    act(() => {
      callbacks["nb-1"]({
        payload: { kind: "progress", notebook_id: "nb-1", processed: 1, total: 5, current_file: "a.pdf" },
      });
      callbacks["nb-2"]({
        payload: { kind: "progress", notebook_id: "nb-2", processed: 9, total: 20, current_file: "z.pdf" },
      });
    });

    // Each notebook's progress must reflect its own event, not the other's.
    await waitFor(() => {
      expect(result.current.syncProgressById["nb-1"]?.processed).toBe(1);
      expect(result.current.syncProgressById["nb-2"]?.processed).toBe(9);
    });

    // Finishing nb-1 first must not clear nb-2's still-in-progress state.
    await act(async () => {
      resolvers["nb-1"]({ indexed: 5, failed: 0, deleted: 0 });
      await syncA;
    });

    expect(result.current.syncingIds.has("nb-1")).toBe(false);
    expect(result.current.syncingIds.has("nb-2")).toBe(true);
    expect(result.current.syncProgressById["nb-2"]?.processed).toBe(9);

    await act(async () => {
      resolvers["nb-2"]({ indexed: 20, failed: 0, deleted: 0 });
      await syncB;
    });

    expect(result.current.syncingIds.has("nb-2")).toBe(false);
  });
});
