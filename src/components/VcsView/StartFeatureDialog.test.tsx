import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { StartFeatureDialog } from "./StartFeatureDialog";
import { LocaleProvider } from "../../contexts/LocaleContext";

const checkOverlapMock = vi.fn();
const startFeatureMock = vi.fn();
vi.mock("../../ipc/vcs", () => ({
  vcsCheckOverlap: (...args: unknown[]) => checkOverlapMock(...args),
  vcsStartFeature: (...args: unknown[]) => startFeatureMock(...args),
}));

const REPO_INFO = {
  vcs_type: "git" as const,
  root: "/tmp/repo",
  remote_url: "https://github.com/acme/widget.git",
  connection_id: "conn-1",
};

beforeEach(() => {
  checkOverlapMock.mockReset();
  startFeatureMock.mockReset();
});

function renderDialog(onStarted = vi.fn(), onClose = vi.fn()) {
  render(
    <LocaleProvider>
      <StartFeatureDialog repoInfo={REPO_INFO} baseBranch="main" onStarted={onStarted} onClose={onClose} />
    </LocaleProvider>
  );
  return { onStarted, onClose };
}

describe("StartFeatureDialog", () => {
  it("starts the feature directly when there is no overlap", async () => {
    checkOverlapMock.mockResolvedValueOnce([]);
    startFeatureMock.mockResolvedValueOnce({
      branch_name: "feature/login-fix",
      pr_number: 7,
      pr_url: "https://github.com/acme/widget/pull/7",
    });
    const { onStarted } = renderDialog();

    fireEvent.change(screen.getByLabelText("功能名稱"), { target: { value: "Login Fix" } });
    fireEvent.click(screen.getByRole("button", { name: "開始新功能" }));

    await waitFor(() => expect(startFeatureMock).toHaveBeenCalledWith(
      REPO_INFO, "Login Fix", "main", [],
    ));
    await waitFor(() => expect(onStarted).toHaveBeenCalled());
  });

  it("shows a warning and waits for confirmation when files overlap", async () => {
    checkOverlapMock.mockResolvedValueOnce([
      { number: 3, title: "Bob 的功能", author: "bob", draft: true, url: "", updated_at: "", head_ref: "", files: ["src/App.tsx"] },
    ]);
    renderDialog();

    fireEvent.change(screen.getByLabelText("功能名稱"), { target: { value: "My Feature" } });
    fireEvent.change(screen.getByLabelText("預計會動到的檔案"), { target: { value: "src/App.tsx" } });
    fireEvent.click(screen.getByRole("button", { name: "開始新功能" }));

    await waitFor(() => expect(screen.getByText(/bob/)).toBeInTheDocument());
    expect(startFeatureMock).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "仍要開始" }));
    await waitFor(() => expect(startFeatureMock).toHaveBeenCalled());
  });

  it("skips the overlap check entirely when no files are declared", async () => {
    startFeatureMock.mockResolvedValueOnce({
      branch_name: "feature/x", pr_number: 1, pr_url: "",
    });
    renderDialog();

    fireEvent.change(screen.getByLabelText("功能名稱"), { target: { value: "X" } });
    fireEvent.click(screen.getByRole("button", { name: "開始新功能" }));

    await waitFor(() => expect(startFeatureMock).toHaveBeenCalled());
    expect(checkOverlapMock).not.toHaveBeenCalled();
  });
});
