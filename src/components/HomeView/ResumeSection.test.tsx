import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { ResumeSection } from "./ResumeSection";
import { RECENT_PROJECTS_KEY } from "../../lib/recentProjects";
import type { Tab } from "../TabBar";

function renderResume(tabs: Tab[], onSelectTab = vi.fn(), onOpenProject = vi.fn()) {
  const { container } = render(
    <LocaleProvider>
      <ResumeSection tabs={tabs} onSelectTab={onSelectTab} onOpenProject={onOpenProject} />
    </LocaleProvider>,
  );
  return { container, onSelectTab, onOpenProject };
}

beforeEach(() => {
  localStorage.clear();
});

describe("ResumeSection", () => {
  it("分頁卡片顯示工作目錄與 AI 摘要", () => {
    renderResume([
      { id: "t1", title: "Tab 1", type: "terminal", cwd: "/repo/aiterm", aiSummary: "跑了建置" },
    ]);
    expect(screen.getByText("/repo/aiterm")).toBeInTheDocument();
    expect(screen.getByText("跑了建置")).toBeInTheDocument();
  });

  // cwd 只對終端機分頁有意義，aiSummary 只有跑過指令的分頁才有。
  // 沒有的欄位就不要留空位。
  it("沒有 cwd 或摘要的分頁只顯示標題", () => {
    const { container } = renderResume([{ id: "t1", title: "資料庫", type: "database" }]);
    expect(screen.getByText("資料庫")).toBeInTheDocument();
    expect(container.querySelector(".home-resume-cwd")).toBeNull();
    expect(container.querySelector(".home-resume-summary")).toBeNull();
  });

  it("點分頁卡片會切到該分頁", () => {
    const { onSelectTab } = renderResume([{ id: "t7", title: "Tab 7", type: "terminal" }]);
    fireEvent.click(screen.getByText("Tab 7"));
    expect(onSelectTab).toHaveBeenCalledWith("t7");
  });

  it("列出最近的專案目錄", () => {
    localStorage.setItem(
      RECENT_PROJECTS_KEY,
      JSON.stringify([{ path: "/repo/aiterm", lastUsedAt: 1 }]),
    );
    renderResume([]);
    expect(screen.getByText("/repo/aiterm")).toBeInTheDocument();
  });

  it("點最近專案會用該路徑呼叫 onOpenProject", () => {
    localStorage.setItem(
      RECENT_PROJECTS_KEY,
      JSON.stringify([{ path: "/repo/aiterm", lastUsedAt: 1 }]),
    );
    const { onOpenProject } = renderResume([]);
    fireEvent.click(screen.getByText("/repo/aiterm"));
    expect(onOpenProject).toHaveBeenCalledWith("/repo/aiterm");
  });

  // 多筆才分得出「列出全部」與「只列第一筆」。
  it("多個最近專案全部列出", () => {
    localStorage.setItem(RECENT_PROJECTS_KEY, JSON.stringify([
      { path: "/a", lastUsedAt: 2 },
      { path: "/b", lastUsedAt: 1 },
    ]));
    const { container } = renderResume([]);
    expect(container.querySelectorAll(".home-recent-item")).toHaveLength(2);
  });

  it("沒有分頁也沒有最近專案時顯示空狀態", () => {
    renderResume([]);
    expect(screen.getByText("還沒有可以接續的工作")).toBeInTheDocument();
  });
});
