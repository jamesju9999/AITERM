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
vi.mock("../../ipc/projects", () => ({ listProjects: (...a: unknown[]) => listProjects(...a) }));
vi.mock("../../ipc/tasks", () => ({}));
vi.mock("../../ipc/agents", () => ({ listAgents: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn(), message: vi.fn() }));
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

describe("RemoteMapBar（我電腦上的專案）", () => {
  it("還沒設定時提醒要先設定；只列本機的看板專案，不列 server 的與打不開的", async () => {
    mount();
    expect(await screen.findByText(/還沒設定「我電腦上的專案」/)).toBeTruthy();
    const options = screen.getAllByRole("option").map((o) => o.textContent);
    expect(options).toEqual(["影子卡片放在哪個本機看板專案", "本機專案"]);
    expect(screen.getByRole("button", { name: "儲存" }).matches(":disabled")).toBe(true);
  });

  it("選好專案與資料夾後存起來，並顯示已儲存", async () => {
    mount();
    await screen.findByText(/還沒設定/);
    fireEvent.change(screen.getByLabelText("影子卡片放在哪個本機看板專案"), { target: { value: "local-1" } });
    fireEvent.change(screen.getByLabelText("在哪個資料夾執行（這台電腦上的路徑）"), { target: { value: " /Users/me/repo " } });
    fireEvent.click(screen.getByRole("button", { name: "儲存" }));
    await waitFor(() => expect(intakeBoardMapSet).toHaveBeenCalledWith("srv-1", "local-1", "/Users/me/repo"));
    expect(await screen.findByRole("status")).toHaveTextContent("已儲存");
    expect(screen.queryByText(/還沒設定/)).toBeNull();
  });

  it("已有設定就帶出來，不再提醒", async () => {
    intakeBoardMapGet.mockResolvedValue({ board_project_id: "local-1", work_dir: "/Users/me/repo" });
    mount();
    await waitFor(() => expect((screen.getByLabelText("在哪個資料夾執行（這台電腦上的路徑）") as HTMLInputElement).value).toBe("/Users/me/repo"));
    expect((screen.getByLabelText("影子卡片放在哪個本機看板專案") as HTMLSelectElement).value).toBe("local-1");
    expect(screen.queryByText(/還沒設定/)).toBeNull();
  });

  it("本機拒絕時（例如找不到資料夾）把原因顯示出來", async () => {
    intakeBoardMapSet.mockRejectedValue("這台電腦上找不到這個資料夾：/nope");
    mount();
    await screen.findByText(/還沒設定/);
    fireEvent.change(screen.getByLabelText("影子卡片放在哪個本機看板專案"), { target: { value: "local-1" } });
    fireEvent.change(screen.getByLabelText("在哪個資料夾執行（這台電腦上的路徑）"), { target: { value: "/nope" } });
    fireEvent.click(screen.getByRole("button", { name: "儲存" }));
    expect((await screen.findByRole("alert")).textContent).toContain("找不到這個資料夾");
  });
});
