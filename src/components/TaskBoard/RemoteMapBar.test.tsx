import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const intakeBoardMapGet = vi.fn();
const intakeBoardMapSet = vi.fn();
const listProjects = vi.fn();
vi.mock("../../ipc/intake", () => ({
  intakeBoardMapGet: (...a: unknown[]) => intakeBoardMapGet(...a),
  intakeBoardMapSet: (...a: unknown[]) => intakeBoardMapSet(...a),
  intakeBoardClaim: vi.fn(),
  intakeBoardRelease: vi.fn(),
}));
const openProject = vi.fn();
vi.mock("../../ipc/projects", () => ({ listProjects: (...a: unknown[]) => listProjects(...a), openProject: (...a: unknown[]) => openProject(...a) }));
vi.mock("../../ipc/tasks", () => ({}));
vi.mock("../../ipc/agents", () => ({ listAgents: vi.fn() }));
const openDialog = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn(), message: vi.fn(), open: (...a: unknown[]) => openDialog(...a) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { RemoteMapBar } from "./ProjectBoard";

const proj = (id: string, name: string, status = "ok") => ({ id, name, description: "", path: `/p/${name}`, status, counts: { planning: 0, queued: 0, running: 0, done: 0 }, error: null });

beforeEach(() => {
  vi.clearAllMocks();
  listProjects.mockResolvedValue([proj("local-1", "本機專案"), proj("remote:srv-1", "🌐 範例科技"), proj("unopened:/x", "壞掉的", "missing")]);
  intakeBoardMapGet.mockResolvedValue(null);
  intakeBoardMapSet.mockResolvedValue(undefined);
});

const mount = () => render(<LocaleProvider><RemoteMapBar serverProjectId="srv-1" /></LocaleProvider>);

const PROJECT = "用哪個本機專案處理這個專案的卡片";
const detailsOf = () => document.querySelector("details") as HTMLDetailsElement;

describe("RemoteMapBar（在我的電腦處理）", () => {
  it("還沒設定：展開並提醒；只列本機的看板專案，不列 server 的與打不開的；沒有路徑欄位可填", async () => {
    mount();
    expect(await screen.findByText(/還沒設定/)).toBeTruthy();
    expect(detailsOf().open).toBe(true);
    expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual([PROJECT, "本機專案"]);
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.queryByRole("button", { name: "儲存" })).toBeNull();
  });

  it("選了專案就直接存（不用再按儲存），顯示已儲存與該專案的資料夾", async () => {
    mount();
    await screen.findByText(/還沒設定/);
    fireEvent.change(screen.getByLabelText(PROJECT), { target: { value: "local-1" } });
    await waitFor(() => expect(intakeBoardMapSet).toHaveBeenCalledWith("srv-1", "local-1"));
    expect(await screen.findByRole("status")).toHaveTextContent("已儲存");
    expect(screen.getByText(/資料夾：\/p\/本機專案/)).toBeTruthy();
    expect(screen.queryByText(/還沒設定/)).toBeNull();
  });

  it("已有設定：預設收起，摘要寫出對應的專案；使用者可以展開", async () => {
    intakeBoardMapGet.mockResolvedValue({ board_project_id: "local-1" });
    mount();
    await screen.findByText(/已對應：本機專案/);
    expect(detailsOf().open).toBe(false);
    expect((screen.getByLabelText(PROJECT) as HTMLSelectElement).value).toBe("local-1");
    expect(screen.queryByText(/還沒設定/)).toBeNull();
  });

  it("本機拒絕時（例如資料夾不見了）把原因顯示出來，且不會當成已儲存", async () => {
    intakeBoardMapSet.mockRejectedValue("這台電腦上找不到這個專案的資料夾：/nope");
    mount();
    await screen.findByText(/還沒設定/);
    fireEvent.change(screen.getByLabelText(PROJECT), { target: { value: "local-1" } });
    expect((await screen.findByRole("alert")).textContent).toContain("找不到這個專案的資料夾");
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("沒有任何本機專案：說明並給「開啟現有的本機專案」，開完自動選起來存好", async () => {
    listProjects.mockResolvedValueOnce([proj("remote:srv-1", "🌐 範例科技")]);
    openDialog.mockResolvedValue("/Users/me/repo/.aitprj");
    openProject.mockResolvedValue("local-9");
    mount();
    expect(await screen.findByText(/還沒有本機專案/)).toBeTruthy();
    expect(screen.queryByRole("combobox")).toBeNull();
    listProjects.mockResolvedValue([proj("local-9", "我的專案")]);
    fireEvent.click(screen.getByRole("button", { name: "開啟現有的本機專案…" }));
    await waitFor(() => expect(intakeBoardMapSet).toHaveBeenCalledWith("srv-1", "local-9"));
    expect(openProject).toHaveBeenCalledWith("/Users/me/repo/.aitprj");
    expect((await screen.findByLabelText(PROJECT) as HTMLSelectElement).value).toBe("local-9");
  });

  it("取消選檔：什麼都不動", async () => {
    openDialog.mockResolvedValue(null);
    mount();
    await screen.findByText(/還沒設定/);
    fireEvent.click(screen.getByRole("button", { name: "開啟現有的本機專案…" }));
    await waitFor(() => expect(openDialog).toHaveBeenCalled());
    expect(openProject).not.toHaveBeenCalled();
    expect(intakeBoardMapSet).not.toHaveBeenCalled();
  });
});
