import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";

vi.mock("../../ipc/tasks", () => ({
  getOrchestration: vi.fn(),
  setOrchestration: vi.fn(),
}));

import { getOrchestration, setOrchestration } from "../../ipc/tasks";
import { ServerRunBar } from "./ServerRunBar";

const mount = (props: Partial<Parameters<typeof ServerRunBar>[0]> = {}) => {
  const onError = vi.fn();
  const utils = render(
    <LocaleProvider>
      <ServerRunBar projectId="remote:p1" onError={onError} {...props} />
    </LocaleProvider>,
  );
  return { onError, ...utils };
};

beforeEach(() => {
  vi.mocked(getOrchestration).mockResolvedValue(false);
  vi.mocked(setOrchestration).mockResolvedValue(undefined);
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("ServerRunBar", () => {
  it("本機專案沒有「由 server 執行」：什麼都不畫", async () => {
    vi.mocked(getOrchestration).mockResolvedValue(null);
    const { container } = mount({ projectId: "local-uuid" });
    await waitFor(() => expect(getOrchestration).toHaveBeenCalledWith("local-uuid"));
    expect(container).toBeEmptyDOMElement();
  });

  it("共用專案：顯示目前狀態，按一下切換並送給 server", async () => {
    mount();
    const btn = await screen.findByTestId("ac-server-run");
    expect(btn).toHaveTextContent("由 server 執行 · 未開啟");
    expect(btn).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(btn);
    await waitFor(() => expect(btn).toHaveTextContent("由 server 執行 · 已開啟"));
    expect(setOrchestration).toHaveBeenCalledWith("remote:p1", true);
    fireEvent.click(btn);
    await waitFor(() => expect(btn).toHaveTextContent("未開啟"));
    expect(setOrchestration).toHaveBeenLastCalledWith("remote:p1", false);
  });

  it("server 拒絕（不是管理員、沒設資料夾…）：畫面維持原狀並把原因交出去", async () => {
    vi.mocked(setOrchestration).mockRejectedValue("只有管理員可以做這件事");
    const { onError } = mount();
    const btn = await screen.findByTestId("ac-server-run");
    fireEvent.click(btn);
    await waitFor(() => expect(onError).toHaveBeenCalledWith("只有管理員可以做這件事"));
    expect(btn).toHaveTextContent("未開啟");
    expect(btn).not.toBeDisabled();
  });

  it("換專案時不會拿上一個專案的狀態來畫", async () => {
    vi.mocked(getOrchestration).mockResolvedValueOnce(true);
    const { rerender } = mount();
    expect(await screen.findByTestId("ac-server-run")).toHaveTextContent("已開啟");
    vi.mocked(getOrchestration).mockReturnValueOnce(new Promise(() => {})); // 新專案還在載入
    rerender(
      <LocaleProvider>
        <ServerRunBar projectId="remote:p2" onError={vi.fn()} />
      </LocaleProvider>,
    );
    expect(screen.queryByTestId("ac-server-run")).toBeNull();
  });
});
