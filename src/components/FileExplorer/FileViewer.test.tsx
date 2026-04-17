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
    render(<FileViewer file={null} />);
    expect(screen.getByText(/選擇左側檔案以預覽內容/)).toBeInTheDocument();
  });

  it("shows file content after successful load", async () => {
    invokeMock.mockResolvedValueOnce({ content: "const x = 1;\n", truncated: false });
    render(<FileViewer file={mockFile} />);
    await waitFor(() =>
      expect(screen.getByText(/const x = 1;/)).toBeInTheDocument()
    );
    expect(screen.getByText("hello.ts")).toBeInTheDocument();
  });

  it("shows truncation banner when truncated=true", async () => {
    invokeMock.mockResolvedValueOnce({ content: "big content", truncated: true });
    render(<FileViewer file={mockFile} />);
    await waitFor(() =>
      expect(screen.getByText(/僅顯示前 10 MB/)).toBeInTheDocument()
    );
  });

  it("shows error message when read fails", async () => {
    invokeMock.mockRejectedValueOnce(new Error("permission denied"));
    render(<FileViewer file={mockFile} />);
    await waitFor(() =>
      expect(screen.getByText(/permission denied/)).toBeInTheDocument()
    );
  });

  it("shows binary message when error is 'binary'", async () => {
    invokeMock.mockRejectedValueOnce("binary");
    render(<FileViewer file={mockFile} />);
    await waitFor(() =>
      expect(screen.getByText(/二進位格式/)).toBeInTheDocument()
    );
  });

  it("shows loading state while fetching", async () => {
    let resolveInvoke!: (v: unknown) => void;
    const invokePromise = new Promise(resolve => { resolveInvoke = resolve; });
    invokeMock.mockReturnValueOnce(invokePromise);
    render(<FileViewer file={mockFile} />);
    expect(screen.getByText(/載入中/)).toBeInTheDocument();
    resolveInvoke({ content: "test", truncated: false });
    await waitFor(() => expect(screen.getByText(/test/)).toBeInTheDocument());
  });

  it("re-fetches when file path changes", async () => {
    const fileA: DirEntry = { name: "a.ts", path: "/p/a.ts", is_dir: false, size: 10 };
    const fileB: DirEntry = { name: "b.ts", path: "/p/b.ts", is_dir: false, size: 20 };
    invokeMock
      .mockResolvedValueOnce({ content: "content-a", truncated: false })
      .mockResolvedValueOnce({ content: "content-b", truncated: false });

    const { rerender } = render(<FileViewer file={fileA} />);
    await waitFor(() => expect(screen.getByText(/content-a/)).toBeInTheDocument());

    rerender(<FileViewer file={fileB} />);
    await waitFor(() => expect(screen.getByText(/content-b/)).toBeInTheDocument());
    expect(invokeMock).toHaveBeenCalledTimes(2);
  });
});
