import { render, screen, waitFor, fireEvent, act } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { IntakeToggle } from "./index";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { translations } from "../../lib/i18n";

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
vi.mock("../../ipc/fs", () => ({ getSessionCwd: vi.fn(() => Promise.resolve("/work/two/src")) }));
import { getSessionCwd } from "../../ipc/fs";
vi.mock("../../ipc/intake", async (orig) => ({
  ...(await orig<typeof import("../../ipc/intake")>()),
  intakeTabs: vi.fn(),
  intakeProjects: vi.fn(),
  intakeTabStart: vi.fn(() => Promise.resolve()),
  intakeTabStop: vi.fn(() => Promise.resolve()),
  intakeTabCd: vi.fn(() => Promise.resolve("/work/one")),
}));
import * as ipc from "../../ipc/intake";

const t = translations["zh-TW"];
const wrap = () => render(<LocaleProvider><IntakeToggle sessionId="pty-1" /></LocaleProvider>);
const proj = (id: string, name: string, repo: string | null) => ({ id, name, repo_dir: repo, open_count: 0, total: 0, enabled: true });
const ONE = [proj("p1", "專案一", "/work/one")];
const TWO = [proj("p1", "專案一", "/work/one"), proj("p2", "專案二", "/work/two")];

beforeEach(() => {
  vi.clearAllMocks();
  // 上一個測試把它改成「已在專案目錄裡」，不重設會漏到下一個測試。
  vi.mocked(getSessionCwd).mockResolvedValue("/work/two/src");
  vi.mocked(ipc.intakeTabCd).mockResolvedValue("/work/one");
  vi.mocked(ipc.intakeTabs).mockResolvedValue([]);
  vi.mocked(ipc.intakeProjects).mockResolvedValue({ projects: TWO, unassigned: 0, legacy_settings: false });
});

describe("IntakeToggle", () => {
  it("with one project it starts straight away for THIS pty session and that project", async () => {
    vi.mocked(ipc.intakeProjects).mockResolvedValue({ projects: ONE, unassigned: 0, legacy_settings: false });
    vi.mocked(ipc.intakeTabs).mockResolvedValueOnce([]).mockResolvedValue([{ tab_id: "pty-1", project_id: "p1", claude_ready: true }]);
    wrap();
    fireEvent.click(await screen.findByText(t.intake_toggle_off));
    await waitFor(() => expect(ipc.intakeTabStart).toHaveBeenCalledWith("pty-1", "p1"));
    expect(await screen.findByText(`${t.intake_toggle_on}：專案一`)).toBeInTheDocument();
  });

  it("with several projects it asks which one, defaulting to the project that contains the tab's directory", async () => {
    wrap();
    fireEvent.click(await screen.findByText(t.intake_toggle_off));
    const select = (await screen.findByLabelText(t.intake_toggle_pick)) as HTMLSelectElement;
    expect(select.value).toBe("p2"); // 分頁在 /work/two/src
    expect(ipc.intakeTabStart).not.toHaveBeenCalled();
    fireEvent.change(select, { target: { value: "p1" } });
    // /work/two/src 不在專案一（/work/one）底下：警告要寫出專案目錄，使用者才看得出差在哪。
    expect(screen.getByText(t.intake_toggle_outside.replace("{dir}", "/work/one"))).toBeInTheDocument();
    fireEvent.click(screen.getByText(t.intake_toggle_start));
    await waitFor(() => expect(ipc.intakeTabStart).toHaveBeenCalledWith("pty-1", "p1"));
  });

  it("the switch button sends the project (never a path) and the warning disappears once the tab is inside it", async () => {
    wrap();
    fireEvent.click(await screen.findByText(t.intake_toggle_off));
    fireEvent.change(await screen.findByLabelText(t.intake_toggle_pick), { target: { value: "p1" } });
    expect(screen.getByText(t.intake_toggle_outside.replace("{dir}", "/work/one"))).toBeInTheDocument();
    vi.mocked(getSessionCwd).mockResolvedValue("/work/one");
    fireEvent.click(screen.getByText(t.intake_toggle_cd));
    await waitFor(() => expect(ipc.intakeTabCd).toHaveBeenCalledWith("pty-1", "p1"));
    await waitFor(() => expect(screen.queryByText(t.intake_toggle_outside.replace("{dir}", "/work/one"))).toBeNull());
    expect(screen.queryByText(t.intake_toggle_cd)).toBeNull();
  });

  it("a refused switch (a program is running) is shown and the warning stays", async () => {
    vi.mocked(ipc.intakeTabCd).mockRejectedValueOnce(new Error("這個分頁正在執行程式，請先結束它再切換"));
    wrap();
    fireEvent.click(await screen.findByText(t.intake_toggle_off));
    fireEvent.change(await screen.findByLabelText(t.intake_toggle_pick), { target: { value: "p1" } });
    fireEvent.click(screen.getByText(t.intake_toggle_cd));
    expect(await screen.findByRole("alert")).toHaveTextContent("正在執行程式");
    expect(screen.getByText(t.intake_toggle_outside.replace("{dir}", "/work/one"))).toBeInTheDocument();
  });

  it("cancelling the picker starts nothing", async () => {
    wrap();
    fireEvent.click(await screen.findByText(t.intake_toggle_off));
    fireEvent.click(await screen.findByText(t.intake_cancel));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(ipc.intakeTabStart).not.toHaveBeenCalled();
  });

  it("with no project it explains instead of starting", async () => {
    vi.mocked(ipc.intakeProjects).mockResolvedValue({ projects: [], unassigned: 0, legacy_settings: false });
    wrap();
    fireEvent.click(await screen.findByText(t.intake_toggle_off));
    expect(await screen.findByText(t.intake_no_projects)).toBeInTheDocument();
    expect(ipc.intakeTabStart).not.toHaveBeenCalled();
  });

  it("stops when already collecting; another tab being registered does not count", async () => {
    vi.mocked(ipc.intakeTabs).mockResolvedValue([
      { tab_id: "other", project_id: "p2", claude_ready: true },
      { tab_id: "pty-1", project_id: "p1", claude_ready: true },
    ]);
    wrap();
    fireEvent.click(await screen.findByText(`${t.intake_toggle_on}：專案一`));
    await waitFor(() => expect(ipc.intakeTabStop).toHaveBeenCalledWith("pty-1"));
    expect(ipc.intakeTabStart).not.toHaveBeenCalled();
  });

  it("when collecting but Claude Code is not detected in the tab, it says so instead of looking ready", async () => {
    vi.mocked(ipc.intakeTabs).mockResolvedValue([{ tab_id: "pty-1", project_id: "p1", claude_ready: false }]);
    wrap();
    const btn = await screen.findByText(`${t.intake_toggle_on}：專案一（${t.intake_toggle_waiting}）`);
    expect(btn.closest("button")).toHaveAttribute("title", t.intake_toggle_waiting_hint);
  });

  it("shows off when only a different tab is registered", async () => {
    vi.mocked(ipc.intakeTabs).mockResolvedValue([{ tab_id: "other", project_id: "p1", claude_ready: true }]);
    wrap();
    // 初始狀態本來就是「關」，不等 intakeTabs 結算就斷言會空過（變異測試抓到過）。
    await act(async () => {});
    expect(ipc.intakeTabs).toHaveBeenCalled();
    expect(screen.getByText(t.intake_toggle_off)).toBeInTheDocument();
    expect(screen.getByRole("button")).toHaveAttribute("aria-pressed", "false");
  });
});
