import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const removeProject = vi.fn();
const confirmDialog = vi.fn();
const messageDialog = vi.fn();

vi.mock("../../ipc/projects", () => ({
  removeProject: (...a: unknown[]) => removeProject(...a),
  openProject: vi.fn(),
  createProject: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  confirm: (...a: unknown[]) => confirmDialog(...a),
  message: (...a: unknown[]) => messageDialog(...a),
  open: vi.fn(),
}));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { ProjectList } from "./ProjectList";
import type { ProjectInfo } from "../../ipc/projects";

const project = (over: Partial<ProjectInfo> = {}): ProjectInfo => ({
  id: "p1",
  name: "makemoney",
  description: "賺錢",
  path: "/projects/makemoney",
  status: "ok",
  counts: { planning: 2, queued: 1, running: 1, done: 3 },
  error: null,
  ...over,
});

// 清單本身是 props 進來的（TaskBoardView 擁有那份資料），所以測試直接給。
const onRefresh = vi.fn<() => Promise<void>>();

const mount = (projects: ProjectInfo[] = [project()], onOpen = vi.fn(), onReport = vi.fn()) =>
  render(
    <LocaleProvider>
      <ProjectList projects={projects} onRefresh={onRefresh} onOpen={onOpen} onReport={onReport} />
    </LocaleProvider>,
  );

