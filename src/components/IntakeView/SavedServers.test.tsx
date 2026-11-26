import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { SavedServers } from "./SavedServers";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { translations } from "../../lib/i18n";
import type { IntakeRemoteConnection, IntakeRemoteStatus } from "../../ipc/intake";

const confirmMock = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: (...a: unknown[]) => confirmMock(...a) }));
vi.mock("../../ipc/intake", async (orig) => ({
  ...(await orig<typeof import("../../ipc/intake")>()),
  intakeRemoteConnections: vi.fn(),
  intakeRemoteSwitch: vi.fn(() => Promise.resolve({})),
  intakeRemoteForget: vi.fn(() => Promise.resolve()),
  intakeRemotePair: vi.fn(),
  onIntakeRemote: vi.fn(() => Promise.resolve(() => {})),
}));
import * as ipc from "../../ipc/intake";

const t = translations["zh-TW"];
const A: IntakeRemoteConnection = { id: "a", addr: "10.0.0.1:8443", name: "小明", role: "member", active: true };
const B: IntakeRemoteConnection = { id: "b", addr: "10.0.0.2:8443", name: "小華", role: "admin", active: false };
const CONNECTED: IntakeRemoteStatus = { id: "a", connected: true, online: true, addr: A.addr, name: A.name, role: "member", error: null };
const LOCAL: IntakeRemoteStatus = { id: "", connected: false, online: false, addr: "", name: "", role: null, error: null };

function show(status: IntakeRemoteStatus, list: IntakeRemoteConnection[]) {
  vi.mocked(ipc.intakeRemoteConnections).mockResolvedValue(list);
  return render(<LocaleProvider><SavedServers status={status} /></LocaleProvider>);
}

beforeEach(() => {
  vi.clearAllMocks();
  confirmMock.mockResolvedValue(true);
});

describe("SavedServers（已記錄的 server）", () => {
  it("沒有任何記錄就不佔畫面", async () => {
    const { container } = show(LOCAL, []);
    await waitFor(() => expect(ipc.intakeRemoteConnections).toHaveBeenCalled());
    expect(container.querySelector("[data-testid=saved-servers]")).toBeNull();
  });

  it("列出每一組；目前連著的標「目前連線」，其他的有「連線」按鈕", async () => {
    show(CONNECTED, [A, B]);
    const rowA = await screen.findByTestId("saved-server-a");
    const rowB = screen.getByTestId("saved-server-b");
    expect(rowA.textContent).toContain(t.intake_conn_current);
    expect(Array.from(rowA.querySelectorAll("button")).map((b) => b.textContent)).toEqual([t.intake_conn_forget]);
    expect(rowB.textContent).toContain("10.0.0.2:8443");
    expect(rowB.textContent).toContain(t.intake_conn_switch);
  });

  it("按「連線」就切換到那一組，不問邀請碼", async () => {
    show(CONNECTED, [A, B]);
    const rowB = await screen.findByTestId("saved-server-b");
    fireEvent.click(rowB.querySelector("button")!);
    await waitFor(() => expect(ipc.intakeRemoteSwitch).toHaveBeenCalledWith("b"));
    expect(ipc.intakeRemotePair).not.toHaveBeenCalled();
  });

  it("後端拒絕切換（有領來的工作在處理）時把原因顯示出來", async () => {
    vi.mocked(ipc.intakeRemoteSwitch).mockRejectedValue("正在處理從 server 領來的工作，請先完成或放回 server，再切換連線。");
    show(CONNECTED, [A, B]);
    const rowB = await screen.findByTestId("saved-server-b");
    fireEvent.click(rowB.querySelector("button")!);
    expect((await screen.findByRole("alert")).textContent).toContain("正在處理從 server 領來的工作");
  });

  it("刪除要先確認；取消就不刪，確定才呼叫後端", async () => {
    show(CONNECTED, [A, B]);
    const rowB = await screen.findByTestId("saved-server-b");
    const del = Array.from(rowB.querySelectorAll("button")).find((b) => b.textContent === t.intake_conn_forget)!;
    confirmMock.mockResolvedValueOnce(false);
    fireEvent.click(del);
    await waitFor(() => expect(confirmMock).toHaveBeenCalledTimes(1));
    expect(ipc.intakeRemoteForget).not.toHaveBeenCalled();
    fireEvent.click(del);
    await waitFor(() => expect(ipc.intakeRemoteForget).toHaveBeenCalledWith("b"));
    expect(confirmMock.mock.calls[1][0]).toContain("小華 · 10.0.0.2:8443");
  });

  it("連線中才有「新增 server」；按下去出現位址與邀請碼表單", async () => {
    show(CONNECTED, [A]);
    fireEvent.click(await screen.findByRole("button", { name: t.intake_conn_add }));
    expect(screen.getByLabelText(t.intake_conn_addr)).toBeTruthy();
    expect(screen.getByLabelText(t.intake_conn_code)).toBeTruthy();
    // 按鈕在標題列，展開後變成「取消新增」，再按就收起。
    fireEvent.click(screen.getByRole("button", { name: t.intake_conn_add_cancel }));
    expect(screen.queryByLabelText(t.intake_conn_addr)).toBeNull();
  });

  it("本機模式（已中斷但有記錄）可以直接按「連線」回去，且不顯示新增（上面的卡片就是表單）", async () => {
    show(LOCAL, [{ ...A, active: false }, B]);
    await screen.findByTestId("saved-server-a");
    expect(screen.queryByRole("button", { name: t.intake_conn_add })).toBeNull();
    expect(screen.getAllByRole("button", { name: t.intake_conn_switch })).toHaveLength(2);
  });
});
