import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const removeProject = vi.fn();
vi.mock("../../ipc/projects", () => ({
  removeProject: (...a: unknown[]) => removeProject(...a),
}));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { ProjectTabBar } from "./ProjectTabBar";
import type { ProjectInfo } from "../../ipc/projects";

const proj = (id: string, running = 0): ProjectInfo => ({
  id,
  name: id,
  description: "",
  path: `/p/${id}`,
  status: "ok",
  counts: { planning: 0, queued: 0, running, done: 0 },
  error: null,
});

const mount = (over: Partial<Parameters<typeof ProjectTabBar>[0]> = {}) => {
  const props = {
    projects: [proj("alpha"), proj("beta", 2)],
    openIds: ["alpha", "beta"],
    activeId: "alpha",
    onActivate: vi.fn(),
    onClose: vi.fn(),
    onOpenOther: vi.fn(),
    onBackToList: vi.fn(),
    ...over,
  };
  render(
    <LocaleProvider>
      <ProjectTabBar {...props} />
    </LocaleProvider>,
  );
  return props;
};

describe("ProjectTabBar", () => {
  beforeEach(() => vi.clearAllMocks());

  it("只顯示已開啟的專案", () => {
    mount({ openIds: ["alpha"] });
    expect(screen.getByTestId("project-tab-alpha")).toBeInTheDocument();
    expect(screen.queryByTestId("project-tab-beta")).not.toBeInTheDocument();
  });

  it("點分頁切換活躍專案", async () => {
    const props = mount();
    await userEvent.click(screen.getByTestId("project-tab-beta"));
    expect(props.onActivate).toHaveBeenCalledWith("beta");
  });

  it("有執行中工作的分頁顯示指示點", () => {
    mount();
    expect(screen.getByTestId("project-tab-running-beta")).toBeInTheDocument();
    expect(screen.queryByTestId("project-tab-running-alpha")).not.toBeInTheDocument();
  });

  // 這是本功能最容易寫錯的地方：關閉分頁跟移除專案視覺上都是
  // 「把這個專案弄掉」，但語意完全不同。這個測試把它釘死。
  it("關閉分頁只呼叫 onClose，絕不呼叫 removeProject", async () => {
    const props = mount();
    await userEvent.click(screen.getByTestId("project-tab-close-alpha"));
    expect(props.onClose).toHaveBeenCalledWith("alpha");
    expect(removeProject).not.toHaveBeenCalled();
  });

  it("關閉鍵不會順帶切換活躍專案", async () => {
    const props = mount({ activeId: "beta" });
    await userEvent.click(screen.getByTestId("project-tab-close-alpha"));
    expect(props.onActivate).not.toHaveBeenCalled();
  });

  it("回專案總覽", async () => {
    const props = mount();
    await userEvent.click(screen.getByTestId("project-tab-back"));
    expect(props.onBackToList).toHaveBeenCalled();
  });

  it("開啟其他專案的按鈕", async () => {
    const props = mount();
    await userEvent.click(screen.getByTestId("project-tab-add"));
    expect(props.onOpenOther).toHaveBeenCalled();
  });
});
