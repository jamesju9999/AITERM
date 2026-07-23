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

// Component call order on mount:
//   1. getSessionCwd (seed ptyCwdRef useEffect)
//   2. listDirectory (loadDir useEffect)
//   3. getSessionCwd (inside loadDir when path === "")
// Then polling getSessionCwd every 1500ms (ignored in tests)

describe("FileExplorer — file selection", () => {
  it("shows empty viewer state initially", async () => {
    invokeMock
      .mockResolvedValueOnce(null) // getSessionCwd (ptyCwdRef seed)
      .mockResolvedValueOnce([])   // listDirectory → empty
      .mockResolvedValueOnce(null) // getSessionCwd (inside loadDir)
      .mockResolvedValue(null);    // polling
    render(<FileExplorer sessionId="s1" />);
    await waitFor(() =>
      expect(screen.getByText(/選擇左側檔案以預覽內容/)).toBeInTheDocument()
    );
  });

  it("clicking a file loads its content in the viewer", async () => {
    invokeMock
      .mockResolvedValueOnce(null) // getSessionCwd (ptyCwdRef seed)
      .mockResolvedValueOnce([     // listDirectory
        { name: "index.ts", path: "/p/index.ts", is_dir: false, size: 20 },
      ])
      .mockResolvedValueOnce("/p") // getSessionCwd (inside loadDir)
      .mockResolvedValueOnce({ content: "export default 1;", truncated: false }) // readFile
      .mockResolvedValue(null); // polling

    render(<FileExplorer sessionId="s1" />);
    await waitFor(() => screen.getByText("index.ts"));

    await userEvent.click(screen.getByText("index.ts"));

    await waitFor(() =>
      expect(screen.getByText(/export default 1;/)).toBeInTheDocument()
    );
  });

  it("clicking a directory does NOT load file content", async () => {
    invokeMock
      .mockResolvedValueOnce(null) // getSessionCwd (ptyCwdRef seed)
      .mockResolvedValueOnce([     // listDirectory
        { name: "src", path: "/p/src", is_dir: true, size: null },
      ])
      .mockResolvedValueOnce("/p") // getSessionCwd (inside loadDir)
      .mockResolvedValueOnce([])   // listDirectory for expanded dir
      .mockResolvedValue(null);    // polling

    render(<FileExplorer sessionId="s1" />);
    await waitFor(() => screen.getByText("src"));

    await userEvent.click(screen.getByText("src"));

    expect(invokeMock).not.toHaveBeenCalledWith("pty_read_file", expect.anything());
    expect(screen.getByText(/選擇左側檔案以預覽內容/)).toBeInTheDocument();
  });
});

describe("FileExplorer — switch terminal here", () => {
  it("does not render the button when onSwitchTerminalHere is not provided", async () => {
    invokeMock
      .mockResolvedValueOnce(null)
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce("/p")
      .mockResolvedValue(null);
    render(<FileExplorer sessionId="s1" />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    expect(screen.queryByTitle("切換終端機到此目錄")).not.toBeInTheDocument();
  });

  it("calls onSwitchTerminalHere with the currently-browsed directory when clicked", async () => {
    invokeMock
      .mockResolvedValueOnce(null) // getSessionCwd (ptyCwdRef seed)
      .mockResolvedValueOnce([])   // listDirectory
      .mockResolvedValueOnce("/Users/jamesju/Downloads") // getSessionCwd (inside loadDir)
      .mockResolvedValue(null);    // polling

    const onSwitch = vi.fn();
    render(<FileExplorer sessionId="s1" onSwitchTerminalHere={onSwitch} />);

    const button = await screen.findByTitle("切換終端機到此目錄");
    expect(button).not.toBeDisabled();
    await userEvent.click(button);

    expect(onSwitch).toHaveBeenCalledWith("/Users/jamesju/Downloads");
  });
});
