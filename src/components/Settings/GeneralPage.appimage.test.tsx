import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { GeneralPage } from "./GeneralPage";

/**
 * GeneralPage calls getConfig() on mount and reads cfg.execution_mode straight
 * off the result, so an unmocked get_config throws inside that promise and the
 * failure looks like it came from the section under test. Every case gets a
 * valid config by default.
 */
const BASE_CONFIG = {
  execution_mode: "graded",
  submit_shortcut: "enter",
  max_agent_steps: 5,
  default_tab: "terminal",
};

function mockCommands(table: Record<string, unknown>) {
  const full = {
    get_config: BASE_CONFIG,
    // GeneralPage also fetches this on mount. Leaving it unstubbed makes
    // every run emit an unhandled rejection, which is exactly the noise
    // that hides a real one later.
    telegram_get_config: { bot_token: null, chat_id: null },
    ...table,
  };
  invokeMock.mockImplementation((cmd: string) =>
    Promise.resolve(cmd in full ? full[cmd] : null),
  );
}

beforeEach(() => { invokeMock.mockReset(); });

describe("GeneralPage — AppImage integration", () => {
  it("hides the section entirely off AppImage", async () => {
    mockCommands({ appimage_integration_state: { state: "not_appimage" } });
    render(<GeneralPage />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByText("應用程式選單項目")).not.toBeInTheDocument();
  });

  it("offers to create when not yet integrated", async () => {
    mockCommands({ appimage_integration_state: { state: "available" } });
    render(<GeneralPage />);

    expect(await screen.findByText("應用程式選單項目")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "建立" }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("appimage_integrate"));
  });

  it("shows the current path and offers removal when integrated", async () => {
    mockCommands({
      appimage_integration_state: { state: "integrated", exec_path: "/home/u/AITerm.AppImage" },
    });
    render(<GeneralPage />);

    expect(await screen.findByText(/\/home\/u\/AITerm\.AppImage/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "移除選單項目" }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("appimage_remove_integration"),
    );
  });
});
