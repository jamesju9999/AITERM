import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";
import { MemoryRouter, Routes, Route } from "react-router-dom";
import { DocConverterView } from "./DocConverterView";

// Mock IPC
vi.mock("../../ipc/docConvert", () => ({
  documentConvert: vi.fn(),
  documentConvertPickFile: vi.fn(),
}));
const getConfigMock = vi.fn();
vi.mock("../../ipc/config", () => ({
  getConfig: () => getConfigMock(),
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
const askConfirm = vi.fn();
const saveDialogMock = vi.fn();
// Not window.confirm: Tauri's webview never shows the JS dialog, so the audio
// prompt was treated as an unconditional yes in production. jsdom returns
// falsy, so this suite passed while behaving the opposite way.
vi.mock("@tauri-apps/plugin-dialog", () => ({
  confirm: (...args: unknown[]) => askConfirm(...args),
  save: (...args: unknown[]) => saveDialogMock(...args),
}));

const writeTextFileMock = vi.fn();
vi.mock("../../ipc/fs", () => ({
  writeTextFile: (...args: unknown[]) => writeTextFileMock(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (eventName: string, cb: (e: unknown) => void) => listenMock(eventName, cb),
}));

const pythonEnvEnsure = vi.fn();
const pythonEnvStatusMock = vi.fn();
vi.mock("../../ipc/pythonEnv", () => ({
  pythonEnvEnsure: (p: string) => pythonEnvEnsure(p),
  pythonEnvStatus: () => pythonEnvStatusMock(),
}));

import { documentConvert, documentConvertPickFile } from "../../ipc/docConvert";
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
    <MemoryRouter>
      <LocaleProvider>
        <DocConverterView isActive={true} />
      </LocaleProvider>
    </MemoryRouter>
  );
}

describe("DocConverterView", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getConfigMock.mockResolvedValue({ doc_convert_engine: "auto" });
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

  it("calls documentConvertPickFile when dropzone is clicked", async () => {
    vi.mocked(documentConvertPickFile).mockResolvedValue(null);
    renderView();
    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });
    expect(documentConvertPickFile).toHaveBeenCalledOnce();
  });

  it("calls documentConvert with picked path and shows extracted state", async () => {
    vi.mocked(documentConvertPickFile).mockResolvedValue("/tmp/test.docx");
    vi.mocked(documentConvert).mockResolvedValue("# Hello\nworld");
    renderView();
    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });
    expect(documentConvert).toHaveBeenCalledWith("/tmp/test.docx", undefined);
    expect(screen.getByText(/test\.docx/)).toBeInTheDocument();
  });

  it("shows error when documentConvert rejects", async () => {
    vi.mocked(documentConvertPickFile).mockResolvedValue("/tmp/bad.xyz");
    vi.mocked(documentConvert).mockRejectedValue(new Error("unsupported format"));
    renderView();
    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });
    expect(screen.getByText(/提取失敗/)).toBeInTheDocument();
  });

  it("does nothing when file picker is cancelled (null path)", async () => {
    vi.mocked(documentConvertPickFile).mockResolvedValue(null);
    renderView();
    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });
    expect(documentConvert).not.toHaveBeenCalled();
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

  it("skips the Python gate entirely for an anydoc-covered file under auto engine", async () => {
    getConfigMock.mockResolvedValue({ doc_convert_engine: "auto" });
    vi.mocked(documentConvertPickFile).mockResolvedValue("/tmp/report.docx");
    vi.mocked(documentConvert).mockResolvedValue("# report");
    renderView();
    await act(async () => {}); // let the config-fetch effect resolve

    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });

    expect(pythonEnvEnsure).not.toHaveBeenCalled();
    expect(documentConvert).toHaveBeenCalledWith("/tmp/report.docx", undefined);
    expect(screen.getByText(/report\.docx/)).toBeInTheDocument();
  });

  it("still runs the Python gate for a MarkItDown-only file (image) under auto engine", async () => {
    getConfigMock.mockResolvedValue({ doc_convert_engine: "auto" });
    vi.mocked(documentConvertPickFile).mockResolvedValue("/tmp/photo.png");
    vi.mocked(documentConvert).mockResolvedValue("# photo");
    renderView();
    await act(async () => {});

    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });

    expect(pythonEnvEnsure).toHaveBeenCalledWith("doc_core");
    expect(documentConvert).toHaveBeenCalledWith("/tmp/photo.png", undefined);
  });

  it("runs the Python gate for every file, including anydoc-covered ones, under markitdown_only engine", async () => {
    getConfigMock.mockResolvedValue({ doc_convert_engine: "markitdown_only" });
    vi.mocked(documentConvertPickFile).mockResolvedValue("/tmp/report.docx");
    vi.mocked(documentConvert).mockResolvedValue("# report");
    renderView();
    await act(async () => {});

    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });

    expect(pythonEnvEnsure).toHaveBeenCalledWith("doc_core");
    expect(documentConvert).toHaveBeenCalledWith("/tmp/report.docx", undefined);
  });
});

