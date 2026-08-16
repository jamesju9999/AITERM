import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { RunningTasks } from "./RunningTasks";
import type { Tab } from "../TabBar";

function renderTasks(tabs: Tab[], onSelectTab = vi.fn()) {
  render(
    <LocaleProvider>
      <RunningTasks tabs={tabs} onSelectTab={onSelectTab} />
    </LocaleProvider>,
  );
  return onSelectTab;
}

describe("RunningTasks", () => {
  it("沒有進行中的任務時整區不出現", () => {
    render(
      <LocaleProvider>
        <RunningTasks tabs={[{ id: "t1", title: "Tab 1", type: "terminal" }]} onSelectTab={vi.fn()} />
      </LocaleProvider>,
    );
    // 查標題而不是查個別任務按鈕：沒有任務時 map 本來就產不出按鈕，
    // 查按鈕的話不管 early return 在不在都會通過（這條測試原本就是這樣壞的）。
    expect(screen.queryByText("進行中的任務")).not.toBeInTheDocument();
  });

  it("有任務時整區出現", () => {
    renderTasks([
      { id: "t1", title: "建置", type: "terminal", agentProgress: { done: 1, total: 2 } },
    ]);
    expect(screen.getByText("進行中的任務")).toBeInTheDocument();
  });

  it("列出有 agentProgress 的分頁與其進度", () => {
    renderTasks([
      { id: "t1", title: "建置", type: "terminal", agentProgress: { done: 3, total: 8 } },
    ]);
    expect(screen.getByText("建置")).toBeInTheDocument();
    expect(screen.getByText("3 / 8")).toBeInTheDocument();
  });

  // 企業浮動面板只顯示 enterpriseTask 的任務，首頁刻意不套那個過濾。
  it("一般 agent 任務也要顯示，不是只有企業任務", () => {
    renderTasks([
      { id: "t1", title: "一般任務", type: "terminal", agentProgress: { done: 1, total: 2 } },
    ]);
    expect(screen.getByText("一般任務")).toBeInTheDocument();
  });

  it("點某個任務會切到該分頁", () => {
    const onSelectTab = renderTasks([
      { id: "t9", title: "建置", type: "terminal", agentProgress: { done: 1, total: 2 } },
    ]);
    fireEvent.click(screen.getByText("建置"));
    expect(onSelectTab).toHaveBeenCalledWith("t9");
  });

  // 只有一筆任務時，「列出全部」跟「只列第一筆」看起來一樣。
  it("多筆任務全部列出", () => {
    const { container } = render(
      <LocaleProvider>
        <RunningTasks
          tabs={[
            { id: "a", title: "任務甲", type: "terminal", agentProgress: { done: 1, total: 3 } },
            { id: "b", title: "任務乙", type: "terminal", agentProgress: { done: 2, total: 5 } },
          ]}
          onSelectTab={vi.fn()}
        />
      </LocaleProvider>,
    );
    expect(container.querySelectorAll(".home-running-task")).toHaveLength(2);
    expect(screen.getByText("任務乙")).toBeInTheDocument();
  });
});
