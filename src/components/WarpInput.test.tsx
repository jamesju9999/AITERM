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

describe("WarpInput — 指令執行中時，導覽鍵直接轉發給 PTY（onRawKey）", () => {
  // 實機抓到的 bug：遠端主控端是 Windows 時，claude CLI 的信任提示不會
  // 觸發 Kitty keyboard protocol 偵測（那個協定本身在 Windows 上就不會被
  // 送出，不是我們判斷錯），WarpInput 因此不會被藏起來、焦點留在這裡。
  // 上下鍵原本只會被這個框當成「瀏覽指令歷史」，Enter 只會被當成「送出
  // 目前輸入」——使用者想操作那個正在跑的互動選單時，一個位元組都送不到
  // PTY，選單完全沒反應。

  it("有指令在跑、輸入框是空的時，上下鍵/Enter/Esc 轉發 raw bytes，不會被歷史導覽或送出攔截", () => {
    const onSubmit = vi.fn();
    const onRawKey = vi.fn();
    render(
      <LocaleProvider>
        <WarpInput onSubmit={onSubmit} sessionId="s1" isCommandRunning onRawKey={onRawKey} />
      </LocaleProvider>,
    );
    const textarea = screen.getByRole("textbox");

    fireEvent.keyDown(textarea, { key: "ArrowDown" });
    fireEvent.keyDown(textarea, { key: "ArrowUp" });
    fireEvent.keyDown(textarea, { key: "Enter" });
    fireEvent.keyDown(textarea, { key: "Escape" });

    expect(onRawKey.mock.calls.map((c) => c[0])).toEqual(["\x1b[B", "\x1b[A", "\r", "\x1b"]);
    // 沒有任何一個鍵被當成「送出指令」或觸發歷史彈出視窗。
    expect(onSubmit).not.toHaveBeenCalled();
    expect(screen.queryByText(/warp_history_title|指令歷史/)).not.toBeInTheDocument();
  });

  it("沒有指令在跑時，上下鍵維持原本的歷史導覽行為，不轉發 raw bytes", () => {
    const onSubmit = vi.fn();
    const onRawKey = vi.fn();
    localStorage.setItem("aiterm-command-history", JSON.stringify(["echo hi"]));
    render(
      <LocaleProvider>
        <WarpInput onSubmit={onSubmit} sessionId="s1" isCommandRunning={false} onRawKey={onRawKey} />
      </LocaleProvider>,
    );
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;

    fireEvent.keyDown(textarea, { key: "ArrowUp" });

    expect(onRawKey).not.toHaveBeenCalled();
    expect(textarea.value).toBe("echo hi");
    localStorage.removeItem("aiterm-command-history");
  });

  it("輸入框裡已經有文字時（使用者正在打下一個指令），維持原本的送出/歷史行為，不轉發 raw bytes", () => {
    const onSubmit = vi.fn();
    const onRawKey = vi.fn();
    render(
      <LocaleProvider>
        <WarpInput onSubmit={onSubmit} sessionId="s1" isCommandRunning onRawKey={onRawKey} />
      </LocaleProvider>,
    );
    const textarea = screen.getByRole("textbox");

    fireEvent.change(textarea, { target: { value: "ls" } });
    fireEvent.keyDown(textarea, { key: "Enter" });

    expect(onRawKey).not.toHaveBeenCalled();
    expect(onSubmit).toHaveBeenCalledWith("ls");
  });
});
