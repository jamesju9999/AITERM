import { describe, expect, it, vi, beforeEach } from "vitest";
import { render } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";

vi.mock("../../ipc/bridge", () => ({ bridgeStatus: vi.fn() }));
// UsageSection 掛載就會查用量。不 mock 的話會真的打 Tauri IPC，在 jsdom 下
// 變成一個沒人管的 rejection，測試結果會取決於它何時被拒絕。
vi.mock("../../ipc/usage", () => ({ usageSummary: vi.fn() }));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { HomeView } from "./index";
import { bridgeStatus } from "../../ipc/bridge";
import { usageSummary } from "../../ipc/usage";
import type { Tab } from "../TabBar";

beforeEach(() => {
  vi.mocked(bridgeStatus).mockReset();
  vi.mocked(bridgeStatus).mockResolvedValue({ running: true, port: 8317, token: "tok", error: null });
  vi.mocked(usageSummary).mockReset();
  vi.mocked(usageSummary).mockResolvedValue([]);
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
  // 順序有意義：進行中的東西 → 行動入口 → 參考資訊。
  it("區塊由上到下是進行中的任務、開始工作、今日 AI 用量", () => {
    const { container } = renderHome([
      { id: "t1", title: "建置", type: "terminal", agentProgress: { done: 1, total: 2 } },
    ]);
    const titles = Array.from(container.querySelectorAll(".home-section-title")).map((e) => e.textContent);
    expect(titles).toEqual(["進行中的任務", "開始工作", "今日 AI 用量"]);
  });
});
