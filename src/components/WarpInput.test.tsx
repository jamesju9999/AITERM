import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { LocaleProvider } from "../contexts/LocaleContext";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { WarpInput } from "./WarpInput";

beforeEach(() => {
  invokeMock.mockReset();
});

function renderInput(onSubmit = vi.fn()) {
  render(
    <LocaleProvider>
      <WarpInput onSubmit={onSubmit} sessionId="s1" />
    </LocaleProvider>,
  );
  return onSubmit;
}

describe("WarpInput — directory picker", () => {
  it("fetches and lists subfolders of the current directory when opened", async () => {
    invokeMock.mockResolvedValueOnce([
      { name: "src", path: "/proj/src", is_dir: true, size: null },
      { name: "README.md", path: "/proj/README.md", is_dir: false, size: 100 },
      { name: "docs", path: "/proj/docs", is_dir: true, size: null },
    ]);
    renderInput();

    const user = userEvent.setup();
    await user.click(screen.getByTitle("切換目錄"));

    await waitFor(() => {
      expect(screen.getByText("src")).toBeInTheDocument();
    });
    expect(screen.getByText("docs")).toBeInTheDocument();
    // Files (is_dir: false) must not appear as cd targets.
    expect(screen.queryByText("README.md")).not.toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("pty_list_dir", { id: "s1", path: "" });
  });

  it("submits `cd \"<name>\"` and closes the picker when a subfolder is clicked", async () => {
    invokeMock.mockResolvedValueOnce([
      { name: "src", path: "/proj/src", is_dir: true, size: null },
    ]);
    const onSubmit = renderInput();

    const user = userEvent.setup();
    await user.click(screen.getByTitle("切換目錄"));
    await waitFor(() => screen.getByText("src"));
    await user.click(screen.getByText("src"));

    expect(onSubmit).toHaveBeenCalledWith('cd "src"');
    expect(screen.queryByText("src")).not.toBeInTheDocument();
  });

  it("submits `cd ..` and closes the picker when the parent-dir entry is clicked", async () => {
    invokeMock.mockResolvedValueOnce([
      { name: "src", path: "/proj/src", is_dir: true, size: null },
    ]);
    const onSubmit = renderInput();

    const user = userEvent.setup();
    await user.click(screen.getByTitle("切換目錄"));
    await waitFor(() => screen.getByText("src")); // wait for the fetch to settle
    await user.click(screen.getByText(/\.\. \(/));

    expect(onSubmit).toHaveBeenCalledWith("cd ..");
    expect(screen.queryByText("src")).not.toBeInTheDocument();
  });

  it("shows the parent-dir entry immediately, before the subfolder fetch resolves", async () => {
    invokeMock.mockResolvedValueOnce([
      { name: "src", path: "/proj/src", is_dir: true, size: null },
    ]);
    renderInput();

    const user = userEvent.setup();
    await user.click(screen.getByTitle("切換目錄"));
    // No `await waitFor` here — the parent-dir entry must be present even
    // while the subfolder listing is still loading.
    expect(screen.getByText(/\.\. \(/)).toBeInTheDocument();
  });

  it("shows an empty state when the current directory has no subfolders", async () => {
    invokeMock.mockResolvedValueOnce([
      { name: "file.txt", path: "/proj/file.txt", is_dir: false, size: 10 },
    ]);
    renderInput();

    const user = userEvent.setup();
    await user.click(screen.getByTitle("切換目錄"));

    await waitFor(() => {
      expect(screen.getByText("此目錄下沒有子資料夾")).toBeInTheDocument();
    });
  });

  it("disables the picker button when no sessionId is available", () => {
    render(
      <LocaleProvider>
        <WarpInput onSubmit={vi.fn()} />
      </LocaleProvider>,
    );
    expect(screen.getByTitle("切換目錄")).toBeDisabled();
  });
});

describe("WarpInput — IME 組字", () => {
  it("組字中按 Enter（確認候選字）不送出指令", () => {
    const onSubmit = renderInput();
    const textarea = screen.getByRole("textbox");

    fireEvent.change(textarea, { target: { value: "中文" } });
    fireEvent.keyDown(textarea, { key: "Enter", isComposing: true });

    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("組字結束後按 Enter 正常送出", () => {
    const onSubmit = renderInput();
    const textarea = screen.getByRole("textbox");

    fireEvent.change(textarea, { target: { value: "中文" } });
    fireEvent.keyDown(textarea, { key: "Enter" });

    expect(onSubmit).toHaveBeenCalledWith("中文");
  });
});
