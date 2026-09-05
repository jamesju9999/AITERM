import { render } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const listen = vi.fn();
const tryUpgradeTranscript = vi.fn();

vi.mock("@tauri-apps/api/event", () => ({ listen: (...a: unknown[]) => listen(...a) }));
vi.mock("./transcriptUpgrade", () => ({
  tryUpgradeTranscript: (...a: unknown[]) => tryUpgradeTranscript(...a),
}));

import { useTranscriptUpgrader } from "./useTranscriptUpgrader";

function Host() {
  useTranscriptUpgrader();
  return null;
}

describe("useTranscriptUpgrader", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    listen.mockResolvedValue(() => {});
  });

  it("訂閱 task-finished", () => {
    render(<Host />);
    expect(listen).toHaveBeenCalledWith("task-finished", expect.any(Function));
  });

  // 這是整個 hook 存在的理由：卡片完成時，負責乾淨化的程式碼不可以
  // 依賴「該專案的看板正好掛載著」。事件酬載自己就帶齊了三個參數。
  it("收到事件就用酬載裡的專案/卡片/分頁去升級", async () => {
    render(<Host />);
    const handler = listen.mock.calls[0][1] as (e: unknown) => void;
    handler({ payload: { project_id: "p1", task_id: "t9", tab_id: "tab-3" } });
    expect(tryUpgradeTranscript).toHaveBeenCalledWith("p1", "t9", "tab-3");
  });

  it("卸載時取消訂閱", async () => {
    const unlisten = vi.fn();
    listen.mockResolvedValue(unlisten);
    const view = render(<Host />);
    view.unmount();
    await vi.waitFor(() => expect(unlisten).toHaveBeenCalled());
  });
});