describe("ProjectList", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    onRefresh.mockResolvedValue(undefined);
    removeProject.mockResolvedValue(undefined);
  });

  // 重排版後總數改成四欄各自列出（project-count-*），意圖不變：
  // 從卡片上看得出這個專案有多少工作、分佈如何。
  it("列出專案與它的工作數", async () => {
    mount();
    expect(await screen.findByText("makemoney")).toBeInTheDocument();
    const cells = screen.getAllByTestId(/^project-count-p1-/);
    const total = cells.reduce((sum, c) => sum + Number(c.textContent?.match(/\d+/)?.[0] ?? 0), 0);
    expect(total).toBe(7); // 2 + 1 + 1 + 3
  });

  // 同上：指示點改成「執行中」那一格的標示樣式，意圖不變。
  it("有執行中工作時顯示指示點", async () => {
    mount();
    await screen.findByText("makemoney");
    expect(screen.getByTestId("project-count-p1-running").className).toContain("--active");
  });

  it("完全沒有專案時顯示空狀態", async () => {
    mount([]);
    expect(await screen.findByTestId("project-empty-state")).toBeInTheDocument();
  });

  it("點專案卡片會呼叫 onOpen", async () => {
    const onOpen = vi.fn();
    mount([project()], onOpen);
    await userEvent.click(await screen.findByText("makemoney"));
    expect(onOpen).toHaveBeenCalledWith("p1");
  });

  it("遺失的專案顯示錯誤而非當機", async () => {
    mount([
      project({
        status: "missing",
        error: "專案資料夾或專案檔不存在",
        counts: { planning: 0, queued: 0, running: 0, done: 0 },
      }),
    ]);
    expect(await screen.findByTestId("project-error-p1")).toHaveTextContent("不存在");
  });

  it("移除專案：兩段式詢問，第二段答否則不刪資料夾", async () => {
    confirmDialog.mockResolvedValueOnce(true).mockResolvedValueOnce(false);
    mount([project({ counts: { planning: 2, queued: 1, running: 0, done: 3 } })]);
    await screen.findByText("makemoney");
    await userEvent.click(screen.getByTestId("project-remove-p1"));
    await waitFor(() => expect(removeProject).toHaveBeenCalledWith("p1", false));
    expect(confirmDialog).toHaveBeenCalledTimes(2);
    // 清單改由擁有者（TaskBoardView）持有，所以移除之後一定要請它重抓，
    // 否則被刪掉的卡片會繼續留在畫面上。
    await waitFor(() => expect(onRefresh).toHaveBeenCalled());
  });

  it("移除專案：第二段答是才刪資料夾", async () => {
    confirmDialog.mockResolvedValueOnce(true).mockResolvedValueOnce(true);
    mount([project({ counts: { planning: 2, queued: 1, running: 0, done: 3 } })]);
    await screen.findByText("makemoney");
    await userEvent.click(screen.getByTestId("project-remove-p1"));
    await waitFor(() => expect(removeProject).toHaveBeenCalledWith("p1", true));
  });

  it("移除專案：第一段就取消則完全不呼叫 removeProject", async () => {
    confirmDialog.mockResolvedValueOnce(false);
    mount([project({ counts: { planning: 2, queued: 1, running: 0, done: 3 } })]);
    await screen.findByText("makemoney");
    await userEvent.click(screen.getByTestId("project-remove-p1"));
    await waitFor(() => expect(confirmDialog).toHaveBeenCalledTimes(1));
    expect(removeProject).not.toHaveBeenCalled();
    expect(onRefresh).not.toHaveBeenCalled();
  });

  // 後端一定會拒絕移除還有工作在跑的專案，所以前端不該把使用者一路
  // 帶到「要連同磁碟上的資料夾一起刪除嗎？此動作無法復原」——那是整個
  // 流程最危險的一問，卻問在一個注定失敗的操作上。
  it("有工作執行中時擋下移除，一個確認對話框都不出現", async () => {
    mount([project({ counts: { planning: 0, queued: 0, running: 1, done: 0 } })]);
    await screen.findByText("makemoney");
    await userEvent.click(screen.getByTestId("project-remove-p1"));

    await waitFor(() => expect(messageDialog).toHaveBeenCalled());
    expect(confirmDialog).not.toHaveBeenCalled();
    expect(removeProject).not.toHaveBeenCalled();
  });

  // counts 是上一次 projects_list 的快照，可能在使用者按下去之前就過期
  // （工作剛開始跑）。後端的拒絕必須被顯示出來，不能靜靜失敗——那會讓
  // 使用者以為自己按錯了。
  it("後端拒絕移除時要把原因顯示出來", async () => {
    removeProject.mockRejectedValueOnce("這個專案還有工作在執行中，請先停止它們");
    confirmDialog.mockResolvedValueOnce(true).mockResolvedValueOnce(false);
    mount([project({ counts: { planning: 2, queued: 1, running: 0, done: 3 } })]);
    await screen.findByText("makemoney");
    await userEvent.click(screen.getByTestId("project-remove-p1"));

    await waitFor(() => expect(messageDialog).toHaveBeenCalled());
    expect(String(messageDialog.mock.calls[0][0])).toContain("執行中");
    expect(onRefresh).not.toHaveBeenCalled();
  });

  it("每張專案卡片都有產生報告的入口", async () => {
    mount([project({ counts: { planning: 1, queued: 0, running: 0, done: 2 } })]);
    await screen.findByText("makemoney");
    expect(screen.getByTestId("project-report-p1")).toBeInTheDocument();
  });

  // 報告按鈕在卡片主體上，很容易不小心讓點擊冒泡上去而連帶進入看板。
  // 重排版之後仍要看得出專案現況——這份資料本來就抓得到，
  // 報告視窗也已經在用，總覽卻只顯示一個總數。
  it("卡片上列出四欄各自的數量", async () => {
    mount([project({ counts: { planning: 3, queued: 2, running: 1, done: 5 } })]);
    await screen.findByText("makemoney");
    const cells = screen.getAllByTestId(/^project-count-p1-/);
    expect(cells).toHaveLength(4);
    const text = cells.map((c) => c.textContent).join("|");
    for (const n of ["3", "2", "1", "5"]) expect(text).toContain(n);
  });

  it("有工作執行中時那一格會標示出來", async () => {
    mount([project({ counts: { planning: 0, queued: 0, running: 2, done: 0 } })]);
    await screen.findByText("makemoney");
    expect(screen.getByTestId("project-count-p1-running").className).toContain("--active");
  });

  it("沒有工作執行中時不標示", async () => {
    mount([project({ counts: { planning: 1, queued: 0, running: 0, done: 0 } })]);
    await screen.findByText("makemoney");
    expect(screen.getByTestId("project-count-p1-running").className).not.toContain("--active");
  });

  it("點報告按鈕不會連帶進入該專案的看板", async () => {
    const onOpen = vi.fn();
    mount([project({ counts: { planning: 1, queued: 0, running: 0, done: 2 } })], onOpen);
    await screen.findByText("makemoney");
    await userEvent.click(screen.getByTestId("project-report-p1"));
    expect(onOpen).not.toHaveBeenCalled();
  });
});