describe("DocConverterView audio profile candidate install", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getConfigMock.mockResolvedValue({ doc_convert_engine: "auto" });
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
    askConfirm.mockResolvedValue(true);
    vi.mocked(documentConvertPickFile).mockResolvedValue("/tmp/voice.mp3");
    vi.mocked(documentConvert).mockResolvedValue("# voice");
    renderView();

    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });

    expect(askConfirm).toHaveBeenCalledWith(
      expect.stringMatching(/音訊/),
      // Labels come from the app's locale; without them the buttons follow the
      // OS language and can end up in a different language than the message.
      expect.objectContaining({ okLabel: "確定", cancelLabel: "取消" }),
    );
    expect(pythonEnvEnsure).toHaveBeenCalledWith("doc_core");
    expect(pythonEnvEnsure).toHaveBeenCalledWith("doc_audio");
    expect(documentConvert).toHaveBeenCalledWith("/tmp/voice.mp3", undefined);
    expect(screen.getByText(/voice\.mp3/)).toBeInTheDocument();
  });

  it("retries doc_audio, not doc_core, when the gate's retry button is clicked after the audio install fails", async () => {
    // doc_core succeeds; the separate doc_audio ensureProfile call fails.
    // Retrying doc_core here would succeed instantly (it's already
    // installed) and silently close the gate without fixing anything.
    askConfirm.mockResolvedValue(true);
    vi.mocked(documentConvertPickFile).mockResolvedValue("/tmp/voice.mp3");
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

  it("aborts the conversion, without calling documentConvert, when the user declines the audio install", async () => {
    askConfirm.mockResolvedValue(false);
    vi.mocked(documentConvertPickFile).mockResolvedValue("/tmp/voice.mp3");
    renderView();

    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });

    expect(askConfirm).toHaveBeenCalled();
    expect(pythonEnvEnsure).toHaveBeenCalledWith("doc_core");
    expect(pythonEnvEnsure).not.toHaveBeenCalledWith("doc_audio");
    expect(documentConvert).not.toHaveBeenCalled();
  });

  it("does not prompt again when doc_audio is already installed", async () => {
    pythonEnvStatusMock.mockResolvedValue({
      uvAvailable: true,
      pythonVersion: "3.12.13",
      installed: ["doc_core", "doc_audio"],
      venvPath: "/data/python-env",
      userInterpreter: null,
    });
    askConfirm.mockReset();
    vi.mocked(documentConvertPickFile).mockResolvedValue("/tmp/voice.mp3");
    vi.mocked(documentConvert).mockResolvedValue("# voice");
    renderView();

    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });

    expect(askConfirm).not.toHaveBeenCalled();
    expect(pythonEnvEnsure).not.toHaveBeenCalledWith("doc_audio");
    expect(documentConvert).toHaveBeenCalledWith("/tmp/voice.mp3", undefined);
  });

  it("does not check for the audio profile at all for a non-audio file", async () => {
    askConfirm.mockReset();
    vi.mocked(documentConvertPickFile).mockResolvedValue("/tmp/report.pdf");
    vi.mocked(documentConvert).mockResolvedValue("# report");
    renderView();

    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });

    expect(pythonEnvStatusMock).not.toHaveBeenCalled();
    expect(askConfirm).not.toHaveBeenCalled();
    expect(documentConvert).toHaveBeenCalledWith("/tmp/report.pdf", undefined);
  });

  it("converts an image straight away — it no longer triggers the audio-profile prompt", async () => {
    // Regression guard: images used to be lumped into the "media" profile
    // alongside audio, but markitdown[image] isn't a real extra. converter.py
    // handles images itself (vision API, Pillow fallback from doc_core), so
    // an image must behave exactly like any other doc_core-only format.
    askConfirm.mockReset();
    vi.mocked(documentConvertPickFile).mockResolvedValue("/tmp/photo.png");
    vi.mocked(documentConvert).mockResolvedValue("# photo");
    renderView();

    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });

    expect(pythonEnvStatusMock).not.toHaveBeenCalled();
    expect(askConfirm).not.toHaveBeenCalled();
    expect(pythonEnvEnsure).not.toHaveBeenCalledWith("doc_audio");
    expect(documentConvert).toHaveBeenCalledWith("/tmp/photo.png", undefined);
  });
});

