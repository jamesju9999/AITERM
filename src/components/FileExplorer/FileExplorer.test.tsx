import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
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
    if (handler) return Promise.resolve(handler(args));
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
