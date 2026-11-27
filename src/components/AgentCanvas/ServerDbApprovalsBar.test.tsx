import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";

const role = vi.hoisted(() => ({ value: "admin" as "admin" | "member" }));
vi.mock("../../hooks/useIntakeRemote", () => ({ useIntakeRemote: () => ({ connected: true, online: true, addr: "x", name: "我", role: role.value, error: null }) }));
vi.mock("../../ipc/serverDb", () => ({ listServerDbApprovals: vi.fn(), decideServerDb: vi.fn() }));

import { decideServerDb, listServerDbApprovals, type ServerDbQuery } from "../../ipc/serverDb";
import { ServerDbApprovalsBar } from "./ServerDbApprovalsBar";

const q = (id: number, sql: string): ServerDbQuery => ({ id, card_id: "c1", agent_id: "a1", connection_id: "s1", connection_name: "正式機", sql, decision: "pending", started_at: 1, finished_at: null, rows: null, error: null, decided_by: null });

const mount = (active = true) =>
  render(
    <LocaleProvider>
      <ServerDbApprovalsBar projectId="remote:p1" agentName={(id) => (id === "a1" ? "工程師1" : "?")} active={active} />
    </LocaleProvider>,
  );

beforeEach(() => {
  role.value = "admin";
  vi.mocked(listServerDbApprovals).mockResolvedValue([q(7, "UPDATE users SET name = 'x'\nWHERE id = 1")]);
  vi.mocked(decideServerDb).mockResolvedValue(undefined);
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.clearAllMocks();
});

describe("ServerDbApprovalsBar", () => {
  it("沒有待核准時什麼都不畫；不在使用中的畫布也不去問 server", async () => {
    vi.mocked(listServerDbApprovals).mockResolvedValue([]);
    const { container } = mount();
    await waitFor(() => expect(listServerDbApprovals).toHaveBeenCalledWith("remote:p1"));
    expect(container).toBeEmptyDOMElement();
    cleanup();
    vi.mocked(listServerDbApprovals).mockClear();
    mount(false);
    await act(async () => {});
    expect(listServerDbApprovals).not.toHaveBeenCalled();
  });

  it("有待核准：顯示數量，按下去看完整 SQL 與是哪位 Agent、哪個連線；管理員可核准", async () => {
    mount();
    fireEvent.click(await screen.findByTestId("ac-db-approvals"));
    expect(screen.getByTestId("ac-db-approvals")).toHaveTextContent("1 筆待核准");
    const item = await screen.findByTestId("ac-db-approval");
    expect(item).toHaveTextContent("工程師1 · 正式機");
    expect(item.querySelector("pre")?.textContent).toBe("UPDATE users SET name = 'x'\nWHERE id = 1");
    vi.mocked(listServerDbApprovals).mockResolvedValue([]);
    fireEvent.click(screen.getByRole("button", { name: "核准執行" }));
    await waitFor(() => expect(decideServerDb).toHaveBeenCalledWith(7, true));
    await waitFor(() => expect(screen.queryByTestId("ac-db-approval")).toBeNull());
  });

  it("管理員可以拒絕；server 說已經決定過就顯示原因並刷新", async () => {
    mount();
    fireEvent.click(await screen.findByTestId("ac-db-approvals"));
    vi.mocked(decideServerDb).mockRejectedValueOnce("這筆已經決定過了，或不是等核准的查詢");
    fireEvent.click(await screen.findByRole("button", { name: "拒絕" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("已經決定過了");
    fireEvent.click(screen.getByRole("button", { name: "拒絕" }));
    await waitFor(() => expect(decideServerDb).toHaveBeenLastCalledWith(7, false));
  });

  it("成員：看得到內容但沒有核准／拒絕按鈕", async () => {
    role.value = "member";
    mount();
    fireEvent.click(await screen.findByTestId("ac-db-approvals"));
    expect(await screen.findByTestId("ac-db-approval")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "核准執行" })).toBeNull();
    expect(screen.queryByRole("button", { name: "拒絕" })).toBeNull();
    expect(screen.getByText(/只有管理員能決定/)).toBeInTheDocument();
  });

  it("每隔幾秒再問一次：新的待核准會自己出現", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.mocked(listServerDbApprovals).mockResolvedValue([]);
    const { container } = mount();
    await act(async () => {});
    expect(container).toBeEmptyDOMElement();
    vi.mocked(listServerDbApprovals).mockResolvedValue([q(8, "DELETE FROM t")]);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3100);
    });
    expect(await screen.findByTestId("ac-db-approvals")).toBeInTheDocument();
  });
});
