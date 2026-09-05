import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const listReports = vi.fn();
const readReport = vi.fn();
const generate = vi.fn();
const cancel = vi.fn();
const listProviders = vi.fn();
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
vi.mock("../../ipc/provider", () => ({
  listProviders: (...a: unknown[]) => listProviders(...a),
}));
// ModelPickerButton 會拉 useProviderQuota / usageQuotaAll，兩者都打真的 IPC。
// 一旦選中的 provider id 是真的字串（本檔的情境幾乎都是），這裡就會被呼叫，
// 不 mock 掉的話會打到不存在的 Tauri invoke。
vi.mock("../../ipc/usage", () => ({
  usageQuota: vi.fn().mockResolvedValue({ status: "not_applicable", provider_id: "x" }),
  usageQuotaAll: vi.fn().mockResolvedValue([]),
  primaryWindow: () => null,
}));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { ReportDialog } from "./ReportDialog";
import type { ProviderInfo } from "../../ipc/provider";

const PROVIDERS: ProviderInfo[] = [
  {
    id: "p-a", display_name: "Provider A", provider_type: "anthropic",
    base_url: null, oauth_client_id: null, model: "claude-x",
    supports_json_mode: true, has_api_key: true, is_default: false, auth_method: null,
  },
  {
    id: "p-b", display_name: "Provider B", provider_type: "openai",
    base_url: null, oauth_client_id: null, model: "gpt-x",
    supports_json_mode: true, has_api_key: true, is_default: true, auth_method: null,
  },
];

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
    listProviders.mockResolvedValue(PROVIDERS);
  });

  it("開啟時先讓使用者選風格", async () => {
    mount();
    expect(await screen.findByTestId("report-style-review")).toBeInTheDocument();
    expect(screen.getByTestId("report-style-formal")).toBeInTheDocument();
  });

  it("選了風格才開始產生，並把選中的模型記住", async () => {
    mount();
    await screen.findByText("Provider B"); // 等預設供應商選好
    await userEvent.click(await screen.findByTestId("report-style-review"));
    expect(generate).toHaveBeenCalledWith("review", "p-b");
    expect(localStorage.getItem("aiterm_report_provider")).toBe("p-b");
  });

  it("另一種風格傳的是 formal", async () => {
    mount();
    await screen.findByText("Provider B");
    await userEvent.click(await screen.findByTestId("report-style-formal"));
    expect(generate).toHaveBeenCalledWith("formal", "p-b");
  });

  it("列出可選的模型，預設選中預設供應商", async () => {
    mount();
    expect(await screen.findByText("Provider B")).toBeInTheDocument();
  });

  it("記住上次選的模型", async () => {
    localStorage.setItem("aiterm_report_provider", "p-a");
    mount();
    expect(await screen.findByText("Provider A")).toBeInTheDocument();
  });

  it("記住的模型已經不存在時退回預設", async () => {
    localStorage.setItem("aiterm_report_provider", "does-not-exist");
    mount();
    expect(await screen.findByText("Provider B")).toBeInTheDocument();
  });

  it("產生時把選中的模型傳給 generate", async () => {
    localStorage.setItem("aiterm_report_provider", "p-a");
    mount();
    await screen.findByText("Provider A");
    await userEvent.click(await screen.findByTestId("report-style-review"));
    expect(generate).toHaveBeenCalledWith("review", "p-a");
  });

  it("列出歷史報告", async () => {
    listReports.mockResolvedValue([
      { filename: "2026-09-05-1430.html", saved_at: 1788600000, title: "第三季進度" },
    ]);
    mount();
    expect(await screen.findByText("第三季進度")).toBeInTheDocument();
  });

  // 實機回報：同一個專案的報告標題幾乎都一樣（AI 會用專案名當標題），
  // 只列標題根本分不出哪份是哪份。產生時間才是實際用來辨識的資訊。
  it("每份歷史報告都顯示產生時間", async () => {
    listReports.mockResolvedValue([
      { filename: "2026-09-05-1430.html", saved_at: 1788600000, title: "TEST-1 工作報告" },
      { filename: "2026-09-05-1445.html", saved_at: 1788600900, title: "TEST-1 工作報告" },
    ]);
    mount();
    await screen.findAllByText("TEST-1 工作報告");
    const stamps = screen.getAllByTestId(/^report-history-time-/);
    expect(stamps).toHaveLength(2);
    // 兩份標題相同，時間必須不同才分得出來——這正是這個功能存在的理由。
    expect(stamps[0].textContent).not.toBe(stamps[1].textContent);
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
