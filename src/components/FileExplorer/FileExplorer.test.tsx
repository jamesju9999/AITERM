import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { FileExplorer } from "./FileExplorer";

beforeEach(() => {
  invokeMock.mockReset();
});

/** Results returned for commands a test does not explicitly handle. */
const DEFAULT_RESULTS: Record<string, unknown> = {
  pty_get_cwd: null,
  pty_list_dir: [],
  list_drives: [],
};

/**
 * Dispatch invoke() by command name and arguments instead of by call order.
 *
 * These tests used to chain mockResolvedValueOnce in the exact sequence the
 * component happens to call things. That made every test hostage to that
 * sequence: adding one new call on mount shifts every queued value by one and
 * breaks the whole file at once, with failures that look like feature bugs.
 * Keying on the command name means a new call only affects the tests that
 * actually care about it.
 */
function mockCommands(
  handlers: Record<string, (args: Record<string, unknown>) => unknown> = {},
) {
  invokeMock.mockImplementation((cmd: string, args: Record<string, unknown> = {}) => {
    const handler = handlers[cmd];
    // Real Tauri invoke() always returns a Promise and never throws
    // synchronously. Wrapping the handler call itself in the Promise (rather
    // than passing its already-evaluated result to Promise.resolve) keeps a
    // handler that throws synchronously — as list_drives does in the
    // "re-reading fails" test — behaving like a rejected promise instead of an
    // uncaught synchronous exception.
    if (handler) return Promise.resolve().then(() => handler(args));
    return Promise.resolve(cmd in DEFAULT_RESULTS ? DEFAULT_RESULTS[cmd] : null);
  });
}

describe("FileExplorer — file selection", () => {
  it("shows empty viewer state initially", async () => {
    mockCommands();
    render(<FileExplorer sessionId="s1" />);
    await waitFor(() =>
      expect(screen.getByText(/選擇左側檔案以預覽內容/)).toBeInTheDocument()
    );
  });

  it("clicking a file loads its content in the viewer", async () => {
    mockCommands({
      pty_get_cwd: () => "/p",
      pty_list_dir: () => [{ name: "index.ts", path: "/p/index.ts", is_dir: false, size: 20 }],
      pty_read_file: () => ({ content: "export default 1;", truncated: false }),
    });

    render(<FileExplorer sessionId="s1" />);
    await waitFor(() => screen.getByText("index.ts"));

    await userEvent.click(screen.getByText("index.ts"));

    await waitFor(() =>
      expect(screen.getByText(/export default 1;/)).toBeInTheDocument()
    );
  });

  it("clicking a directory does NOT load file content", async () => {
    mockCommands({
      pty_get_cwd: () => "/p",
      pty_list_dir: ({ path }) =>
        path === "/p/src" ? [] : [{ name: "src", path: "/p/src", is_dir: true, size: null }],
    });

    render(<FileExplorer sessionId="s1" />);
    await waitFor(() => screen.getByText("src"));

    await userEvent.click(screen.getByText("src"));

    expect(invokeMock).not.toHaveBeenCalledWith("pty_read_file", expect.anything());
    expect(screen.getByText(/選擇左側檔案以預覽內容/)).toBeInTheDocument();
  });
});

describe("FileExplorer — hidden files toggle", () => {
  // This control had no coverage at all while its label was a literal ".",
  // which is also roughly how visible it was on screen.
  //
  // Both tests query by accessible name. Note what that does *not* buy: the
  // button carries both `aria-label` and `title`, and `title` is itself a
  // fallback source for the accessible name, so deleting the aria-label leaves
  // these tests green (checked, not assumed). They pin the behaviour and the
  // exposed state — not which attribute supplies the name.
  it("hides dotfiles until asked, then shows them", async () => {
    mockCommands({
      pty_get_cwd: () => "/p",
      pty_list_dir: () => [
        { name: ".zshrc", path: "/p/.zshrc", is_dir: false, size: 10 },
        { name: "notes.md", path: "/p/notes.md", is_dir: false, size: 10 },
      ],
    });

    render(<FileExplorer sessionId="s1" />);
    await waitFor(() => screen.getByText("notes.md"));
    expect(screen.queryByText(".zshrc")).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "顯示/隱藏隱藏檔案" }));

    await waitFor(() => expect(screen.getByText(".zshrc")).toBeInTheDocument());
  });

  it("reports whether it is on, since the icon alone does not change", async () => {
    // The icon is the same whichever way the toggle sits — the on-state is
    // carried by styling, which assistive technology cannot see. aria-pressed
    // is what makes the state available to something other than an eye.
    mockCommands({ pty_get_cwd: () => "/p" });
    render(<FileExplorer sessionId="s1" />);

    const toggle = await screen.findByRole("button", { name: "顯示/隱藏隱藏檔案" });
    expect(toggle).toHaveAttribute("aria-pressed", "false");

    await userEvent.click(toggle);

    expect(toggle).toHaveAttribute("aria-pressed", "true");
  });
});

