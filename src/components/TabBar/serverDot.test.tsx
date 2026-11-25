import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const navigateMock = vi.fn();
vi.mock("react-router-dom", async () => {
  const actual = await vi.importActual<typeof import("react-router-dom")>("react-router-dom");
  return { ...actual, useNavigate: () => navigateMock };
});
const remote = vi.hoisted(() => ({
  s: { connected: false, online: false, addr: "", name: "", role: null as null | "admin", error: null as null | string },
}));
vi.mock("../../hooks/useIntakeRemote", () => ({ useIntakeRemote: () => remote.s }));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { TabBar } from "./index";

function renderBar() {
  return render(
    <LocaleProvider>
      <TabBar tabs={[]} activeId="" onSelect={() => {}} onClose={() => {}} onAdd={() => {}} onRename={() => {}} onReorder={() => {}} isSidebarOpen onToggle={() => {}} width={220} />
    </LocaleProvider>,
  );
}

describe("側欄底部的共用 server 狀態", () => {
  beforeEach(() => {
    navigateMock.mockReset();
    remote.s = { connected: false, online: false, addr: "", name: "", role: null, error: null };
  });

  it.each([
    ["未連線", { connected: false }, "off", "未連線"],
    ["在線", { connected: true, online: true, addr: "10.0.0.2:8443" }, "on", "已連線"],
    ["離線", { connected: true, online: false, addr: "10.0.0.2:8443" }, "bad", "離線"],
    ["需要處理", { connected: true, online: true, addr: "10.0.0.2:8443", error: "憑證改變" }, "bad", "需要處理"],
  ])("%s：點的顏色與提示", (_n, patch, tone, tip) => {
    remote.s = { ...remote.s, ...patch };
    renderBar();
    const item = screen.getByTestId("tabbar-server");
    expect(item.querySelector(".server-dot")).toHaveAttribute("data-tone", tone);
    expect(item.getAttribute("title")).toContain(tip);
  });

  it("點下去直接到設定的共用 server 頁", async () => {
    renderBar();
    await userEvent.click(screen.getByTestId("tabbar-server"));
    expect(navigateMock).toHaveBeenCalledWith("/settings", { state: { tab: "sharedServer" } });
  });
});
