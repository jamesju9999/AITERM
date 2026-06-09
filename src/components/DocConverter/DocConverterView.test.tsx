import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";
import { DocConverterView } from "./DocConverterView";

// Mock IPC
vi.mock("../../ipc/markitdown", () => ({
  markitdownConvert: vi.fn(),
  markitdownPickFile: vi.fn(),
}));
vi.mock("../../ipc/provider", () => ({
  listProviders: vi.fn().mockResolvedValue([]),
}));
vi.mock("../../ipc/ai", () => ({
  aiChat: vi.fn(),
  formatAiError: vi.fn((e) => String(e)),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

import { markitdownConvert, markitdownPickFile } from "../../ipc/markitdown";
import { LocaleProvider } from "../../contexts/LocaleContext";

beforeEach(() => {
  const localStorageMock = {
    getItem: vi.fn((key: string) => (key === "aiterm_locale" ? "zh-TW" : null)),
    setItem: vi.fn(),
    removeItem: vi.fn(),
    clear: vi.fn(),
    key: vi.fn(),
    length: 0,
  };
  Object.defineProperty(window, "localStorage", {
    value: localStorageMock,
    writable: true,
  });
});

function renderView() {
  return render(
    <LocaleProvider>
      <DocConverterView isActive={true} />
    </LocaleProvider>
  );
}

describe("DocConverterView", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders dropzone", () => {
    renderView();
    expect(screen.getByText(/拖放或點擊選擇檔案/)).toBeInTheDocument();
  });

  it("calls markitdownPickFile when dropzone is clicked", async () => {
    vi.mocked(markitdownPickFile).mockResolvedValue(null);
    renderView();
    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });
    expect(markitdownPickFile).toHaveBeenCalledOnce();
  });

  it("calls markitdownConvert with picked path and shows extracted state", async () => {
    vi.mocked(markitdownPickFile).mockResolvedValue("/tmp/test.docx");
    vi.mocked(markitdownConvert).mockResolvedValue("# Hello\nworld");
    renderView();
    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });
    expect(markitdownConvert).toHaveBeenCalledWith("/tmp/test.docx");
    expect(screen.getByText(/test\.docx/)).toBeInTheDocument();
  });

  it("shows error when markitdownConvert rejects", async () => {
    vi.mocked(markitdownPickFile).mockResolvedValue("/tmp/bad.xyz");
    vi.mocked(markitdownConvert).mockRejectedValue(new Error("unsupported format"));
    renderView();
    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });
    expect(screen.getByText(/提取失敗/)).toBeInTheDocument();
  });

  it("does nothing when file picker is cancelled (null path)", async () => {
    vi.mocked(markitdownPickFile).mockResolvedValue(null);
    renderView();
    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });
    expect(markitdownConvert).not.toHaveBeenCalled();
  });
});
