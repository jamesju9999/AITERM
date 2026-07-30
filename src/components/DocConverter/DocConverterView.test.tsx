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
const dragDropListenCalls: number[] = [];
let pythonEnvLogListener: ((e: { payload: { level: string; message: string } }) => void) | undefined;
const listenMock = vi.fn((eventName: string, cb: (e: unknown) => void) => {
  if (eventName === "tauri://drag-drop") dragDropListenCalls.push(dragDropListenCalls.length);
  if (eventName === "python-env-log") {
    pythonEnvLogListener = cb as (e: { payload: { level: string; message: string } }) => void;
  }
  return Promise.resolve(() => {});
});
vi.mock("@tauri-apps/api/event", () => ({
  listen: (eventName: string, cb: (e: unknown) => void) => listenMock(eventName, cb),
}));

const pythonEnvEnsure = vi.fn();
const pythonEnvStatusMock = vi.fn();
vi.mock("../../ipc/pythonEnv", () => ({
  pythonEnvEnsure: (p: string) => pythonEnvEnsure(p),
  pythonEnvStatus: () => pythonEnvStatusMock(),
}));

import { markitdownConvert, markitdownPickFile } from "../../ipc/markitdown";
import { LocaleProvider } from "../../contexts/LocaleContext";

beforeEach(() => {
  dragDropListenCalls.length = 0;
  pythonEnvLogListener = undefined;
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
    pythonEnvEnsure.mockResolvedValue(undefined);
    pythonEnvStatusMock.mockResolvedValue({
      uvAvailable: true,
      pythonVersion: "3.12.13",
      installed: ["doc_core"],
      venvPath: "/data/python-env",
      userInterpreter: null,
    });
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
    expect(markitdownConvert).toHaveBeenCalledWith("/tmp/test.docx", undefined);
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

  it("registers the OS drag-drop listener exactly once, even as install log lines re-render the view", async () => {
    // usePythonEnvGate() returns a new object every render. processFilePath
    // used to depend on that whole object, so every log line appended during
    // an install (each one a state update, each one a re-render) produced a
    // brand-new processFilePath — and the drag-drop useEffect tore down and
    // re-registered its listener on every single one of them.
    renderView();
    await act(async () => {}); // let the initial effects run
    expect(dragDropListenCalls.length).toBe(1);

    expect(pythonEnvLogListener).toBeDefined();
    await act(async () => {
      for (let i = 0; i < 20; i++) {
        pythonEnvLogListener!({ payload: { level: "info", message: `line ${i}` } });
      }
    });

    expect(dragDropListenCalls.length).toBe(1);
  });
});

