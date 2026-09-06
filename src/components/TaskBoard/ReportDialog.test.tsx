import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const listReports = vi.fn();
const readReport = vi.fn();
const generate = vi.fn();
const cancel = vi.fn();
const listProviders = vi.fn();
const deleteReport = vi.fn();
const confirmDialog = vi.fn();
const listTasks = vi.fn();
let hookState: Record<string, unknown> = {};
/** 讓測試拿到 hook 內部的 setHtml，用來模擬「產生完成」。 */
let captureSetHtml: ((fn: (v: string) => void) => void) | null = null;

vi.mock("../../ipc/reports", () => ({
  listReports: (...a: unknown[]) => listReports(...a),
  readReport: (...a: unknown[]) => readReport(...a),
  deleteReport: (...a: unknown[]) => deleteReport(...a),
  saveReport: vi.fn(),
}));
// `html` 用真的 useState 而不是靜態值：元件會呼叫 `setHtml(null)` 回到
// 風格選擇頁、也會用它載入歷史報告，裸的 vi.fn() 模擬不出「setHtml 之後
// html 真的變了」這件事，那些行為就測不到。初始值從 hookState 帶進來，
// 讓各測試還是能指定一開始有沒有報告在看。
vi.mock("./useWorkReport", async () => {
  const { useState } = await import("react");
  return {
    useWorkReport: () => {
      const [html, setHtml] = useState<string | null>((hookState.html as string | undefined) ?? null);
      captureSetHtml?.(setHtml as (v: string) => void);
      return {
        generate,
        cancel,
        busy: false,
        progress: null,
        error: null,
        rawReply: null,
        ...hookState,
        html,
        setHtml,
      };
    },
  };
});
vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: vi.fn(),
  confirm: (...a: unknown[]) => confirmDialog(...a),
}));
vi.mock("../../ipc/fs", () => ({ writeTextFile: vi.fn() }));
vi.mock("../../ipc/provider", () => ({
  listProviders: (...a: unknown[]) => listProviders(...a),
}));
vi.mock("../../ipc/tasks", () => ({ listTasks: (...a: unknown[]) => listTasks(...a) }));
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
    captureSetHtml = null;
    generate.mockReset();
    listReports.mockResolvedValue([]);
    readReport.mockResolvedValue("<html><title>舊報告</title></html>");
    listProviders.mockResolvedValue(PROVIDERS);
    listTasks.mockResolvedValue([]);
    deleteReport.mockResolvedValue(undefined);
    confirmDialog.mockResolvedValue(true);
  });

  const taskCard = (over: Record<string, unknown> = {}) => ({
    id: "t1", title: "卡片", body: "", project_dir: "/r", status: "done",
    parallel_ok: true, interactive: false, sort_order: 1, outcome: "success",
    tab_id: null, transcript_path: null, error_message: null,
    created_at: "2026-09-05T10:00:00Z", dispatched_at: null, finished_at: null,
    ai_summary: null, archived_at: null, attachments: [], ...over,
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
    expect(generate).toHaveBeenCalledWith("review", "p-b", false);
    expect(localStorage.getItem("aiterm_report_provider")).toBe("p-b");
  });

  it("另一種風格傳的是 formal", async () => {
    mount();
    await screen.findByText("Provider B");
    await userEvent.click(await screen.findByTestId("report-style-formal"));
    expect(generate).toHaveBeenCalledWith("formal", "p-b", false);
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
    expect(generate).toHaveBeenCalledWith("review", "p-a", false);
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
  // 風格選擇頁原本只有兩張卡片、底下一大片空白，而且沒有回答使用者
  // 真正關心的問題：這次要跑多久。第一階段是 N 次 AI 呼叫，N 就是
  // 「需要重新整理」的張數。
  it("選風格前先說明這次會包含哪些工作項目", async () => {
    listTasks.mockResolvedValue([
      taskCard({ id: "a", status: "done" }),
      taskCard({ id: "b", status: "done" }),
      taskCard({ id: "c", status: "planning", outcome: null }),
      taskCard({ id: "d", status: "running", outcome: null }),
    ]);
    mount();
    const scope = await screen.findByTestId("report-scope");
    expect(scope.textContent).toContain("4");
  });

  // 這是整個摘要面板存在的主要理由：它預告了要跑多久。
  it("標出有幾張需要重新整理對話記錄", async () => {
    listTasks.mockResolvedValue([
      taskCard({ id: "a", status: "done", ai_summary: "已經整理過" }),
      taskCard({ id: "b", status: "done", ai_summary: null }),
      taskCard({ id: "c", status: "done", ai_summary: null }),
    ]);
    mount();
    const need = await screen.findByTestId("report-scope-pending");
    expect(need.textContent).toContain("2");
  });

  // 全部都有快取時，第二階段只要一次呼叫，會很快——要讓使用者知道，
  // 不然他會以為又要等剛才那麼久。
  it("全部都有快取時明說不需要重新整理", async () => {
    listTasks.mockResolvedValue([
      taskCard({ id: "a", status: "done", ai_summary: "有了" }),
    ]);
    mount();
    await screen.findByTestId("report-scope");
    expect(screen.queryByTestId("report-scope-pending")).not.toBeInTheDocument();
  });

  // 實機回報：點了歷史報告之後，風格選擇區就永遠隱藏了（條件是
  // !started && !html），只能關掉視窗重開才回得去。
  it("看完報告後可以回去產生新的一份", async () => {
    hookState = { html: "<html><title>舊報告</title></html>" };
    mount();

    // 有報告在看的時候，風格選擇區是收起來的
    expect(screen.queryByTestId("report-style-review")).not.toBeInTheDocument();

    await userEvent.click(await screen.findByTestId("report-new"));
    expect(await screen.findByTestId("report-style-review")).toBeInTheDocument();
  });

  // 上面那條只涵蓋「從歷史點開報告」（started 仍是 false）。自己產生完
  // 之後 started 是 true，只清 html 不清 started 一樣回不去——實測過
  // 這個突變不會被上面那條抓到，所以要分開測。
  it("自己產生完報告後也能回去再產一份", async () => {
    // 模擬「按了風格 → 產生完成」：generate 一被呼叫就讓 hook 交出 html。
    // 這樣元件的 started 是 true（自己按的），html 也有值——只清 html
    // 不清 started 的話，風格選擇區的 `!started && !html` 依然不成立。
    let setHtmlFromHook: ((v: string) => void) | null = null;
    generate.mockImplementation(() => {
      setHtmlFromHook?.("<html><title>剛產的</title></html>");
    });
    captureSetHtml = (fn) => {
      setHtmlFromHook = fn;
    };

    mount();
    await screen.findByText("Provider B");
    await userEvent.click(await screen.findByTestId("report-style-review"));
    await screen.findByTestId("report-new"); // 報告出來了

    await userEvent.click(screen.getByTestId("report-new"));
    expect(await screen.findByTestId("report-style-review")).toBeInTheDocument();
  });

  it("沒有報告在看的時候不顯示「產生新報告」", async () => {
    mount();
    await screen.findByTestId("report-style-review");
    expect(screen.queryByTestId("report-new")).not.toBeInTheDocument();
  });

  // 實機回報「感覺快取都沒生效」的根因。統計只在掛載時抓一次，所以產生
  // 報告之後按「重新產生」回到選擇頁，它還顯示上一次那個「N 張需要重新
  // 整理」——摘要明明已經寫進 ai_summary 了，畫面卻說沒有。
  it("產生報告之後重新抓一次卡片統計", async () => {
    listTasks.mockResolvedValue([
      taskCard({ id: "a", status: "done", ai_summary: null }),
      taskCard({ id: "b", status: "done", ai_summary: null }),
    ]);

    let setHtmlFromHook: ((v: string) => void) | null = null;
    generate.mockImplementation(() => {
      // 第一階段已經把兩張的摘要寫回快取了，之後再讀就都有。
      listTasks.mockResolvedValue([
        taskCard({ id: "a", status: "done", ai_summary: "剛整理好" }),
        taskCard({ id: "b", status: "done", ai_summary: "剛整理好" }),
      ]);
      setHtmlFromHook?.("<html><title>剛產的</title></html>");
    });
    captureSetHtml = (fn) => {
      setHtmlFromHook = fn;
    };

    mount();
    await screen.findByText("Provider B");
    expect((await screen.findByTestId("report-scope-pending")).textContent).toContain("2");

    await userEvent.click(await screen.findByTestId("report-style-review"));
    await userEvent.click(await screen.findByTestId("report-new"));

    await screen.findByTestId("report-scope");
    expect(screen.queryByTestId("report-scope-pending")).not.toBeInTheDocument();
  });

  describe("刪除歷史報告", () => {
    const TWO = [
      { filename: "a.html", saved_at: 1_788_600_000, title: "第一份" },
      { filename: "b.html", saved_at: 1_788_610_000, title: "第二份" },
    ];

    it("確認之後刪掉檔案並重抓清單", async () => {
      listReports.mockResolvedValue(TWO);
      mount();
      await screen.findByText("第一份");

      listReports.mockResolvedValue([TWO[1]]);
      await userEvent.click(screen.getByTestId("report-history-delete-a.html"));

      await waitFor(() => expect(deleteReport).toHaveBeenCalledWith("p1", "a.html"));
      await waitFor(() => expect(screen.queryByText("第一份")).not.toBeInTheDocument());
      expect(screen.getByText("第二份")).toBeInTheDocument();
    });

    // 刪除不可復原，按錯的代價是一份報告消失——取消一定要真的不刪。
    it("取消確認就不刪", async () => {
      listReports.mockResolvedValue(TWO);
      confirmDialog.mockResolvedValue(false);
      mount();
      await screen.findByText("第一份");

      await userEvent.click(screen.getByTestId("report-history-delete-a.html"));
      expect(deleteReport).not.toHaveBeenCalled();
      expect(screen.getByText("第一份")).toBeInTheDocument();
    });

    // 刪掉正在看的那一份之後，畫面不能繼續顯示一份已經不存在的報告。
    it("刪掉正在看的那一份會回到風格選擇頁", async () => {
      listReports.mockResolvedValue(TWO);
      mount();
      await userEvent.click(await screen.findByText("第一份"));
      await screen.findByTestId("report-new"); // 正在看報告

      listReports.mockResolvedValue([TWO[1]]);
      await userEvent.click(screen.getByTestId("report-history-delete-a.html"));

      expect(await screen.findByTestId("report-style-review")).toBeInTheDocument();
    });
  });

  // 封存的卡片預設不進報告：第二階段的提示詞會把每一張卡都放進去，
  // 全都算的話遲早撐爆 context window。要回顧全部時才明確勾選。
  it("預設不包含已封存", async () => {
    mount();
    await screen.findByText("Provider B");
    await userEvent.click(screen.getByTestId("report-style-review"));
    expect(generate).toHaveBeenLastCalledWith("review", "p-b", false);
  });

  it("勾選之後才把已封存算進去", async () => {
    mount();
    await screen.findByText("Provider B");
    await userEvent.click(screen.getByTestId("report-include-archived"));
    await userEvent.click(screen.getByTestId("report-style-review"));
    expect(generate).toHaveBeenLastCalledWith("review", "p-b", true);
  });

  it("有原始回覆時顯示出來", async () => {
    hookState = { error: "AI 沒有產生報告文件", rawReply: "我不會做這個" };
    mount();
    expect(await screen.findByText(/我不會做這個/)).toBeInTheDocument();
  });
});