describe("FileExplorer — switch terminal here", () => {
  it("does not render the button when onSwitchTerminalHere is not provided", async () => {
    mockCommands({ pty_get_cwd: () => "/p" });
    render(<FileExplorer sessionId="s1" />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    expect(screen.queryByTitle("切換終端機到此目錄")).not.toBeInTheDocument();
  });

  it("calls onSwitchTerminalHere with the currently-browsed directory when clicked", async () => {
    mockCommands({ pty_get_cwd: () => "/Users/jamesju/Downloads" });

    const onSwitch = vi.fn();
    render(<FileExplorer sessionId="s1" onSwitchTerminalHere={onSwitch} />);

    const button = await screen.findByTitle("切換終端機到此目錄");
    expect(button).not.toBeDisabled();
    await userEvent.click(button);

    expect(onSwitch).toHaveBeenCalledWith("/Users/jamesju/Downloads");
  });

  it("follows the last-clicked tree folder without navigating away from the root listing", async () => {
    const ROOT = "/Users/jamesju/Downloads";
    const CHILD = `${ROOT}/08_軟體安裝檔`;
    mockCommands({
      pty_get_cwd: () => ROOT,
      pty_list_dir: ({ path }) =>
        path === CHILD
          ? [{ name: "desktop.ini", path: `${CHILD}/desktop.ini`, is_dir: false, size: 282 }]
          : [{ name: "08_軟體安裝檔", path: CHILD, is_dir: true, size: null }],
    });

    const onSwitch = vi.fn();
    render(<FileExplorer sessionId="s1" onSwitchTerminalHere={onSwitch} />);

    await waitFor(() => screen.getByText("08_軟體安裝檔"));
    // Root listing is still what's shown — expanding is inline, not a navigation.
    expect(screen.getByText("Downloads")).toBeInTheDocument();

    await userEvent.click(screen.getByText("08_軟體安裝檔"));
    await waitFor(() => screen.getByText("desktop.ini")); // tree expanded inline

    // Breadcrumb now shows the expanded folder too — it and the tree row
    // both render the same text, so there should be exactly two matches.
    expect(screen.getAllByText("08_軟體安裝檔")).toHaveLength(2);
    // Root-level sibling context is still visible — this was an in-place
    // expand, not a full navigation away from the Downloads listing.
    expect(screen.getByText("desktop.ini")).toBeInTheDocument();

    await userEvent.click(screen.getByTitle("切換終端機到此目錄"));
    expect(onSwitch).toHaveBeenCalledWith("/Users/jamesju/Downloads/08_軟體安裝檔");
  });
});

describe("FileExplorer — drive switcher", () => {
  it("does not render the control when there are no drives (non-Windows)", async () => {
    mockCommands({ pty_get_cwd: () => "/p" });
    render(<FileExplorer sessionId="s1" />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByTitle("切換磁碟機")).not.toBeInTheDocument();
  });

  it("does not render the control when there is only one drive", async () => {
    // Nothing to switch to, and the breadcrumb already shows C:.
    mockCommands({
      pty_get_cwd: () => "C:/Users",
      list_drives: () => [{ path: "C:/", kind: "fixed" }],
    });
    render(<FileExplorer sessionId="s1" />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByTitle("切換磁碟機")).not.toBeInTheDocument();
  });

  it("shows the drive taken from the current path", async () => {
    mockCommands({
      pty_get_cwd: () => "D:/work",
      list_drives: () => [{ path: "C:/", kind: "fixed" }, { path: "D:/", kind: "fixed" }],
    });
    render(<FileExplorer sessionId="s1" />);

    const button = await screen.findByTitle("切換磁碟機");
    expect(button).toHaveTextContent("D:");
  });

  it("shows a neutral marker when the path carries no drive letter", async () => {
    // A UNC path (\\server\share) normalises to //server/share and matches no
    // drive letter. Claiming "C:" there would be worse than admitting none.
    mockCommands({
      pty_get_cwd: () => "//server/share/x",
      list_drives: () => [{ path: "C:/", kind: "fixed" }, { path: "D:/", kind: "fixed" }],
    });
    render(<FileExplorer sessionId="s1" />);

    const button = await screen.findByTitle("切換磁碟機");
    expect(button).toHaveTextContent("—");
    expect(button).not.toHaveTextContent("C:");
  });

  it("normalises a lowercase drive letter to match the menu", async () => {
    // `cd d:\work` keeps the argument verbatim, so cwd can be lowercase.
    mockCommands({
      pty_get_cwd: () => "d:/work",
      list_drives: () => [{ path: "C:/", kind: "fixed" }, { path: "D:/", kind: "fixed" }],
    });
    render(<FileExplorer sessionId="s1" />);

    expect(await screen.findByTitle("切換磁碟機")).toHaveTextContent("D:");
  });

  it("navigates to the drive root when one is picked", async () => {
    mockCommands({
      pty_get_cwd: () => "C:/Users",
      list_drives: () => [{ path: "C:/", kind: "fixed" }, { path: "D:/", kind: "fixed" }],
    });
    render(<FileExplorer sessionId="s1" />);

    await userEvent.click(await screen.findByTitle("切換磁碟機"));
    await userEvent.click(await screen.findByRole("button", { name: "D:" }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("pty_list_dir", { id: "s1", path: "D:/" })
    );
  });

  it("re-reads the drive list each time the menu opens", async () => {
    // A USB stick plugged in after launch has to show up.
    mockCommands({
      pty_get_cwd: () => "C:/Users",
      list_drives: () => [{ path: "C:/", kind: "fixed" }, { path: "D:/", kind: "fixed" }],
    });
    render(<FileExplorer sessionId="s1" />);

    const button = await screen.findByTitle("切換磁碟機");
    const afterMount = invokeMock.mock.calls.filter((c) => c[0] === "list_drives").length;

    await userEvent.click(button);

    await waitFor(() =>
      expect(invokeMock.mock.calls.filter((c) => c[0] === "list_drives").length)
        .toBeGreaterThan(afterMount)
    );
  });

  it("keeps the previous list when re-reading fails", async () => {
    // The user has the menu open; blanking it is worse than showing a list that
    // may be a moment stale.
    let firstCall = true;
    mockCommands({
      pty_get_cwd: () => "C:/Users",
      list_drives: () => {
        if (firstCall) { firstCall = false; return [{ path: "C:/", kind: "fixed" }, { path: "D:/", kind: "fixed" }]; }
        throw new Error("drive enumeration failed");
      },
    });
    render(<FileExplorer sessionId="s1" />);

    await userEvent.click(await screen.findByTitle("切換磁碟機"));

    expect(await screen.findByRole("button", { name: "D:" })).toBeInTheDocument();
  });

  it("keeps following the terminal after a drive is picked", async () => {
    // The invariant this whole design is shaped around: picking a drive is user
    // navigation, so it must not touch ptyCwdRef. If it did, the poll would
    // believe the terminal had already moved and would stop following it.
    vi.useFakeTimers();
    try {
      let terminalCwd = "C:/Users";
      mockCommands({
        pty_get_cwd: () => terminalCwd,
        list_drives: () => [{ path: "C:/", kind: "fixed" }, { path: "D:/", kind: "fixed" }],
      });
      render(<FileExplorer sessionId="s1" />);

      await vi.waitFor(() => screen.getByTitle("切換磁碟機"));
      // userEvent.click() hangs indefinitely here even with advanceTimers +
      // delay: null passed to userEvent.setup() — its internal pointer-event
      // sequencing never settles under these fake timers. fireEvent.click()
      // dispatches the same DOM click event without that machinery.
      fireEvent.click(screen.getByTitle("切換磁碟機"));
      await vi.waitFor(() => screen.getByRole("button", { name: "D:" }));
      fireEvent.click(screen.getByRole("button", { name: "D:" }));
      await vi.waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith("pty_list_dir", { id: "s1", path: "D:/" })
      );

      // The terminal itself has NOT moved (it is still where it was before the
      // drive pick). If selectDrive had updated ptyCwdRef to "D:/", the poll
      // would wrongly see a mismatch against the terminal's real, unchanged
      // "C:/Users" and revert the user's manual D: browsing right back to it.
      await vi.advanceTimersByTimeAsync(1600);
      expect(invokeMock).not.toHaveBeenCalledWith(
        "pty_list_dir", { id: "s1", path: "C:/Users" }
      );

      // Now the terminal itself moves somewhere new — the panel must still
      // pick that up.
      terminalCwd = "C:/Windows";
      await vi.advanceTimersByTimeAsync(1600);

      await vi.waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith("pty_list_dir", { id: "s1", path: "C:/Windows" })
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it("closes the menu when the backdrop is clicked", async () => {
    mockCommands({
      pty_get_cwd: () => "C:/Users",
      list_drives: () => [{ path: "C:/", kind: "fixed" }, { path: "D:/", kind: "fixed" }],
    });
    const { container } = render(<FileExplorer sessionId="s1" />);

    await userEvent.click(await screen.findByTitle("切換磁碟機"));
    expect(screen.getByRole("button", { name: "D:" })).toBeInTheDocument();

    const backdrop = container.querySelector(".fe-drive-backdrop");
    expect(backdrop).not.toBeNull();
    await userEvent.click(backdrop!);

    expect(screen.queryByRole("button", { name: "D:" })).not.toBeInTheDocument();
  });

  it("labels network drives so a slow one is not a surprise", async () => {
    // GetDriveTypeW cannot tell a disconnected mapping from a live one, so the
    // label is a warning, not a guarantee — see the spec.
    mockCommands({
      pty_get_cwd: () => "C:/Users",
      list_drives: () => [
        { path: "C:/", kind: "fixed" },
        { path: "Z:/", kind: "network" },
      ],
    });
    const { container } = render(<FileExplorer sessionId="s1" />);

    await userEvent.click(await screen.findByTitle("切換磁碟機"));

    const menu = within(container.querySelector(".fe-drive-menu")!);
    const zItem = await menu.findByRole("button", { name: /^Z:/ });
    expect(zItem).toHaveTextContent("網路");
    expect(menu.getByRole("button", { name: /^C:/ })).not.toHaveTextContent("網路");
  });
});
