import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { UpdaterApi, UpdaterState } from "../../hooks/useUpdater";

const getVersionMock = vi.fn();
const openUrlMock = vi.fn();
const useUpdaterContextMock = vi.fn();

vi.mock("@tauri-apps/api/app", () => ({
  getVersion: (...args: unknown[]) => getVersionMock(...args),
}));
vi.mock("../../ipc/shell", () => ({
  openUrl: (...args: unknown[]) => openUrlMock(...args),
}));
vi.mock("../../contexts/UpdaterContext", () => ({
  useUpdaterContext: () => useUpdaterContextMock(),
}));

import { AboutPage } from "./AboutPage";
import { GITHUB_REPO_URL, GITHUB_RELEASES_URL } from "../../lib/repo";

/** Feeds a fixed updater state to the component without a real UpdaterProvider. */
function mockUpdater(state: UpdaterState, overrides: Partial<UpdaterApi> = {}): UpdaterApi {
  const api: UpdaterApi = {
    state,
    hasUpdate: false,
    dismissed: false,
    check: vi.fn(),
    install: vi.fn(),
    relaunch: vi.fn(),
    dismiss: vi.fn(),
    ...overrides,
  };
  useUpdaterContextMock.mockReturnValue(api);
  return api;
}

beforeEach(() => {
  getVersionMock.mockReset();
  getVersionMock.mockResolvedValue("1.2.3");
  openUrlMock.mockReset();
  openUrlMock.mockResolvedValue(undefined);
  useUpdaterContextMock.mockReset();
});

describe("AboutPage", () => {
  it("renders the version once getVersion resolves", async () => {
    mockUpdater({ status: "idle" });
    render(<AboutPage />);

    expect(await screen.findByText("v1.2.3")).toBeTruthy();
  });

  it("GitHub button opens the repo URL", async () => {
    mockUpdater({ status: "idle" });
    render(<AboutPage />);

    await userEvent.click(screen.getByRole("button", { name: "GitHub" }));

    expect(openUrlMock).toHaveBeenCalledWith(GITHUB_REPO_URL);
  });

  it("the unsupported branch's link opens the releases URL, not the repo URL", async () => {
    mockUpdater({ status: "unsupported", version: "1.3.0" });
    render(<AboutPage />);

    await userEvent.click(screen.getByRole("button", { name: "點此前往下載" }));

    expect(openUrlMock).toHaveBeenCalledWith(GITHUB_RELEASES_URL);
    expect(openUrlMock).not.toHaveBeenCalledWith(GITHUB_REPO_URL);
  });

  it("available renders the version and its button calls install", async () => {
    const api = mockUpdater({ status: "available", version: "1.3.0", notes: "" });
    render(<AboutPage />);

    expect(screen.getByText(/1\.3\.0/)).toBeTruthy();

    await userEvent.click(screen.getByRole("button", { name: "立即更新" }));
    expect(api.install).toHaveBeenCalledTimes(1);
  });

  it("ready renders and its button calls relaunch", async () => {
    const api = mockUpdater({ status: "ready", version: "1.3.0" });
    render(<AboutPage />);

    expect(screen.getByText(/1\.3\.0/)).toBeTruthy();

    await userEvent.click(screen.getByRole("button", { name: "重新啟動以完成更新" }));
    expect(api.relaunch).toHaveBeenCalledTimes(1);
  });

  const checkButtonCases: { label: string; state: UpdaterState; disabled: boolean }[] = [
    { label: "idle", state: { status: "idle" }, disabled: false },
    { label: "checking", state: { status: "checking" }, disabled: true },
    { label: "none", state: { status: "none" }, disabled: false },
    { label: "available", state: { status: "available", version: "1.3.0", notes: "" }, disabled: false },
    {
      label: "downloading",
      state: { status: "downloading", version: "1.3.0", downloaded: 0, total: null },
      disabled: true,
    },
    // The important case: a staged update must not be re-offered by a manual
    // check, so the button is disabled here even though nothing is in flight.
    { label: "ready", state: { status: "ready", version: "1.3.0" }, disabled: true },
    { label: "unsupported", state: { status: "unsupported", version: "1.3.0" }, disabled: false },
    { label: "error", state: { status: "error", message: "boom" }, disabled: false },
  ];

  it.each(checkButtonCases)(
    "check button disabled=$disabled when status is $label",
    ({ state, disabled }) => {
      mockUpdater(state);
      render(<AboutPage />);

      const button = screen.getByRole("button", { name: "檢查更新" });
      if (disabled) {
        expect(button).toBeDisabled();
      } else {
        expect(button).not.toBeDisabled();
      }
    },
  );
});