describe("DocConverterView pick-interpreter escape hatch", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getConfigMock.mockResolvedValue({ doc_convert_engine: "auto" });
    pythonEnvEnsure.mockRejectedValue("無法取得 Python：network unreachable");
    pythonEnvStatusMock.mockResolvedValue({
      uvAvailable: true,
      pythonVersion: null,
      installed: [],
      venvPath: "/data/python-env",
      userInterpreter: null,
    });
  });

  it("navigates to Settings → General when the user picks an interpreter manually", async () => {
    // .png isn't anydoc-covered, so it still needs the MarkItDown Python
    // gate under the default "auto" engine — unlike .pdf, which anydoc
    // converts natively and would never trigger this gate at all.
    vi.mocked(documentConvertPickFile).mockResolvedValue("/tmp/report.png");
    render(
      <MemoryRouter initialEntries={["/"]}>
        <LocaleProvider>
          <Routes>
            <Route path="/" element={<DocConverterView isActive={true} />} />
            <Route path="/settings" element={<div>SETTINGS_STUB</div>} />
          </Routes>
        </LocaleProvider>
      </MemoryRouter>
    );

    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });

    const pickBtn = await screen.findByRole("button", { name: /interpreter|手動指定/ });
    fireEvent.click(pickBtn);

    expect(screen.getByText("SETTINGS_STUB")).toBeInTheDocument();
  });
});

describe("DocConverterView download with save dialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getConfigMock.mockResolvedValue({ doc_convert_engine: "auto" });
    pythonEnvEnsure.mockResolvedValue(undefined);
    pythonEnvStatusMock.mockResolvedValue({
      uvAvailable: true,
      pythonVersion: "3.12.13",
      installed: ["doc_core"],
      venvPath: "/data/python-env",
      userInterpreter: null,
    });
  });

  async function convertAFile() {
    vi.mocked(documentConvertPickFile).mockResolvedValue("/tmp/report.docx");
    vi.mocked(documentConvert).mockResolvedValue("# Hello\nworld");
    renderView();
    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });
  }

  it("opens a native save dialog and writes the file when downloading the raw markdown", async () => {
    saveDialogMock.mockResolvedValue("/Users/me/Desktop/report.md");
    await convertAFile();

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /下載 Markdown/ }));
    });

    expect(saveDialogMock).toHaveBeenCalledWith({
      defaultPath: "report.md",
      filters: [{ name: "Markdown", extensions: ["md"] }],
    });
    expect(writeTextFileMock).toHaveBeenCalledWith("/Users/me/Desktop/report.md", "# Hello\nworld");
  });

  it("does not write anything when the user cancels the save dialog", async () => {
    saveDialogMock.mockResolvedValue(null);
    await convertAFile();

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /下載 Markdown/ }));
    });

    expect(saveDialogMock).toHaveBeenCalledOnce();
    expect(writeTextFileMock).not.toHaveBeenCalled();
  });
});
