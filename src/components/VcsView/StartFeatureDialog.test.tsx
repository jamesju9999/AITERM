import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { StartFeatureDialog } from "./StartFeatureDialog";
import { LocaleProvider } from "../../contexts/LocaleContext";

const checkOverlapMock = vi.fn();
const startFeatureMock = vi.fn();
const getDefaultBranchMock = vi.fn();
vi.mock("../../ipc/vcs", () => ({
  vcsCheckOverlap: (...args: unknown[]) => checkOverlapMock(...args),
  vcsStartFeature: (...args: unknown[]) => startFeatureMock(...args),
  vcsGetDefaultBranch: (...args: unknown[]) => getDefaultBranchMock(...args),
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
  getDefaultBranchMock.mockReset();
  getDefaultBranchMock.mockResolvedValue("main");
});

function renderDialog(onStarted = vi.fn(), onClose = vi.fn()) {
  render(
    <LocaleProvider>
      <StartFeatureDialog repoInfo={REPO_INFO} onStarted={onStarted} onClose={onClose} />
    </LocaleProvider>
  );
  return { onStarted, onClose };
}

describe("StartFeatureDialog", () => {
  it("fetches the repo's default branch via IPC instead of assuming one", async () => {
    renderDialog();
    await waitFor(() => expect(getDefaultBranchMock).toHaveBeenCalledWith(REPO_INFO));
  });

  it("starts the feature directly when there is no overlap", async () => {
    checkOverlapMock.mockResolvedValueOnce([]);
    startFeatureMock.mockResolvedValueOnce({
      branch_name: "feature/login-fix",
      pr_number: 7,
      pr_url: "https://github.com/acme/widget/pull/7",
    });
    const { onStarted } = renderDialog();

    await waitFor(() => expect(getDefaultBranchMock).toHaveBeenCalled());
    fireEvent.change(screen.getByLabelText("功能名稱"), { target: { value: "Login Fix" } });
    fireEvent.click(screen.getByRole("button", { name: "開始新功能" }));

    await waitFor(() => expect(startFeatureMock).toHaveBeenCalledWith(
      REPO_INFO, "Login Fix", "main", [],
    ));
    await waitFor(() => expect(onStarted).toHaveBeenCalledWith("Login Fix"));
  });

  it("shows a warning and waits for confirmation when files overlap", async () => {
    checkOverlapMock.mockResolvedValueOnce([
      { number: 3, title: "Bob 的功能", author: "bob", draft: true, url: "", updated_at: "", head_ref: "", base_ref: "main", files: ["src/App.tsx"] },
    ]);
    renderDialog();

    await waitFor(() => expect(getDefaultBranchMock).toHaveBeenCalled());
    fireEvent.change(screen.getByLabelText("功能名稱"), { target: { value: "My Feature" } });
    fireEvent.change(screen.getByLabelText("預計會動到的檔案"), { target: { value: "src/App.tsx" } });
    fireEvent.click(screen.getByRole("button", { name: "開始新功能" }));

    await waitFor(() => expect(screen.getByText(/bob/)).toBeInTheDocument());
    expect(startFeatureMock).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "仍要開始" }));
    await waitFor(() => expect(startFeatureMock).toHaveBeenCalled());
  });

  it("clears the overlap warning and re-checks when the file list is edited afterward", async () => {
    checkOverlapMock.mockResolvedValueOnce([
      { number: 3, title: "Bob 的功能", author: "bob", draft: true, url: "", updated_at: "", head_ref: "", base_ref: "main", files: ["src/App.tsx"] },
    ]);
    renderDialog();

    await waitFor(() => expect(getDefaultBranchMock).toHaveBeenCalled());
    fireEvent.change(screen.getByLabelText("功能名稱"), { target: { value: "My Feature" } });
    fireEvent.change(screen.getByLabelText("預計會動到的檔案"), { target: { value: "src/App.tsx" } });
    fireEvent.click(screen.getByRole("button", { name: "開始新功能" }));
    await waitFor(() => expect(screen.getByText(/bob/)).toBeInTheDocument());

    // Editing the file list after the warning should clear it, not leave "Start Anyway" as the only path.
    fireEvent.change(screen.getByLabelText("預計會動到的檔案"), { target: { value: "src/Other.tsx" } });
    expect(screen.queryByText(/bob/)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "開始新功能" })).toBeInTheDocument();

    // Submitting again should re-check overlap for the NEW file list, not skip straight to starting.
    checkOverlapMock.mockResolvedValueOnce([]);
    fireEvent.click(screen.getByRole("button", { name: "開始新功能" }));
    await waitFor(() => expect(checkOverlapMock).toHaveBeenLastCalledWith(REPO_INFO, ["src/Other.tsx"]));
  });

  it("skips the overlap check entirely when no files are declared", async () => {
    startFeatureMock.mockResolvedValueOnce({
      branch_name: "feature/x", pr_number: 1, pr_url: "",
    });
    renderDialog();

    await waitFor(() => expect(getDefaultBranchMock).toHaveBeenCalled());
    fireEvent.change(screen.getByLabelText("功能名稱"), { target: { value: "X" } });
    fireEvent.click(screen.getByRole("button", { name: "開始新功能" }));

    await waitFor(() => expect(startFeatureMock).toHaveBeenCalled());
    expect(checkOverlapMock).not.toHaveBeenCalled();
  });

  it("disables the submit button and shows a hint while the default branch is still loading", async () => {
    let resolveDefaultBranch: (branch: string) => void = () => {};
    getDefaultBranchMock.mockReset();
    getDefaultBranchMock.mockReturnValueOnce(new Promise<string>((resolve) => {
      resolveDefaultBranch = resolve;
    }));
    renderDialog();

    fireEvent.change(screen.getByLabelText("功能名稱"), { target: { value: "Login Fix" } });
    expect(screen.getByRole("button", { name: "開始新功能" })).toBeDisabled();
    expect(screen.getByText("偵測預設分支中…")).toBeInTheDocument();

    resolveDefaultBranch("master");
    await waitFor(() => expect(screen.getByRole("button", { name: "開始新功能" })).not.toBeDisabled());
  });

  it("shows an error and blocks submission when fetching the default branch fails", async () => {
    getDefaultBranchMock.mockReset();
    getDefaultBranchMock.mockRejectedValueOnce("GitHub API error 404: Not Found");
    renderDialog();

    await waitFor(() => expect(screen.getByText(/Not Found/)).toBeInTheDocument());
    fireEvent.change(screen.getByLabelText("功能名稱"), { target: { value: "Login Fix" } });
    expect(screen.getByRole("button", { name: "開始新功能" })).toBeDisabled();
  });
});
