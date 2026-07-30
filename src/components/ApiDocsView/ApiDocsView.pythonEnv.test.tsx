import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";
import { MemoryRouter, Routes, Route } from "react-router-dom";
import { ApiDocsView } from "./ApiDocsView";
import { LocaleProvider } from "../../contexts/LocaleContext";

const pythonEnvEnsure = vi.fn();
const pythonEnvStatusMock = vi.fn();
vi.mock("../../ipc/pythonEnv", () => ({
  pythonEnvEnsure: (p: string) => pythonEnvEnsure(p),
  pythonEnvStatus: () => pythonEnvStatusMock(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

vi.mock("../../ipc/provider", () => ({
  listProviders: vi.fn().mockResolvedValue([]),
}));

vi.mock("../../ipc/vcs", () => ({
  pickFolder: vi.fn(),
}));

vi.mock("../../ipc/apiDocs", () => ({
  apiDocsDetect: vi.fn().mockResolvedValue("generic"),
  apiDocsFetchTree: vi.fn().mockResolvedValue([]),
  apiDocsExtract: vi.fn(),
  apiDocsLogin: vi.fn(),
  apiDocsLogout: vi.fn(),
  apiDocsAuthStatus: vi.fn().mockResolvedValue({ logged_in: false, account: "" }),
  onApiDocsDetected: vi.fn().mockResolvedValue(() => {}),
  onApiDocsProgress: vi.fn().mockResolvedValue(() => {}),
  onApiDocsLog: vi.fn().mockResolvedValue(() => {}),
  onApiDocsDone: vi.fn().mockResolvedValue(() => {}),
  DEFAULT_KEEP_OPTIONS: {
    description: true,
    parameters: true,
    request_body: true,
    responses: true,
    code_samples: true,
  },
}));

function renderView(withSettingsRoute = false) {
  return render(
    <MemoryRouter initialEntries={["/"]}>
      <LocaleProvider>
        {withSettingsRoute ? (
          <Routes>
            <Route path="/" element={<ApiDocsView isActive={true} />} />
            <Route path="/settings" element={<div>SETTINGS_STUB</div>} />
          </Routes>
        ) : (
          <ApiDocsView isActive={true} />
        )}
      </LocaleProvider>
    </MemoryRouter>
  );
}

describe("ApiDocsView python env gate", () => {
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
    renderView(true);

    await act(async () => {
      fireEvent.change(screen.getByPlaceholderText(/docs\.stripe\.com/), {
        target: { value: "https://docs.example.com" },
      });
      fireEvent.click(screen.getByRole("button", { name: /Load Doc Tree|載入文件樹/ }));
    });

    const pickBtn = await screen.findByRole("button", { name: /interpreter|手動指定/ });
    fireEvent.click(pickBtn);

    expect(screen.getByText("SETTINGS_STUB")).toBeInTheDocument();
  });
});
