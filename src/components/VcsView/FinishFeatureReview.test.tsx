import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { FinishFeatureReview } from "./FinishFeatureReview";
import { LocaleProvider } from "../../contexts/LocaleContext";
import type { ActiveFeature } from "../../ipc/vcs";

const getDiffMock = vi.fn();
const mergeMock = vi.fn();
const finishMock = vi.fn();
vi.mock("../../ipc/vcs", () => ({
  vcsGetFeatureDiff: (...args: unknown[]) => getDiffMock(...args),
  vcsMergeFeature: (...args: unknown[]) => mergeMock(...args),
  vcsFinishFeature: (...args: unknown[]) => finishMock(...args),
}));

const FEATURE: ActiveFeature = {
  number: 7,
  title: "登入頁優化",
  author: "alice",
  draft: false,
  url: "https://github.com/acme/widget/pull/7",
  updated_at: "2026-08-17T00:00:00Z",
  head_ref: "feature/login-optimize",
  base_ref: "main",
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
  finishMock.mockReset();
  finishMock.mockResolvedValue(undefined);
});

describe("FinishFeatureReview", () => {
  it("loads the diff on mount regardless of draft status, without ever calling vcsFinishFeature automatically", async () => {
    const draftFeature = { ...FEATURE, draft: true };
    getDiffMock.mockResolvedValueOnce("diff content");
    render(
      <LocaleProvider>
        <FinishFeatureReview repoInfo={REPO_INFO} feature={draftFeature} onSubmittedForReview={vi.fn()} onMerged={vi.fn()} onClose={vi.fn()} />
      </LocaleProvider>
    );

    await waitFor(() => expect(screen.getByText("diff content")).toBeInTheDocument());
    expect(getDiffMock).toHaveBeenCalledWith(REPO_INFO, "main", "feature/login-optimize");
    expect(finishMock).not.toHaveBeenCalled();
  });

  it("shows a submit-for-review button for a draft feature; clicking it calls vcsFinishFeature and then switches to the merge button", async () => {
    const draftFeature = { ...FEATURE, draft: true };
    getDiffMock.mockResolvedValueOnce("diff content");
    finishMock.mockResolvedValueOnce(undefined);
    const onSubmittedForReview = vi.fn();
    render(
      <LocaleProvider>
        <FinishFeatureReview repoInfo={REPO_INFO} feature={draftFeature} onSubmittedForReview={onSubmittedForReview} onMerged={vi.fn()} onClose={vi.fn()} />
      </LocaleProvider>
    );

    await waitFor(() => expect(screen.getByText("diff content")).toBeInTheDocument());
    expect(screen.getByRole("button", { name: "送出審核" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "合併" })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "送出審核" }));

    await waitFor(() => expect(finishMock).toHaveBeenCalledWith(REPO_INFO, 7));
    await waitFor(() => expect(onSubmittedForReview).toHaveBeenCalled());
    await waitFor(() => expect(screen.getByRole("button", { name: "合併" })).toBeInTheDocument());
    expect(screen.queryByRole("button", { name: "送出審核" })).not.toBeInTheDocument();
  });

  it("shows the merge button directly for a non-draft feature, without ever calling vcsFinishFeature", async () => {
    getDiffMock.mockResolvedValueOnce("diff content");
    render(
      <LocaleProvider>
        <FinishFeatureReview repoInfo={REPO_INFO} feature={FEATURE} onSubmittedForReview={vi.fn()} onMerged={vi.fn()} onClose={vi.fn()} />
      </LocaleProvider>
    );

    await waitFor(() => expect(screen.getByText("diff content")).toBeInTheDocument());
    expect(screen.getByRole("button", { name: "合併" })).toBeInTheDocument();
    expect(finishMock).not.toHaveBeenCalled();
  });

  it("shows an error and lets the user retry when submitting for review fails", async () => {
    const draftFeature = { ...FEATURE, draft: true };
    getDiffMock.mockResolvedValueOnce("diff content");
    finishMock.mockRejectedValueOnce("此連線為唯讀模式，無法送審");
    render(
      <LocaleProvider>
        <FinishFeatureReview repoInfo={REPO_INFO} feature={draftFeature} onSubmittedForReview={vi.fn()} onMerged={vi.fn()} onClose={vi.fn()} />
      </LocaleProvider>
    );

    await waitFor(() => expect(screen.getByText("diff content")).toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: "送出審核" }));

    await waitFor(() => expect(screen.getByText(/唯讀模式/)).toBeInTheDocument());
    // Still draft (mark-ready failed), button still present and clickable again — not stuck busy.
    const retryButton = screen.getByRole("button", { name: "送出審核" });
    expect(retryButton).not.toBeDisabled();
  });

  it("merges and deletes the branch by default (checkbox starts checked)", async () => {
    getDiffMock.mockResolvedValueOnce("diff content");
    mergeMock.mockResolvedValueOnce(undefined);
    const onMerged = vi.fn();
    render(
      <LocaleProvider>
        <FinishFeatureReview repoInfo={REPO_INFO} feature={FEATURE} onSubmittedForReview={vi.fn()} onMerged={onMerged} onClose={vi.fn()} />
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
        <FinishFeatureReview repoInfo={REPO_INFO} feature={FEATURE} onSubmittedForReview={vi.fn()} onMerged={vi.fn()} onClose={vi.fn()} />
      </LocaleProvider>
    );

    await waitFor(() => expect(screen.getByText("diff content")).toBeInTheDocument());
    fireEvent.click(screen.getByLabelText("合併後刪除這個功能分支"));
    fireEvent.click(screen.getByRole("button", { name: "合併" }));

    await waitFor(() => expect(mergeMock).toHaveBeenCalledWith(REPO_INFO, 7, null));
  });

  it("shows an error and keeps the merge button disabled when the diff fails to load", async () => {
    getDiffMock.mockRejectedValueOnce("GitHub API error 404: Not Found");
    render(
      <LocaleProvider>
        <FinishFeatureReview repoInfo={REPO_INFO} feature={FEATURE} onSubmittedForReview={vi.fn()} onMerged={vi.fn()} onClose={vi.fn()} />
      </LocaleProvider>
    );

    await waitFor(() => expect(screen.getByText(/Not Found/)).toBeInTheDocument());
    expect(screen.getByRole("button", { name: "合併" })).toBeDisabled();
  });

  it("shows a clear conflict message when merge fails", async () => {
    getDiffMock.mockResolvedValueOnce("diff content");
    mergeMock.mockRejectedValueOnce("GitHub API error 405: Pull Request is not mergeable");
    render(
      <LocaleProvider>
        <FinishFeatureReview repoInfo={REPO_INFO} feature={FEATURE} onSubmittedForReview={vi.fn()} onMerged={vi.fn()} onClose={vi.fn()} />
      </LocaleProvider>
    );

    await waitFor(() => expect(screen.getByText("diff content")).toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: "合併" }));

    await waitFor(() => expect(screen.getByText(/not mergeable/)).toBeInTheDocument());
  });
});
