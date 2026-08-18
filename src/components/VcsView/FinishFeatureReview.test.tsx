import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { FinishFeatureReview } from "./FinishFeatureReview";
import { LocaleProvider } from "../../contexts/LocaleContext";
import type { ActiveFeature } from "../../ipc/vcs";

const getDiffMock = vi.fn();
const mergeMock = vi.fn();
vi.mock("../../ipc/vcs", () => ({
  vcsGetFeatureDiff: (...args: unknown[]) => getDiffMock(...args),
  vcsMergeFeature: (...args: unknown[]) => mergeMock(...args),
}));

const FEATURE: ActiveFeature = {
  number: 7,
  title: "登入頁優化",
  author: "alice",
  draft: false,
  url: "https://github.com/acme/widget/pull/7",
  updated_at: "2026-08-17T00:00:00Z",
  head_ref: "feature/login-optimize",
  files: ["src/Login.tsx"],
};

const REPO_INFO = {
  vcs_type: "git" as const,
  root: "/tmp/repo",
  remote_url: "https://github.com/acme/widget.git",
  connection_id: "conn-1",
};

beforeEach(() => {
  getDiffMock.mockReset();
  mergeMock.mockReset();
});

describe("FinishFeatureReview", () => {
  it("loads and shows the diff on mount", async () => {
    getDiffMock.mockResolvedValueOnce("diff --git a/x b/x\n+hello\n");
    render(
      <LocaleProvider>
        <FinishFeatureReview repoInfo={REPO_INFO} feature={FEATURE} baseBranch="main" onMerged={vi.fn()} onClose={vi.fn()} />
      </LocaleProvider>
    );

    await waitFor(() => expect(screen.getByText(/\+hello/)).toBeInTheDocument());
    expect(getDiffMock).toHaveBeenCalledWith(REPO_INFO, "main", "feature/login-optimize");
  });

  it("merges and deletes the branch by default (checkbox starts checked)", async () => {
    getDiffMock.mockResolvedValueOnce("diff content");
    mergeMock.mockResolvedValueOnce(undefined);
    const onMerged = vi.fn();
    render(
      <LocaleProvider>
        <FinishFeatureReview repoInfo={REPO_INFO} feature={FEATURE} baseBranch="main" onMerged={onMerged} onClose={vi.fn()} />
      </LocaleProvider>
    );

    await waitFor(() => expect(screen.getByText("diff content")).toBeInTheDocument());
    expect(screen.getByLabelText("合併後刪除這個功能分支")).toBeChecked();
    fireEvent.click(screen.getByRole("button", { name: "合併" }));

    await waitFor(() => expect(mergeMock).toHaveBeenCalledWith(REPO_INFO, 7, "feature/login-optimize"));
    await waitFor(() => expect(onMerged).toHaveBeenCalled());
  });

  it("does not delete the branch when the user unchecks the box first", async () => {
    getDiffMock.mockResolvedValueOnce("diff content");
    mergeMock.mockResolvedValueOnce(undefined);
    render(
      <LocaleProvider>
        <FinishFeatureReview repoInfo={REPO_INFO} feature={FEATURE} baseBranch="main" onMerged={vi.fn()} onClose={vi.fn()} />
      </LocaleProvider>
    );

    await waitFor(() => expect(screen.getByText("diff content")).toBeInTheDocument());
    fireEvent.click(screen.getByLabelText("合併後刪除這個功能分支"));
    fireEvent.click(screen.getByRole("button", { name: "合併" }));

    await waitFor(() => expect(mergeMock).toHaveBeenCalledWith(REPO_INFO, 7, null));
  });

  it("shows a clear conflict message when merge fails", async () => {
    getDiffMock.mockResolvedValueOnce("diff content");
    mergeMock.mockRejectedValueOnce("GitHub API error 405: Pull Request is not mergeable");
    render(
      <LocaleProvider>
        <FinishFeatureReview repoInfo={REPO_INFO} feature={FEATURE} baseBranch="main" onMerged={vi.fn()} onClose={vi.fn()} />
      </LocaleProvider>
    );

    await waitFor(() => expect(screen.getByText("diff content")).toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: "合併" }));

    await waitFor(() => expect(screen.getByText(/not mergeable/)).toBeInTheDocument());
  });
});
