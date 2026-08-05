import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";

const navigateMock = vi.fn();
vi.mock("react-router-dom", async () => {
  const actual = await vi.importActual<typeof import("react-router-dom")>("react-router-dom");
  return { ...actual, useNavigate: () => navigateMock };
});

import { LocaleProvider } from "../../contexts/LocaleContext";
import { TabBar, type Tab, type TabBarProps } from "./index";

const baseTabs: Tab[] = [{ id: "t1", title: "Tab 1", type: "terminal" }];

function renderTabBar(overrides: Partial<TabBarProps> = {}) {
  return render(
    <LocaleProvider>
      <TabBar
        tabs={baseTabs}
        activeId="t1"
        onSelect={() => {}}
        onClose={() => {}}
        onAdd={() => {}}
        isSidebarOpen={false}
        onToggle={() => {}}
        width={260}
        {...overrides}
      />
    </LocaleProvider>,
  );
}

describe("TabBar settings navigation — collapsed gear icon", () => {
  beforeEach(() => {
    navigateMock.mockReset();
  });

  it("navigates to the about tab when hasUpdate is true", () => {
    renderTabBar({ isSidebarOpen: false, hasUpdate: true });
    fireEvent.click(screen.getByTitle(/Ctrl\+,/));
    expect(navigateMock).toHaveBeenCalledWith("/settings", { state: { tab: "about" } });
  });

  it("navigates to settings without a tab override when hasUpdate is false", () => {
    renderTabBar({ isSidebarOpen: false, hasUpdate: false });
    fireEvent.click(screen.getByTitle(/Ctrl\+,/));
    expect(navigateMock).toHaveBeenCalledWith("/settings", undefined);
  });
});

describe("TabBar settings navigation — expanded footer item", () => {
  beforeEach(() => {
    navigateMock.mockReset();
  });

  it("navigates to the about tab when hasUpdate is true", () => {
    renderTabBar({ isSidebarOpen: true, hasUpdate: true });
    fireEvent.click(screen.getByTitle(/Ctrl\+,/));
    expect(navigateMock).toHaveBeenCalledWith("/settings", { state: { tab: "about" } });
  });

  it("navigates to settings without a tab override when hasUpdate is false", () => {
    renderTabBar({ isSidebarOpen: true, hasUpdate: false });
    fireEvent.click(screen.getByTitle(/Ctrl\+,/));
    expect(navigateMock).toHaveBeenCalledWith("/settings", undefined);
  });
});

// LocaleProvider defaults to zh-TW, so these assert the zh-TW accessible names.
describe("TabBar mail connection-failure indicator", () => {
  const mailTabs: Tab[] = [{ id: "m1", title: "Mail", type: "mail" }];

  it("marks the mail icon when an account has stopped connecting", () => {
    renderTabBar({ tabs: mailTabs, activeId: "m1", mailFailedAccountCount: 1 });
    expect(screen.getByRole("img", { name: "1 個信箱帳號連線失敗" })).toBeTruthy();
  });

  it("shows nothing while every account is healthy", () => {
    renderTabBar({ tabs: mailTabs, activeId: "m1", mailFailedAccountCount: 0 });
    expect(screen.queryByRole("img", { name: /連線失敗/ })).toBeNull();
  });

  it("does not mark a non-mail tab", () => {
    renderTabBar({ tabs: baseTabs, activeId: "t1", mailFailedAccountCount: 1 });
    expect(screen.queryByRole("img", { name: /連線失敗/ })).toBeNull();
  });

  // The two are different kinds of signal, so they must stay tellable apart —
  // both by a screen reader (distinct accessible names) and visually (distinct
  // classes, which anchor them to opposite corners of the icon).
  it("stays distinguishable from the unread badge when both are showing", () => {
    renderTabBar({ tabs: mailTabs, activeId: "m1", mailUnreadCount: 3, mailFailedAccountCount: 2 });

    // Both queries throw if their element is missing, so reaching the last
    // assertion already proves the two coexist under distinct accessible names.
    const failure = screen.getByRole("img", { name: "2 個信箱帳號連線失敗" });
    const unread = screen.getByRole("img", { name: "3 封未讀郵件" });
    expect(failure.className).not.toBe(unread.className);
  });
});
