import { describe, expect, it, vi, beforeEach } from "vitest";
import { render } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";

vi.mock("../../ipc/bridge", () => ({ bridgeStatus: vi.fn() }));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { HomeView } from "./index";
import { bridgeStatus } from "../../ipc/bridge";
import type { Tab } from "../TabBar";

beforeEach(() => {
  vi.mocked(bridgeStatus).mockReset();
  vi.mocked(bridgeStatus).mockResolvedValue({ running: true, port: 8317, token: "tok", error: null });
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
  it("進行中的任務排在開始工作之前", () => {
    const { container } = renderHome([
      { id: "t1", title: "建置", type: "terminal", agentProgress: { done: 1, total: 2 } },
    ]);
    const titles = Array.from(container.querySelectorAll(".home-section-title")).map((e) => e.textContent);
    expect(titles).toEqual(["進行中的任務", "開始工作"]);
  });
});