describe("DocConverterView audio profile candidate install", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    pythonEnvEnsure.mockResolvedValue(undefined);
    pythonEnvStatusMock.mockResolvedValue({
      uvAvailable: true,
      pythonVersion: "3.12.13",
      installed: ["doc_core"],
      venvPath: "/data/python-env",
      userInterpreter: null,
    });
  });

  it("prompts before installing doc_audio for an audio file, then converts once confirmed", async () => {
    vi.spyOn(window, "confirm").mockReturnValue(true);
    vi.mocked(markitdownPickFile).mockResolvedValue("/tmp/voice.mp3");
    vi.mocked(markitdownConvert).mockResolvedValue("# voice");
    renderView();

    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });

    expect(window.confirm).toHaveBeenCalledWith(expect.stringMatching(/音訊/));
    expect(pythonEnvEnsure).toHaveBeenCalledWith("doc_core");
    expect(pythonEnvEnsure).toHaveBeenCalledWith("doc_audio");
    expect(markitdownConvert).toHaveBeenCalledWith("/tmp/voice.mp3", undefined);
    expect(screen.getByText(/voice\.mp3/)).toBeInTheDocument();
  });

  it("retries doc_audio, not doc_core, when the gate's retry button is clicked after the audio install fails", async () => {
    // doc_core succeeds; the separate doc_audio ensureProfile call fails.
    // Retrying doc_core here would succeed instantly (it's already
    // installed) and silently close the gate without fixing anything.
    vi.spyOn(window, "confirm").mockReturnValue(true);
    vi.mocked(markitdownPickFile).mockResolvedValue("/tmp/voice.mp3");
    pythonEnvEnsure.mockImplementation((p: string) =>
      p === "doc_audio"
        ? Promise.reject("安裝 doc_audio 相依套件失敗：boom")
        : Promise.resolve(undefined)
    );
    renderView();

    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });

    const retryBtn = await screen.findByRole("button", { name: /重試|Retry/ });
    pythonEnvEnsure.mockClear();
    pythonEnvEnsure.mockResolvedValue(undefined);

    await act(async () => {
      fireEvent.click(retryBtn);
    });

    expect(pythonEnvEnsure).toHaveBeenCalledWith("doc_audio");
    expect(pythonEnvEnsure).not.toHaveBeenCalledWith("doc_core");
  });

  it("aborts the conversion, without calling markitdownConvert, when the user declines the audio install", async () => {
    vi.spyOn(window, "confirm").mockReturnValue(false);
    vi.mocked(markitdownPickFile).mockResolvedValue("/tmp/voice.mp3");
    renderView();

    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });

    expect(window.confirm).toHaveBeenCalled();
    expect(pythonEnvEnsure).toHaveBeenCalledWith("doc_core");
    expect(pythonEnvEnsure).not.toHaveBeenCalledWith("doc_audio");
    expect(markitdownConvert).not.toHaveBeenCalled();
  });

  it("does not prompt again when doc_audio is already installed", async () => {
    pythonEnvStatusMock.mockResolvedValue({
      uvAvailable: true,
      pythonVersion: "3.12.13",
      installed: ["doc_core", "doc_audio"],
      venvPath: "/data/python-env",
      userInterpreter: null,
    });
    vi.spyOn(window, "confirm");
    vi.mocked(markitdownPickFile).mockResolvedValue("/tmp/voice.mp3");
    vi.mocked(markitdownConvert).mockResolvedValue("# voice");
    renderView();

    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });

    expect(window.confirm).not.toHaveBeenCalled();
    expect(pythonEnvEnsure).not.toHaveBeenCalledWith("doc_audio");
    expect(markitdownConvert).toHaveBeenCalledWith("/tmp/voice.mp3", undefined);
  });

  it("does not check for the audio profile at all for a non-audio file", async () => {
    vi.spyOn(window, "confirm");
    vi.mocked(markitdownPickFile).mockResolvedValue("/tmp/report.pdf");
    vi.mocked(markitdownConvert).mockResolvedValue("# report");
    renderView();

    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });

    expect(pythonEnvStatusMock).not.toHaveBeenCalled();
    expect(window.confirm).not.toHaveBeenCalled();
    expect(markitdownConvert).toHaveBeenCalledWith("/tmp/report.pdf", undefined);
  });

  it("converts an image straight away — it no longer triggers the audio-profile prompt", async () => {
    // Regression guard: images used to be lumped into the "media" profile
    // alongside audio, but markitdown[image] isn't a real extra. converter.py
    // handles images itself (vision API, Pillow fallback from doc_core), so
    // an image must behave exactly like any other doc_core-only format.
    vi.spyOn(window, "confirm");
    vi.mocked(markitdownPickFile).mockResolvedValue("/tmp/photo.png");
    vi.mocked(markitdownConvert).mockResolvedValue("# photo");
    renderView();

    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });

    expect(pythonEnvStatusMock).not.toHaveBeenCalled();
    expect(window.confirm).not.toHaveBeenCalled();
    expect(pythonEnvEnsure).not.toHaveBeenCalledWith("doc_audio");
    expect(markitdownConvert).toHaveBeenCalledWith("/tmp/photo.png", undefined);
  });
});
