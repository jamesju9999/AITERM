import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";

const remote = vi.hoisted(() => ({
  list: [] as Array<{ id: string; addr: string; name: string; role: "admin" | "member"; active: boolean }>,
  s: { connected: false, online: false, addr: "", name: "", role: null as null | "admin", error: null as null | string },
}));
vi.mock("../../hooks/useIntakeRemote", () => ({ useIntakeRemote: () => remote.s, useIntakeConnections: () => remote.list }));
vi.mock("../../ipc/intake", () => ({ intakeRemotePair: vi.fn(), intakeRemoteDisconnect: vi.fn() }));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { SharedServerPage } from "./SharedServerPage";

describe("設定：共用 server", () => {
  it("沒連線：出現位址與邀請碼欄位", () => {
    render(<LocaleProvider><SharedServerPage /></LocaleProvider>);
    expect(screen.getByRole("heading", { name: "共用 server" })).toBeInTheDocument();
    expect(screen.getByLabelText("邀請碼")).toBeInTheDocument();
  });

  it("連著的那組已在清單裡：只顯示一張卡片（狀態與中斷連線併在清單那一列），不再重複顯示連線卡片", () => {
    remote.s = { connected: true, online: true, addr: "10.0.0.2:8443", name: "小明", role: "admin", error: null };
    remote.list = [{ id: "a", addr: "10.0.0.2:8443", name: "小明", role: "admin", active: true }];
    render(<LocaleProvider><SharedServerPage /></LocaleProvider>);
    expect(screen.queryByRole("heading", { name: "連到 server" })).toBeNull();
    expect(screen.getAllByText("小明")).toHaveLength(1);
    expect(screen.getByRole("button", { name: "中斷連線，切回本機" })).toBeInTheDocument();
  });

  it("已連線：顯示 server 名稱與解除連線，不再要邀請碼", () => {
    remote.s = { connected: true, online: true, addr: "10.0.0.2:8443", name: "小明", role: "admin", error: null };
    render(<LocaleProvider><SharedServerPage /></LocaleProvider>);
    expect(screen.getByText("小明")).toBeInTheDocument();
    expect(screen.queryByLabelText("邀請碼")).toBeNull();
  });
});
