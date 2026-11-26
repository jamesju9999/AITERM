import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";

vi.mock("../../ipc/agents", () => ({ verifyReport: vi.fn() }));
vi.mock("../../ipc/fs", () => ({ writeTextFile: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn() }));

import { verifyReport } from "../../ipc/agents";
import { VerifyReportDialog } from "./VerifyReportDialog";

const REPORT = {
  verdict: "fail" as const,
  project_type: "web",
  items: [
    { criterion: "按開始倒數", status: "fail" as const, module_id: "m1", evidence: ["evidence/click.png", "npm test: ok"], steps: "按下開始", expected: "25:00", actual: "24:58" },
    { criterion: "主要入口可用", status: "pass" as const, evidence: ["evidence/start.png"] },
  ],
};

const mount = (over: Partial<Parameters<typeof VerifyReportDialog>[0]> = {}) => {
  const props = { projectId: "remote:p1", projectName: "範例", taskId: "v1", canAccept: true, onAccept: vi.fn(), onClose: vi.fn(), ...over };
  render(
    <LocaleProvider>
      <VerifyReportDialog {...props} />
    </LocaleProvider>,
  );
  return props;
};

beforeEach(() => {
  vi.mocked(verifyReport).mockResolvedValue({ report: REPORT, images: { "evidence/click.png": "data:image/png;base64,AAA" } });
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("VerifyReportDialog（server 驗收的新增部分）", () => {
  it("顯示每個條件、重現步驟與截圖縮圖；沒有 onRetry 就沒有「重新驗收」", async () => {
    mount();
    expect(await screen.findByText("按開始倒數")).toBeInTheDocument();
    expect(screen.getByText("按下開始")).toBeInTheDocument();
    expect(screen.getByAltText("evidence/click.png")).toBeInTheDocument();
    expect(screen.queryByTestId("verify-retry")).toBeNull();
  });

  it("有 onRetry（共用看板）時出現「重新驗收」，按下去呼叫它", async () => {
    const onRetry = vi.fn();
    mount({ onRetry });
    fireEvent.click(await screen.findByTestId("verify-retry"));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it("被禁止清單擋下的命令列在報告裡，並提醒誤擋要找管理員", async () => {
    vi.mocked(verifyReport).mockResolvedValue({ report: REPORT, images: {}, denied: ["git push origin main"] });
    mount();
    const box = await screen.findByTestId("verify-denied");
    expect(box).toHaveTextContent("git push origin main");
    expect(box).toHaveTextContent("管理員");
  });

  it("驗收卡自己失敗（沒有報告）時顯示原因，仍可放行與重新驗收", async () => {
    vi.mocked(verifyReport).mockResolvedValue({ report: null, images: {}, error: "合併衝突：「工程師2」的分支無法併進整合版本", denied: [] });
    const onAccept = vi.fn();
    const onRetry = vi.fn();
    mount({ onAccept, onRetry });
    expect(await screen.findByTestId("verify-error")).toHaveTextContent("合併衝突");
    fireEvent.click(screen.getByRole("button", { name: "放行" }));
    expect(onAccept).toHaveBeenCalled();
    fireEvent.click(screen.getByTestId("verify-retry"));
    expect(onRetry).toHaveBeenCalled();
    await waitFor(() => expect(verifyReport).toHaveBeenCalledWith("remote:p1", "v1"));
  });

  it("本機驗收（沒有 denied／error 欄位）照舊顯示，不出現多餘區塊", async () => {
    mount();
    await screen.findByText("按開始倒數");
    expect(screen.queryByTestId("verify-denied")).toBeNull();
    expect(screen.queryByTestId("verify-error")).toBeNull();
  });
});
