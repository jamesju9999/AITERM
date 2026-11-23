import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { ConnectionCard } from "./ConnectionCard";
import { validInviteCode, validServerAddr } from "../../lib/intakeServerConn";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { translations } from "../../lib/i18n";
import type { IntakeRemoteStatus } from "../../ipc/intake";

vi.mock("../../ipc/intake", async (orig) => ({
  ...(await orig<typeof import("../../ipc/intake")>()),
  intakeRemotePair: vi.fn(),
  intakeRemoteDisconnect: vi.fn(() => Promise.resolve()),
}));
import * as ipc from "../../ipc/intake";

const t = translations["zh-TW"];
const LOCAL: IntakeRemoteStatus = { connected: false, online: false, addr: "", name: "", role: null, error: null };
const GOOD_CODE = "ABCD-EFGH-JKMN-PQRS-TUVW-XY";

function show(status: IntakeRemoteStatus = LOCAL) {
  return render(<LocaleProvider><ConnectionCard status={status} /></LocaleProvider>);
}

beforeEach(() => vi.clearAllMocks());

describe("validation helpers", () => {
  it("accepts host, host:port and https:// forms and rejects the rest", () => {
    for (const ok of ["192.168.1.20:8443", "intake.example.com", "https://intake.example.com:8443/"]) expect(validServerAddr(ok), ok).toBe(true);
    for (const bad of ["", "host:abc", "a b", "host/path", ":8443", "host:99999"]) expect(validServerAddr(bad), bad).toBe(false);
  });
  it("accepts a 22-character invite code in the server alphabet, ignoring case, spaces and dashes", () => {
    expect(validInviteCode(GOOD_CODE)).toBe(true);
    expect(validInviteCode(" abcd-efgh-jkmn-pqrs-tuvw-xy ")).toBe(true);
    for (const bad of ["", "ABCD-EFGH", "ABCD-EFGH-JKMN-PQRS-TUVW-X0", "ABCD-EFGH-JKMN-PQRS-TUVW-XYZ"]) expect(validInviteCode(bad), bad).toBe(false);
  });
});

describe("ConnectionCard (not connected)", () => {
  it("does not call the backend until both fields are valid", () => {
    show();
    const connect = screen.getByRole("button", { name: t.intake_conn_connect }) as HTMLButtonElement;
    expect(connect.disabled).toBe(true);
    fireEvent.change(screen.getByLabelText(t.intake_conn_addr), { target: { value: "not a host" } });
    fireEvent.change(screen.getByLabelText(t.intake_conn_code), { target: { value: "short" } });
    expect(screen.getByText(t.intake_conn_addr_bad)).toBeTruthy();
    expect(screen.getByText(t.intake_conn_code_bad)).toBeTruthy();
    expect(connect.disabled).toBe(true);
    fireEvent.click(connect);
    expect(ipc.intakeRemotePair).not.toHaveBeenCalled();
  });

  it("pairs with the trimmed address and code, then clears the code", async () => {
    vi.mocked(ipc.intakeRemotePair).mockResolvedValue({ ...LOCAL, connected: true });
    show();
    fireEvent.change(screen.getByLabelText(t.intake_conn_addr), { target: { value: " 192.168.1.20:8443 " } });
    fireEvent.change(screen.getByLabelText(t.intake_conn_code), { target: { value: ` ${GOOD_CODE} ` } });
    fireEvent.click(screen.getByRole("button", { name: t.intake_conn_connect }));
    await waitFor(() => expect(ipc.intakeRemotePair).toHaveBeenCalledWith("192.168.1.20:8443", GOOD_CODE));
    await waitFor(() => expect((screen.getByLabelText(t.intake_conn_code) as HTMLInputElement).value).toBe(""));
  });

  it("shows the reason when pairing fails and lets the user retry", async () => {
    vi.mocked(ipc.intakeRemotePair).mockRejectedValue("配對失敗：邀請碼錯誤、已過期或已被使用");
    show();
    fireEvent.change(screen.getByLabelText(t.intake_conn_addr), { target: { value: "host:1" } });
    fireEvent.change(screen.getByLabelText(t.intake_conn_code), { target: { value: GOOD_CODE } });
    fireEvent.click(screen.getByRole("button", { name: t.intake_conn_connect }));
    expect((await screen.findByRole("alert")).textContent).toContain("邀請碼錯誤");
    expect((screen.getByRole("button", { name: t.intake_conn_connect }) as HTMLButtonElement).disabled).toBe(false);
  });
});

describe("ConnectionCard (connected)", () => {
  const CONNECTED: IntakeRemoteStatus = { connected: true, online: true, addr: "192.168.1.20:8443", name: "小華", role: "admin", error: null };

  it("shows the server, the user's name and role, and that local receiving is stopped", () => {
    show(CONNECTED);
    expect(screen.getByText("小華")).toBeTruthy();
    expect(screen.getByText(new RegExp(t.intake_conn_role_admin))).toBeTruthy();
    expect(screen.getByText(t.intake_conn_online)).toBeTruthy();
    expect(screen.getByText(t.intake_conn_local_stopped)).toBeTruthy();
  });

  it("explains an offline server and a server-side problem separately", () => {
    show({ ...CONNECTED, online: false, error: "你已被移除，請向管理員重新取得邀請" });
    expect(screen.getByText(t.intake_conn_offline)).toBeTruthy();
    expect(screen.getByRole("alert").textContent).toContain("你已被移除");
  });

  it("disconnects on request", async () => {
    show(CONNECTED);
    fireEvent.click(screen.getByRole("button", { name: t.intake_conn_disconnect }));
    await waitFor(() => expect(ipc.intakeRemoteDisconnect).toHaveBeenCalledTimes(1));
  });
});
