import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { useTeamFeatures } from "./useTeamFeatures";

const REPO_INFO = {
  vcs_type: "git" as const,
  root: "/tmp/repo",
  remote_url: "https://github.com/acme/widget.git",
  connection_id: "conn-1",
};

const FEATURE = {
  number: 7,
  title: "登入頁優化",
  author: "alice",
  draft: true,
  url: "https://github.com/acme/widget/pull/7",
  updated_at: "2026-08-17T00:00:00Z",
  head_ref: "feature/login-optimize",
  files: ["src/Login.tsx"],
};

beforeEach(() => {
  invokeMock.mockReset();
});

describe("useTeamFeatures", () => {
  it("loads active features on mount", async () => {
    invokeMock.mockResolvedValueOnce([FEATURE]);
    const { result } = renderHook(() => useTeamFeatures(REPO_INFO));

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.features).toEqual([FEATURE]);
    expect(invokeMock).toHaveBeenCalledWith("vcs_list_active_features", { repoInfo: REPO_INFO });
  });

  it("refresh() re-fetches the list", async () => {
    invokeMock.mockResolvedValueOnce([]);
    const { result } = renderHook(() => useTeamFeatures(REPO_INFO));
    await waitFor(() => expect(result.current.loading).toBe(false));

    invokeMock.mockResolvedValueOnce([FEATURE]);
    await act(async () => {
      await result.current.refresh();
    });

    expect(result.current.features).toEqual([FEATURE]);
  });

  it("surfaces an error message when the fetch fails", async () => {
    invokeMock.mockRejectedValueOnce("此功能需要 GitHub token");
    const { result } = renderHook(() => useTeamFeatures(REPO_INFO));

    await waitFor(() => expect(result.current.error).toBe("此功能需要 GitHub token"));
    expect(result.current.features).toEqual([]);
  });

  it("does nothing when repoInfo is null", async () => {
    const { result } = renderHook(() => useTeamFeatures(null));
    expect(result.current.loading).toBe(false);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("does not refetch when repoInfo has the same values but a new object identity", async () => {
    invokeMock.mockResolvedValueOnce([FEATURE]);
    const { result, rerender } = renderHook(
      ({ repo }) => useTeamFeatures(repo),
      { initialProps: { repo: REPO_INFO } },
    );
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(invokeMock).toHaveBeenCalledTimes(1);

    // Same values, different object identity.
    rerender({ repo: { ...REPO_INFO } });

    expect(invokeMock).toHaveBeenCalledTimes(1);
  });
});
