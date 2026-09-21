import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent, cleanup } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { LocaleProvider } from "../contexts/LocaleContext";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { WarpInput } from "./WarpInput";

beforeEach(() => {
  invokeMock.mockReset();
  localStorage.clear();
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

describe("WarpInput — 歷史灰字建議", () => {
  const HISTORY_KEY = "aiterm-command-history";
  type Key = "tab" | "right" | "off" | undefined;

  function renderWith(
    suggestionKey: Key,
    opts: { history?: string[]; isCommandRunning?: boolean } = {},
  ) {
    localStorage.setItem(HISTORY_KEY, JSON.stringify(opts.history ?? ["git status", "git stash"]));
    const onSubmit = vi.fn();
    render(
      <LocaleProvider>
        <WarpInput
          onSubmit={onSubmit}
          sessionId="s1"
          suggestionKey={suggestionKey}
          isCommandRunning={opts.isCommandRunning}
        />
      </LocaleProvider>,
    );
    return { ta: screen.getByRole("textbox") as HTMLTextAreaElement, onSubmit };
  }
  const ghost = () => document.querySelector(".warp-input-ghost-suggestion");
  const typeText = (ta: HTMLTextAreaElement, value: string) =>
    fireEvent.change(ta, { target: { value } });
  // fireEvent 回傳 false 代表事件被 preventDefault
  const press = (ta: HTMLTextAreaElement, key: string) => fireEvent.keyDown(ta, { key });

  it("tab 模式：輸入開頭後，游標後面顯示最新一筆符合歷史的剩餘部分（灰字），輸入框的值不變", () => {
    const { ta } = renderWith("tab");
    typeText(ta, "git s");
    expect(ghost()?.textContent).toBe("tash"); // git stash 比 git status 新
    expect(ta.value).toBe("git s");
  });

  it("空輸入、沒有符合的歷史：沒有灰字", () => {
    const { ta } = renderWith("tab");
    expect(ghost()).toBeNull();
    typeText(ta, "docker");
    expect(ghost()).toBeNull();
  });

  it("off 模式與沒傳 suggestionKey：完全沒有灰字", () => {
    for (const key of ["off", undefined] as const) {
      const { ta } = renderWith(key);
      typeText(ta, "git s");
      expect(ghost()).toBeNull();
      cleanup();
    }
  });

  it("tab 模式：Tab 補上整段建議（被 preventDefault），而且不會送出", () => {
    const { ta, onSubmit } = renderWith("tab");
    typeText(ta, "git s");
    const notPrevented = press(ta, "Tab");
    expect(notPrevented).toBe(false);
    expect(ta.value).toBe("git stash");
    expect(onSubmit).not.toHaveBeenCalled();
    expect(ghost()).toBeNull(); // 補上後已與歷史相同，不再有剩餘部分
  });

  it("tab 模式：→ 不接受建議，維持原本的移動游標", () => {
    const { ta } = renderWith("tab");
    typeText(ta, "git s");
    expect(press(ta, "ArrowRight")).toBe(true);
    expect(ta.value).toBe("git s");
  });

  it("right 模式：游標在結尾按 → 補上建議（被 preventDefault）；Tab 不接受也不攔", () => {
    const { ta } = renderWith("right");
    typeText(ta, "git s");
    expect(press(ta, "Tab")).toBe(true);
    expect(ta.value).toBe("git s");
    expect(press(ta, "ArrowRight")).toBe(false);
    expect(ta.value).toBe("git stash");
  });

  it("right 模式：游標不在結尾時沒有灰字，→ 只是移動游標", () => {
    const { ta } = renderWith("right");
    typeText(ta, "git s");
    ta.setSelectionRange(2, 2);
    fireEvent.keyUp(ta, { key: "ArrowLeft" });
    expect(ghost()).toBeNull();
    expect(press(ta, "ArrowRight")).toBe(true);
    expect(ta.value).toBe("git s");
  });

  it("off 模式：Tab 與 → 都維持原本行為", () => {
    const { ta } = renderWith("off");
    typeText(ta, "git s");
    expect(press(ta, "Tab")).toBe(true);
    expect(press(ta, "ArrowRight")).toBe(true);
    expect(ta.value).toBe("git s");
  });

  it("tab 模式但沒有建議時：Tab 不被攔截（維持原本行為）", () => {
    const { ta } = renderWith("tab");
    typeText(ta, "docker");
    expect(press(ta, "Tab")).toBe(true);
  });

  it("帶 Shift／Ctrl／Meta／Alt 的 Tab 不接受建議", () => {
    const { ta } = renderWith("tab");
    typeText(ta, "git s");
    for (const mod of ["shiftKey", "ctrlKey", "metaKey", "altKey"] as const) {
      expect(fireEvent.keyDown(ta, { key: "Tab", [mod]: true })).toBe(true);
    }
    expect(ta.value).toBe("git s");
  });

  it("指令執行中：沒有灰字（這時輸入框的按鍵另有用途）", () => {
    const { ta } = renderWith("tab", { isCommandRunning: true });
    typeText(ta, "git s");
    expect(ghost()).toBeNull();
  });

  it("按 ↑ 開歷史清單後：沒有灰字", () => {
    // 較新的 "git stash" 之外還有較舊、更長的符合者——若沒把「清單開啟」納入條件，
    // 填入 "git stash" 之後灰字會殘留成 " --all"。
    const { ta } = renderWith("tab", { history: ["git stash --all", "git stash"] });
    press(ta, "ArrowUp");
    expect(ta.value).toBe("git stash");
    expect(ghost()).toBeNull();
  });

  it("目錄選單開啟時：沒有灰字", async () => {
    invokeMock.mockResolvedValueOnce([]);
    const { ta } = renderWith("tab");
    typeText(ta, "git s");
    expect(ghost()).not.toBeNull();
    await userEvent.setup().click(screen.getByTitle("切換目錄"));
    await waitFor(() => expect(screen.getByText(/\.\. \(/)).toBeInTheDocument());
    expect(ghost()).toBeNull();
  });

  it("輸入法組字中：沒有灰字，組字結束後恢復", () => {
    const { ta } = renderWith("tab");
    typeText(ta, "git s");
    expect(ghost()).not.toBeNull();
    fireEvent.compositionStart(ta);
    expect(ghost()).toBeNull();
    fireEvent.compositionEnd(ta);
    expect(ghost()?.textContent).toBe("tash");
  });

  it("按 Enter 送出的是輸入的原文，不含灰字", () => {
    const { ta, onSubmit } = renderWith("tab");
    typeText(ta, "git s");
    press(ta, "Enter");
    expect(onSubmit).toHaveBeenCalledWith("git s");
    expect(ghost()).toBeNull();
  });
});
