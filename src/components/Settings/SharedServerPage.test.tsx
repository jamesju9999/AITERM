import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";

const remote = vi.hoisted(() => ({
  s: { connected: false, online: false, addr: "", name: "", role: null as null | "admin", error: null as null | string },
}));
vi.mock("../../hooks/useIntakeRemote", () => ({ useIntakeRemote: () => remote.s, useIntakeConnections: () => [] }));
vi.mock("../../ipc/intake", () => ({ intakeRemotePair: vi.fn(), intakeRemoteDisconnect: vi.fn() }));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { SharedServerPage } from "./SharedServerPage";

describe("設定：共用 server", () => {
  it("沒連線：出現位址與邀請碼欄位", () => {
    render(<LocaleProvider><SharedServerPage /></LocaleProvider>);
    expect(screen.getByRole("heading", { name: "共用 server" })).toBeInTheDocument();
    expect(screen.getByLabelText("邀請碼")).toBeInTheDocument();
  });

  it("已連線：顯示 server 名稱與解除連線，不再要邀請碼", () => {
    remote.s = { connected: true, online: true, addr: "10.0.0.2:8443", name: "小明", role: "admin", error: null };
    render(<LocaleProvider><SharedServerPage /></LocaleProvider>);
    expect(screen.getByText("小明")).toBeInTheDocument();
    expect(screen.queryByLabelText("邀請碼")).toBeNull();
  });
});
