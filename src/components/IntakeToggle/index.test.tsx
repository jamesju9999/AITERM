import { render, screen, waitFor, fireEvent, act } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { IntakeToggle } from "./index";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { translations } from "../../lib/i18n";

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
vi.mock("../../ipc/intake", async (orig) => ({
  ...(await orig<typeof import("../../ipc/intake")>()),
  intakeTabs: vi.fn(),
  intakeTabStart: vi.fn(() => Promise.resolve()),
  intakeTabStop: vi.fn(() => Promise.resolve()),
}));
import * as ipc from "../../ipc/intake";

const t = translations["zh-TW"];
const wrap = () => render(<LocaleProvider><IntakeToggle sessionId="pty-1" /></LocaleProvider>);

beforeEach(() => vi.clearAllMocks());

describe("IntakeToggle", () => {
  it("starts collecting for THIS pty session id when off", async () => {
    vi.mocked(ipc.intakeTabs).mockResolvedValueOnce([]).mockResolvedValue(["pty-1"]);
    wrap();
    fireEvent.click(await screen.findByText(t.intake_toggle_off));
    await waitFor(() => expect(ipc.intakeTabStart).toHaveBeenCalledWith("pty-1"));
    expect(await screen.findByText(t.intake_toggle_on)).toBeInTheDocument();
  });

  it("stops when already collecting; another tab being registered does not count", async () => {
    vi.mocked(ipc.intakeTabs).mockResolvedValue(["other", "pty-1"]);
    wrap();
    fireEvent.click(await screen.findByText(t.intake_toggle_on));
    await waitFor(() => expect(ipc.intakeTabStop).toHaveBeenCalledWith("pty-1"));
    expect(ipc.intakeTabStart).not.toHaveBeenCalled();
  });

  it("shows off when only a different tab is registered", async () => {
    vi.mocked(ipc.intakeTabs).mockResolvedValue(["other"]);
    wrap();
    // 初始狀態本來就是「關」，不等 intakeTabs 結算就斷言會空過（變異測試抓到過）。
    await act(async () => {});
    expect(ipc.intakeTabs).toHaveBeenCalled();
    expect(screen.getByText(t.intake_toggle_off)).toBeInTheDocument();
    expect(screen.getByRole("button")).toHaveAttribute("aria-pressed", "false");
  });
});
