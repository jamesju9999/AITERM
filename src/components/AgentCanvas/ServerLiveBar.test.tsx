import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";

vi.mock("../../ipc/serverLive", () => ({
  listLiveRuns: vi.fn(),
  openLive: vi.fn(async () => ({ send: vi.fn(), close: vi.fn() })),
  cancelRemoteLogin: vi.fn(),
}));

import { listLiveRuns, openLive } from "../../ipc/serverLive";
import { ServerLiveBar } from "./ServerLiveBar";

const run = (card: string, over = {}) => ({ card_id: card, system_id: "s1", system_name: "ERP", agent_id: "a1", manual: false, viewers: 0, ...over });

const mount = (active = true) =>
  render(
    <LocaleProvider>
      <ServerLiveBar projectId="remote:p1" agentName={(id) => (id === "a1" ? "工程師1" : "?")} active={active} />
    </LocaleProvider>,
  );

beforeEach(() => {
  vi.mocked(listLiveRuns).mockResolvedValue([run("c1"), run("c2", { manual: true })]);
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("ServerLiveBar", () => {
  it("沒有在跑的瀏覽器時什麼都不畫；不在使用中的畫布也不去問 server", async () => {
    vi.mocked(listLiveRuns).mockResolvedValue([]);
    const { container } = mount();
    await waitFor(() => expect(listLiveRuns).toHaveBeenCalledWith("remote:p1"));
    expect(container).toBeEmptyDOMElement();
    cleanup();
    vi.mocked(listLiveRuns).mockClear();
    mount(false);
    await new Promise((r) => setTimeout(r, 20));
    expect(listLiveRuns).not.toHaveBeenCalled();
  });

  it("顯示數量；點開列出每個瀏覽器（系統、Agent、接手中標示）；按查看開啟畫面", async () => {
    mount();
    expect(await screen.findByTestId("ac-live-bar")).toHaveTextContent("2 個瀏覽器執行中");
    fireEvent.click(screen.getByTestId("ac-live-bar"));
    const rows = await screen.findAllByTestId("ac-live-run");
    expect(rows[0]).toHaveTextContent("ERP · 工程師1");
    expect(rows[1]).toHaveTextContent("接手中");
    fireEvent.click(screen.getAllByRole("button", { name: "查看 ERP" })[1]);
    expect(await screen.findByTestId("live-view-dialog")).toBeInTheDocument();
    await waitFor(() => expect(openLive).toHaveBeenCalledWith({ kind: "agent", systemId: "s1", cardId: "c2" }, expect.any(Function), expect.any(Function)));
  });

  it("查詢失敗（server 較舊）不畫也不報錯", async () => {
    vi.mocked(listLiveRuns).mockRejectedValue("not found");
    const { container } = mount();
    await waitFor(() => expect(listLiveRuns).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });
});
