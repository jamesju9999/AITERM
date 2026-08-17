import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";
import { MemoryRouter, Routes, Route } from "react-router-dom";
import { KnowledgeBaseView } from "./index";
import { LocaleProvider } from "../../contexts/LocaleContext";
import type { Notebook } from "../../ipc/knowledgeBase";

const sync = vi.fn();
const pythonEnvEnsure = vi.fn();
const pythonEnvStatusMock = vi.fn();

const testNotebook: Notebook = {
  id: "nb1",
  name: "Test Notebook",
  folder_path: "/docs",
  embed_provider_id: null,
  embed_model: null,
  embed_dim: null,
  last_synced_at: 1700000000,
  created_at: "2024-01-01T00:00:00Z",
};

vi.mock("../../hooks/useNotebooks", () => ({
  useNotebooks: () => ({
    notebooks: [testNotebook],
    loading: false,
    error: null,
    syncingIds: new Set(),
    syncProgressById: {},
    refresh: vi.fn(),
    create: vi.fn(),
    remove: vi.fn(),
    sync,
  }),
}));

vi.mock("../../hooks/useKnowledgeBaseChat", () => ({
  useKnowledgeBaseChat: () => ({
    messages: [],
    isStreaming: false,
    error: null,
    isFallbackMode: false,
    tokenCount: 0,
    tokenLimit: 8000,
    sessions: [],
    activeChatSessionId: null,
    send: vi.fn(),
    clear: vi.fn(),
    loadSession: vi.fn(),
    deleteSession: vi.fn(),
  }),
}));

vi.mock("../../ipc/config", () => ({
  getConfig: vi.fn().mockResolvedValue({ submit_shortcut: "enter" }),
}));
vi.mock("../../ipc/provider", () => ({
  listProviders: vi.fn().mockResolvedValue([]),
}));
vi.mock("../../ipc/knowledgeBase", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../ipc/knowledgeBase")>();
  return { ...actual, kbOpenDocument: vi.fn() };
});
vi.mock("../../ipc/pythonEnv", () => ({
  pythonEnvEnsure: (p: string) => pythonEnvEnsure(p),
  pythonEnvStatus: () => pythonEnvStatusMock(),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

function renderView() {
  return render(
    <MemoryRouter>
      <LocaleProvider>
        <KnowledgeBaseView isActive={true} />
      </LocaleProvider>
    </MemoryRouter>
  );
}

describe("KnowledgeBaseView python env gate", () => {
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

  it("ensures doc_core before syncing a notebook", async () => {
    sync.mockResolvedValue({ indexed: 1, failed: 0, deleted: 0 });
    renderView();

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /^(同步|Sync)$/ }));
    });

    expect(pythonEnvEnsure).toHaveBeenCalledWith("doc_core");
    expect(sync).toHaveBeenCalledWith("nb1");
  });

  it("does not sync when doc_core fails to install", async () => {
    pythonEnvEnsure.mockRejectedValue("無法取得 Python：network unreachable");
    pythonEnvStatusMock.mockResolvedValue({
      uvAvailable: true,
      pythonVersion: null,
      installed: [],
      venvPath: "/data/python-env",
      userInterpreter: null,
    });
    renderView();

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /^(同步|Sync)$/ }));
    });

    expect(pythonEnvEnsure).toHaveBeenCalledWith("doc_core");
    expect(sync).not.toHaveBeenCalled();
  });
});

describe("KnowledgeBaseView pick-interpreter escape hatch", () => {
  beforeEach(() => {
    vi.clearAllMocks();
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
    render(
      <MemoryRouter initialEntries={["/"]}>
        <LocaleProvider>
          <Routes>
            <Route path="/" element={<KnowledgeBaseView isActive={true} />} />
            <Route path="/settings" element={<div>SETTINGS_STUB</div>} />
          </Routes>
        </LocaleProvider>
      </MemoryRouter>
    );

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /^(同步|Sync)$/ }));
    });

    const pickBtn = await screen.findByRole("button", { name: /interpreter|手動指定/ });
    fireEvent.click(pickBtn);

    expect(screen.getByText("SETTINGS_STUB")).toBeInTheDocument();
  });
});
