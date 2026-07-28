import { render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MemoryRouter } from "react-router-dom";

const terminalProps: Record<string, unknown>[] = [];
vi.mock("./components/TerminalApp", () => ({
  TerminalApp: (props: Record<string, unknown>) => {
    terminalProps.push(props);
    return <div data-testid="terminal" />;
  },
}));
vi.mock("./components/Settings/SettingsView", () => ({ SettingsView: () => null }));
vi.mock("./components/Onboarding/OnboardingWizard", () => ({ OnboardingWizard: () => null }));
vi.mock("./ipc/config", () => ({ isOnboardingDone: () => Promise.resolve(true) }));

const checkMock = vi.fn();
const invokeMock = vi.fn();
vi.mock("@tauri-apps/plugin-updater", () => ({
  check: (...args: unknown[]) => checkMock(...args),
}));
vi.mock("@tauri-apps/plugin-process", () => ({
  relaunch: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { AppRoutes } from "./App";
import { UpdaterProvider } from "./contexts/UpdaterContext";

beforeEach(() => {
  terminalProps.length = 0;
  checkMock.mockReset();
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(true); // updater_supported
});

describe("App hasUpdate wiring", () => {
  it("passes hasUpdate=false to TerminalApp when no update is pending", async () => {
    checkMock.mockResolvedValue(null);
    render(
      <MemoryRouter initialEntries={["/"]}>
        <UpdaterProvider>
          <AppRoutes />
        </UpdaterProvider>
      </MemoryRouter>,
    );

    await waitFor(() => expect(terminalProps.length).toBeGreaterThan(0));
    expect(terminalProps[terminalProps.length - 1].hasUpdate).toBe(false);
  });

  it("passes hasUpdate=true to TerminalApp once an update is found", async () => {
    checkMock.mockResolvedValue({
      version: "9.9.9",
      body: "notes",
      downloadAndInstall: vi.fn(),
    });
    render(
      <MemoryRouter initialEntries={["/"]}>
        <UpdaterProvider>
          <AppRoutes />
        </UpdaterProvider>
      </MemoryRouter>,
    );

    await waitFor(() =>
      expect(terminalProps.some((p) => p.hasUpdate === true)).toBe(true),
    );
  });
});
