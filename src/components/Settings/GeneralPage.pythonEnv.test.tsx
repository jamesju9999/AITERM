import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { GeneralPage } from "./GeneralPage";

const status = vi.fn();
const reset = vi.fn();
const setInterpreter = vi.fn();

vi.mock("../../ipc/pythonEnv", () => ({
  pythonEnvStatus: () => status(),
  pythonEnvReset: (purge: boolean) => reset(purge),
  pythonEnvSetInterpreter: (p: string | null) => setInterpreter(p),
}));

// GeneralPage also calls getConfig()/getTelegramConfig()/appimageIntegrationState()
// on mount via the real `invoke`, which throws outside a Tauri context (see
// GeneralPage.appimage.test.tsx). Stub it so those unrelated calls don't blow up.
const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

describe("GeneralPage — Python environment", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_config") {
        return Promise.resolve({
          execution_mode: "graded",
          submit_shortcut: "enter",
          max_agent_steps: 5,
          default_tab: "terminal",
        });
      }
      if (cmd === "telegram_get_config") return Promise.resolve({ bot_token: null, chat_id: null });
      if (cmd === "appimage_integration_state") return Promise.resolve({ state: "not_appimage" });
      return Promise.resolve(null);
    });
    status.mockResolvedValue({
      uvAvailable: true,
      pythonVersion: "3.12.13",
      installed: ["doc_core"],
      venvPath: "/data/python-env",
      userInterpreter: null,
    });
  });

  it("shows the resolved version and venv path", async () => {
    render(<GeneralPage />);
    await waitFor(() => expect(screen.getByText(/3\.12\.13/)).toBeTruthy());
    expect(screen.getByText("/data/python-env")).toBeTruthy();
  });

  it("rebuild keeps the runtimes, delete-everything purges them", async () => {
    render(<GeneralPage />);
    await waitFor(() => expect(status).toHaveBeenCalled());

    await userEvent.click(screen.getByRole("button", { name: /Rebuild|重建環境/ }));
    expect(reset).toHaveBeenCalledWith(false);
  });

  it("explains that the old pip --user packages are no longer used", async () => {
    render(<GeneralPage />);
    await waitFor(() => expect(screen.getByText(/pip --user/)).toBeTruthy());
  });

  it("shows the bundled source when no interpreter was chosen", async () => {
    render(<GeneralPage />);
    await waitFor(() => expect(screen.getByText(/Bundled|內建/)).toBeTruthy());
  });
});
