import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { FileViewer } from "./FileViewer";
import type { DirEntry } from "../../ipc/fs";

const mockFile: DirEntry = {
  name: "hello.ts",
  path: "/project/hello.ts",
  is_dir: false,
  size: 42,
};

beforeEach(() => {
  invokeMock.mockReset();
});

describe("FileViewer", () => {
  it("shows empty state when no file selected", () => {
    render(<FileViewer sessionId="s1" file={null} />);
    expect(screen.getByText(/選擇左側檔案以預覽內容/)).toBeInTheDocument();
  });

  it("shows file content after successful load", async () => {
    invokeMock.mockResolvedValueOnce({ content: "const x = 1;\n", truncated: false });
    render(<FileViewer sessionId="s1" file={mockFile} />);
    await waitFor(() =>
      expect(screen.getByText(/const x = 1;/)).toBeInTheDocument()
    );
    expect(screen.getByText("hello.ts")).toBeInTheDocument();
  });

  it("shows truncation banner when truncated=true", async () => {
    invokeMock.mockResolvedValueOnce({ content: "big content", truncated: true });
    render(<FileViewer sessionId="s1" file={mockFile} />);
    await waitFor(() =>
      expect(screen.getByText(/僅顯示前 10 MB/)).toBeInTheDocument()
    );
  });

  it("shows error message when read fails", async () => {
    invokeMock.mockRejectedValueOnce(new Error("permission denied"));
    render(<FileViewer sessionId="s1" file={mockFile} />);
    await waitFor(() =>
      expect(screen.getByText(/permission denied/)).toBeInTheDocument()
    );
  });

  it("shows binary message when error is 'binary'", async () => {
    invokeMock.mockRejectedValueOnce("binary");
    render(<FileViewer sessionId="s1" file={mockFile} />);
    await waitFor(() =>
      expect(screen.getByText(/二進位格式/)).toBeInTheDocument()
    );
  });
});
