import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { AppImageIntegrationPrompt } from "./AppImageIntegrationPrompt";

const DEFAULTS: Record<string, unknown> = {
  appimage_integration_state: { state: "available" },
  is_appimage_integration_declined: false,
  is_onboarding_done: true,
};

function mockCommands(overrides: Record<string, unknown> = {}) {
  const table = { ...DEFAULTS, ...overrides };
  invokeMock.mockImplementation((cmd: string) =>
    Promise.resolve(cmd in table ? table[cmd] : null),
  );
}

beforeEach(() => { invokeMock.mockReset(); });

describe("AppImageIntegrationPrompt", () => {
  it("offers the entry when running an un-integrated AppImage", async () => {
    mockCommands();
    render(<AppImageIntegrationPrompt hasUpdate={false} />);

    expect(await screen.findByText("建立應用程式選單項目？")).toBeInTheDocument();
  });

  it("stays hidden on a non-AppImage install", async () => {
    mockCommands({ appimage_integration_state: { state: "not_appimage" } });
    render(<AppImageIntegrationPrompt hasUpdate={false} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByText("建立應用程式選單項目？")).not.toBeInTheDocument();
  });

  it("stays hidden once already integrated", async () => {
    mockCommands({
      appimage_integration_state: { state: "integrated", exec_path: "/x/A.AppImage" },
    });
    render(<AppImageIntegrationPrompt hasUpdate={false} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByText("建立應用程式選單項目？")).not.toBeInTheDocument();
  });

  it("stays hidden after the user declined once", async () => {
    mockCommands({ is_appimage_integration_declined: true });
    render(<AppImageIntegrationPrompt hasUpdate={false} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByText("建立應用程式選單項目？")).not.toBeInTheDocument();
  });

  it("stays hidden until onboarding is done", async () => {
    // A brand-new user already has the onboarding wizard on screen.
    mockCommands({ is_onboarding_done: false });
    render(<AppImageIntegrationPrompt hasUpdate={false} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByText("建立應用程式選單項目？")).not.toBeInTheDocument();
  });

  it("yields to the update prompt", async () => {
    // Both live in the bottom-right corner; the update matters more.
    mockCommands();
    render(<AppImageIntegrationPrompt hasUpdate />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByText("建立應用程式選單項目？")).not.toBeInTheDocument();
  });

  it("installs the entry and closes when accepted", async () => {
    mockCommands();
    render(<AppImageIntegrationPrompt hasUpdate={false} />);

    await userEvent.click(await screen.findByRole("button", { name: "建立" }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("appimage_integrate"));
    await waitFor(() =>
      expect(screen.queryByText("建立應用程式選單項目？")).not.toBeInTheDocument(),
    );
  });

  it("records the refusal so it is not asked again", async () => {
    mockCommands();
    render(<AppImageIntegrationPrompt hasUpdate={false} />);

    await userEvent.click(await screen.findByRole("button", { name: "不用了" }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("set_appimage_integration_declined"),
    );
  });
});
