import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { GeneralPage } from "./GeneralPage";

const status = vi.fn();
const reset = vi.fn();
const setInterpreter = vi.fn();
const setIndexUrl = vi.fn();

vi.mock("../../ipc/pythonEnv", () => ({
  pythonEnvStatus: () => status(),
  pythonEnvReset: (purge: boolean) => reset(purge),
  pythonEnvSetInterpreter: (p: string | null) => setInterpreter(p),
  pythonEnvSetIndexUrl: (url: string | null) => setIndexUrl(url),
}));

const pickFile = vi.fn();
const askConfirm = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: (...args: unknown[]) => pickFile(...args),
  // Not window.confirm: Tauri's webview never shows the JS dialog, so a purge
  // guarded by it runs unasked. jsdom's returns falsy, which is why this suite
  // passed while production purged without a prompt.
  confirm: (...args: unknown[]) => askConfirm(...args),
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
      indexUrl: null,
    });
  });

  it("shows the resolved version and venv path", async () => {
    render(<GeneralPage />);
    await waitFor(() => expect(screen.getByText(/3\.12\.13/)).toBeTruthy());
    expect(screen.getByText("/data/python-env")).toBeTruthy();
  });

  it("rebuild keeps the runtimes, delete-everything purges them", async () => {
    render(<GeneralPage />);
    await userEvent.click(
      await screen.findByRole("button", { name: /Clear environment|清除環境/ }),
    );
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

  it("lets the user point at their own interpreter", async () => {
    pickFile.mockResolvedValue("/usr/local/bin/python3.12");
    render(<GeneralPage />);
    await userEvent.click(
      await screen.findByRole("button", { name: /own Python|使用自己的/ }),
    );

    expect(setInterpreter).toHaveBeenCalledWith("/usr/local/bin/python3.12");
  });

  it("switches back to the bundled interpreter", async () => {
    status.mockResolvedValue({
      uvAvailable: true,
      pythonVersion: "3.12.13",
      installed: ["doc_core"],
      venvPath: "/data/python-env",
      userInterpreter: "/usr/local/bin/python3.12",
    });
    render(<GeneralPage />);
    await userEvent.click(await screen.findByRole("button", { name: /bundled|內建/ }));

    expect(setInterpreter).toHaveBeenCalledWith(null);
  });

  it("only purges after the user confirms", async () => {
    askConfirm.mockResolvedValue(false);
    render(<GeneralPage />);
    await userEvent.click(
      await screen.findByRole("button", { name: /Delete everything|完全刪除/ }),
    );
    expect(askConfirm).toHaveBeenCalled();
    expect(reset).not.toHaveBeenCalled();

    askConfirm.mockResolvedValue(true);
    await userEvent.click(screen.getByRole("button", { name: /Delete everything|完全刪除/ }));
    expect(reset).toHaveBeenCalledWith(true);
  });

  it("surfaces a failure instead of doing nothing visible", async () => {
    // The whole point of this section is being the way out when something broke;
    // a silent failure sends the user back to advice that no longer works.
    reset.mockRejectedValue("刪除 /data/python-env 失敗：Permission denied");
    render(<GeneralPage />);
    await userEvent.click(
      await screen.findByRole("button", { name: /Clear environment|清除環境/ }),
    );

    await waitFor(() => expect(screen.getByText(/Permission denied/)).toBeTruthy());
  });

  it("prefills the Index URL field from the persisted config", async () => {
    status.mockResolvedValue({
      uvAvailable: true,
      pythonVersion: "3.12.13",
      installed: ["doc_core"],
      venvPath: "/data/python-env",
      userInterpreter: null,
      indexUrl: "https://pypi.mycompany.com/simple",
    });
    render(<GeneralPage />);

    await waitFor(() =>
      expect(screen.getByPlaceholderText(/pypi\.mycompany\.com/)).toHaveValue(
        "https://pypi.mycompany.com/simple",
      ),
    );
  });

  it("saves the index url as the user types it", async () => {
    render(<GeneralPage />);
    const input = await screen.findByPlaceholderText(/pypi\.mycompany\.com/);
    await userEvent.type(input, "https://x");

    expect(setIndexUrl).toHaveBeenLastCalledWith("https://x");
  });

  it("clears the index url (passes null) when the field is emptied", async () => {
    status.mockResolvedValue({
      uvAvailable: true,
      pythonVersion: "3.12.13",
      installed: ["doc_core"],
      venvPath: "/data/python-env",
      userInterpreter: null,
      indexUrl: "https://pypi.mycompany.com/simple",
    });
    render(<GeneralPage />);
    const input = await screen.findByPlaceholderText(/pypi\.mycompany\.com/);
    await waitFor(() => expect(input).toHaveValue("https://pypi.mycompany.com/simple"));

    await userEvent.clear(input);

    expect(setIndexUrl).toHaveBeenLastCalledWith(null);
  });
});
