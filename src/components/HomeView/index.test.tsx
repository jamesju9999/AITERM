import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";

vi.mock("../../ipc/bridge", () => ({ bridgeStatus: vi.fn() }));
// UsageSection 掛載就會查用量。不 mock 的話會真的打 Tauri IPC，在 jsdom 下
// 變成一個沒人管的 rejection，測試結果會取決於它何時被拒絕。
vi.mock("../../ipc/usage", () => ({ usageSummary: vi.fn() }));
const invokeAiChatMock = vi.fn();
vi.mock("../../ipc/ai", () => ({
  invokeAiChat: (...args: unknown[]) => invokeAiChatMock(...args),
}));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { HomeView } from "./index";
import { bridgeStatus } from "../../ipc/bridge";
import { usageSummary } from "../../ipc/usage";
import type { Tab } from "../TabBar";
import type { RouteResult } from "./routeIntent";

beforeEach(() => {
  vi.mocked(bridgeStatus).mockReset();
  vi.mocked(bridgeStatus).mockResolvedValue({ running: true, port: 8317, token: "tok", error: null });
  vi.mocked(usageSummary).mockReset();
  vi.mocked(usageSummary).mockResolvedValue([]);
  invokeAiChatMock.mockReset();
});

function renderHome(tabs: Tab[]) {
  return render(
    <MemoryRouter initialEntries={["/"]}>
      <LocaleProvider>
        <HomeView onOpenTab={vi.fn()} tabs={tabs} onSelectTab={vi.fn()} />
      </LocaleProvider>
    </MemoryRouter>,
  );
}

describe("HomeView", () => {
  // 順序有意義：進行中的東西 → 接續上次的工作 → 行動入口 → 參考資訊。
  it("區塊由上到下是進行中的任務、接續上次的工作、開始工作、今日 AI 用量", () => {
    const { container } = renderHome([
      { id: "t1", title: "建置", type: "terminal", agentProgress: { done: 1, total: 2 } },
    ]);
    const titles = Array.from(container.querySelectorAll(".home-section-title")).map((e) => e.textContent);
    expect(titles).toEqual(["進行中的任務", "接續上次的工作", "開始工作", "今日 AI 用量"]);
  });
});

// 這一段是跨 Task 的交界：HomeInput 的測試管到 onRoute 為止，RouteHint 的測試
// 從 props 開始，中間這個「fallback 時不顯示提示」的判斷原本沒有任何測試。
// 規格明訂降級開出來的分頁不該有提示——那個終端機直接看得到任務在跑。
describe("HomeView 的 AI 路由接線", () => {
  function renderWithRoute(onAiRouted: (tabId: string, route: RouteResult) => void) {
    const onOpenTab = vi.fn().mockReturnValue("new-tab-id");
    render(
      <MemoryRouter initialEntries={["/"]}>
        <LocaleProvider>
          <HomeView onOpenTab={onOpenTab} tabs={[]} onSelectTab={vi.fn()} onAiRouted={onAiRouted} />
        </LocaleProvider>
      </MemoryRouter>,
    );
    return onOpenTab;
  }

  async function submit(text: string) {
    const box = screen.getByRole("textbox");
    fireEvent.change(box, { target: { value: text } });
    fireEvent.keyDown(box, { key: "Enter" });
  }

  it("AI 判斷成功時回報，讓上層顯示可反悔的提示", async () => {
    invokeAiChatMock.mockResolvedValue({ content: '{"type":"database"}' });
    const onAiRouted = vi.fn();
    renderWithRoute(onAiRouted);
    await submit("查一下訂單表");
    await waitFor(() =>
      expect(onAiRouted).toHaveBeenCalledWith(
        "new-tab-id",
        expect.objectContaining({ type: "database", fallback: false }),
      ),
    );
  });

  it("降級開出來的分頁不回報——那裡不該有提示", async () => {
    invokeAiChatMock.mockRejectedValue({ kind: "not_configured" });
    const onAiRouted = vi.fn();
    const onOpenTab = renderWithRoute(onAiRouted);
    await submit("幫我修 build");
    // 分頁照樣要開（輸入框永遠有反應），只是不回報。
    await waitFor(() => expect(onOpenTab).toHaveBeenCalled());
    expect(onAiRouted).not.toHaveBeenCalled();
  });

  it("降級時把整句話當成 agent 任務目標開終端機", async () => {
    invokeAiChatMock.mockRejectedValue({ kind: "network" });
    const onOpenTab = renderWithRoute(vi.fn());
    await submit("幫我修 build");
    await waitFor(() =>
      expect(onOpenTab).toHaveBeenCalledWith("terminal", {
        initialMission: { goal: "幫我修 build", maxSteps: 20 },
      }),
    );
  });
});
