import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { UpdateModalView } from "./UpdateModal";
import type { UpdaterState } from "../hooks/useUpdater";

function renderView(state: UpdaterState, overrides: Record<string, unknown> = {}) {
  const props = {
    state,
    dismissed: false,
    onInstall: vi.fn(),
    onDismiss: vi.fn(),
    onRelaunch: vi.fn(),
    onOpenReleases: vi.fn(),
    ...overrides,
  };
  render(<UpdateModalView {...props} />);
  return props;
}

describe("UpdateModalView", () => {
  it("renders nothing while idle", () => {
    const { container } = render(
      <UpdateModalView
        state={{ status: "idle" }}
        dismissed={false}
        onInstall={vi.fn()}
        onDismiss={vi.fn()}
        onRelaunch={vi.fn()}
        onOpenReleases={vi.fn()}
      />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("renders nothing once dismissed", () => {
    const { container } = render(
      <UpdateModalView
        state={{ status: "available", version: "1.2.0", notes: "" }}
        dismissed
        onInstall={vi.fn()}
        onDismiss={vi.fn()}
        onRelaunch={vi.fn()}
        onOpenReleases={vi.fn()}
      />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("shows the version, notes and an update button when available", async () => {
    const props = renderView({ status: "available", version: "1.2.0", notes: "Bug fixes" });

    expect(screen.getByText(/1\.2\.0/)).toBeTruthy();
    expect(screen.getByText("Bug fixes")).toBeTruthy();

    await userEvent.click(screen.getByRole("button", { name: "立即更新" }));
    expect(props.onInstall).toHaveBeenCalledTimes(1);
  });

  it("shows a percentage when the total size is known", () => {
    renderView({ status: "downloading", version: "1.2.0", downloaded: 500, total: 1000 });
    // Regex, not an exact string: the label and the percentage are separate text
    // nodes inside one <p>, so its textContent is "下載中… 50%".
    expect(screen.getByText(/50%/)).toBeTruthy();
  });

  it("omits the percentage when the total size is unknown", () => {
    renderView({ status: "downloading", version: "1.2.0", downloaded: 500, total: null });
    expect(screen.queryByText(/%/)).toBeNull();
  });

  it("clamps the percentage at 100 if the download overruns the reported size", () => {
    // chunkLength sums can exceed a content-encoded response's declared length.
    renderView({ status: "downloading", version: "1.2.0", downloaded: 1500, total: 1000 });
    expect(screen.getByText(/100%/)).toBeTruthy();
  });

  it("hides the dismiss button while downloading", () => {
    // Dismissing mid-download would persist past Finished — the hook only resets
    // `dismissed` in runCheck — leaving the user with no restart prompt.
    renderView({ status: "downloading", version: "1.2.0", downloaded: 500, total: 1000 });
    expect(screen.queryByRole("button", { name: "稍後" })).toBeNull();
  });

  it("warns about losing terminal sessions before restarting", async () => {
    const props = renderView({ status: "ready", version: "1.2.0" });

    // The title must switch: a restart prompt headed "an update is available"
    // is stale, and the accessible name must not contradict the visible one.
    expect(screen.getByText("更新已下載完成")).toBeTruthy();
    expect(screen.queryByText("有新版本可用")).toBeNull();
    expect(screen.getByRole("status", { name: "更新已下載完成" })).toBeTruthy();

    expect(screen.getByText(/重新啟動將結束所有終端機分頁與執行中的指令。/)).toBeTruthy();

    await userEvent.click(screen.getByRole("button", { name: "重新啟動以完成更新" }));
    expect(props.onRelaunch).toHaveBeenCalledTimes(1);
  });

  it("offers a manual download link on unsupported installs", async () => {
    const props = renderView({ status: "unsupported", version: "1.2.0" });

    expect(screen.getByText(/此安裝方式不支援自動更新/)).toBeTruthy();

    await userEvent.click(screen.getByRole("button", { name: "點此前往下載" }));
    expect(props.onOpenReleases).toHaveBeenCalledTimes(1);
    expect(props.onInstall).not.toHaveBeenCalled();
  });

  it("shows the failure message on error", () => {
    renderView({ status: "error", message: "signature mismatch" });
    expect(screen.getByText(/signature mismatch/)).toBeTruthy();
  });
});
