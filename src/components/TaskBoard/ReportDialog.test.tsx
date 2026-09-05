import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const listReports = vi.fn();
const readReport = vi.fn();
const generate = vi.fn();
const cancel = vi.fn();
let hookState: Record<string, unknown> = {};

vi.mock("../../ipc/reports", () => ({
  listReports: (...a: unknown[]) => listReports(...a),
  readReport: (...a: unknown[]) => readReport(...a),
  saveReport: vi.fn(),
}));
vi.mock("./useWorkReport", () => ({
  useWorkReport: () => ({
    generate,
    cancel,
    busy: false,
    progress: null,
    html: null,
    error: null,
    rawReply: null,
    setHtml: vi.fn(),
    ...hookState,
  }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn() }));
vi.mock("../../ipc/fs", () => ({ writeTextFile: vi.fn() }));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { ReportDialog } from "./ReportDialog";

const mount = () =>
  render(
    <LocaleProvider>
      <ReportDialog projectId="p1" projectName="我的專案" onClose={vi.fn()} />
    </LocaleProvider>,
  );

describe("ReportDialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    hookState = {};
    listReports.mockResolvedValue([]);
    readReport.mockResolvedValue("<html><title>舊報告</title></html>");
  });

  it("開啟時先讓使用者選風格", async () => {
    mount();
    expect(await screen.findByTestId("report-style-review")).toBeInTheDocument();
    expect(screen.getByTestId("report-style-formal")).toBeInTheDocument();
  });

  it("選了風格才開始產生", async () => {
    mount();
    await userEvent.click(await screen.findByTestId("report-style-review"));
    expect(generate).toHaveBeenCalledWith("review");
  });

  it("另一種風格傳的是 formal", async () => {
    mount();
    await userEvent.click(await screen.findByTestId("report-style-formal"));
    expect(generate).toHaveBeenCalledWith("formal");
  });

  it("列出歷史報告", async () => {
    listReports.mockResolvedValue([
      { filename: "2026-09-05-1430.html", saved_at: 1788600000, title: "第三季進度" },
    ]);
    mount();
    expect(await screen.findByText("第三季進度")).toBeInTheDocument();
  });

  it("沒有標題的報告顯示檔名", async () => {
    listReports.mockResolvedValue([
      { filename: "2026-09-05-1430.html", saved_at: 1788600000, title: null },
    ]);
    mount();
    expect(await screen.findByText("2026-09-05-1430.html")).toBeInTheDocument();
  });

  it("點歷史報告會讀回它的內容", async () => {
    listReports.mockResolvedValue([
      { filename: "2026-09-05-1430.html", saved_at: 1788600000, title: "舊報告" },
    ]);
    mount();
    await userEvent.click(await screen.findByText("舊報告"));
    await waitFor(() => expect(readReport).toHaveBeenCalledWith("p1", "2026-09-05-1430.html"));
  });

  it("沒有歷史報告時顯示空狀態", async () => {
    mount();
    expect(await screen.findByTestId("report-history-empty")).toBeInTheDocument();
  });

  it("產生中顯示具體進度而不是只轉圈圈", async () => {
    hookState = { busy: true, progress: { done: 3, total: 10 } };
    mount();
    const text = (await screen.findByTestId("report-progress")).textContent ?? "";
    expect(text).toContain("3");
    expect(text).toContain("10");
  });

  it("產生中可以取消", async () => {
    hookState = { busy: true, progress: { done: 1, total: 5 } };
    mount();
    await userEvent.click(await screen.findByTestId("report-cancel"));
    expect(cancel).toHaveBeenCalled();
  });

  // AI 沒吐出 artifact 時要讓使用者看到它到底回了什麼，
  // 不然只會看到一句「失敗」卻不知道發生什麼事。
  it("有原始回覆時顯示出來", async () => {
    hookState = { error: "AI 沒有產生報告文件", rawReply: "我不會做這個" };
    mount();
    expect(await screen.findByText(/我不會做這個/)).toBeInTheDocument();
  });
});
