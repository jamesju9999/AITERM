import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";

const role = vi.hoisted(() => ({ value: "admin" as "admin" | "member" }));
vi.mock("../../hooks/useIntakeRemote", () => ({ useIntakeRemote: () => ({ connected: true, online: true, addr: "x", name: "我", role: role.value, error: null }) }));
vi.mock("../../ipc/serverSystems", () => ({ listServerSystems: vi.fn(), shareSystem: vi.fn(), shareSystemSession: vi.fn(), deleteServerSystem: vi.fn(), deleteServerSystemSession: vi.fn() }));
vi.mock("../../ipc/systems", () => ({ systemsList: vi.fn() }));
vi.mock("../../ipc/serverLive", () => ({ startRemoteLogin: vi.fn(), cancelRemoteLogin: vi.fn(), openLive: vi.fn(async () => ({ send: vi.fn(), close: vi.fn() })) }));
const dialog = vi.hoisted(() => ({ confirm: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => dialog);

import { deleteServerSystem, deleteServerSystemSession, listServerSystems, shareSystem, shareSystemSession, type ServerSystem } from "../../ipc/serverSystems";
import { startRemoteLogin } from "../../ipc/serverLive";
import { systemsList, type SystemConnection } from "../../ipc/systems";
import { ServerSystemsDialog } from "./ServerSystemsDialog";

const srv = (id: string, name: string, over: Partial<ServerSystem> = {}): ServerSystem => ({ id, name, start_url: `https://${id}.example.com/`, mode: "read_only", enabled: true, session: null, ...over });
const loc = (id: string, name: string, over: Partial<SystemConnection> = {}): SystemConnection => ({ id, name, start_url: `https://${id}.example.com/`, allowed_hosts: [`${id}.example.com`], mode: "read_only", rules: [], enabled: true, ...over });

const mount = (current: string[] = []) => {
  const onSave = vi.fn().mockResolvedValue(undefined);
  const onClose = vi.fn();
  render(
    <LocaleProvider>
      <ServerSystemsDialog agentName="工程師1" current={current} onSave={onSave} onClose={onClose} />
    </LocaleProvider>,
  );
  return { onSave, onClose };
};

beforeEach(() => {
  role.value = "admin";
  vi.mocked(listServerSystems).mockResolvedValue([srv("erp", "ERP", { session: { exported_by: "小華", exported_at: 1_900_000_000, cookie_count: 7 } }), srv("crm", "CRM")]);
  vi.mocked(systemsList).mockResolvedValue([loc("erp", "ERP"), loc("hr", "HR"), loc("ruled", "規則系統", { mode: "rules" }), loc("app", "記事本", { kind: "desktop" })]);
  dialog.confirm.mockResolvedValue(true);
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("ServerSystemsDialog", () => {
  it("管理員：顯示每個系統的權限與 session 狀態、勾選後儲存；沒有 session 的會提醒", async () => {
    const { onSave, onClose } = mount(["erp"]);
    expect(await screen.findByLabelText("ERP")).toBeChecked();
    expect(screen.getByTestId("session-erp")).toHaveTextContent("小華");
    expect(screen.getByTestId("session-erp")).toHaveTextContent("7 個 cookie");
    expect(screen.getByTestId("session-crm")).toHaveTextContent("還沒有分享登入 session");
    fireEvent.click(screen.getByLabelText("CRM"));
    expect(screen.getByText(/被勾選的系統裡有還沒分享 session/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "儲存" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith(["erp", "crm"]));
    expect(onClose).toHaveBeenCalled();
  });

  it("分享本機系統：已在 server 的同名不再列出、桌面 App 型不列、規則模式停用；分享後自動勾選並提示接著登入", async () => {
    vi.mocked(shareSystem).mockResolvedValue(srv("hr", "HR"));
    vi.mocked(listServerSystems).mockResolvedValueOnce([srv("erp", "ERP")]).mockResolvedValueOnce([srv("erp", "ERP"), srv("hr", "HR")]);
    const { onSave } = mount();
    await screen.findByLabelText("ERP");
    const share = await screen.findByTestId("server-systems-share");
    expect(share.textContent).toContain("HR");
    expect(share.textContent).not.toContain("記事本");
    expect(screen.getByRole("button", { name: "分享 規則系統" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "分享 HR" }));
    expect(await screen.findByLabelText("HR")).toBeChecked();
    expect(shareSystem).toHaveBeenCalledWith("hr");
    expect(screen.getByText(/先在它的瀏覽器視窗登入，再按「分享 session」/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "儲存" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith(["hr"]));
  });

  it("分享 session：用名稱找到本機對應的系統，成功顯示 cookie 數；找不到本機對應時說明原因", async () => {
    vi.mocked(shareSystemSession).mockResolvedValue(7);
    mount();
    await screen.findByLabelText("ERP");
    fireEvent.click(screen.getByRole("button", { name: "分享 session ERP" }));
    expect(await screen.findByText(/ERP 的登入 session 已分享（7 個 cookie）/)).toBeInTheDocument();
    expect(shareSystemSession).toHaveBeenCalledWith("erp");
    fireEvent.click(screen.getByRole("button", { name: "分享 session CRM" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("這台電腦上沒有叫「CRM」的系統連線");
    expect(shareSystemSession).toHaveBeenCalledTimes(1);
  });

  it("分享 session 失敗（例如瀏覽器沒登入）顯示 server 或本機給的原因", async () => {
    vi.mocked(shareSystemSession).mockRejectedValue("這個系統的瀏覽器裡沒有任何 cookie，請先在它的瀏覽器視窗登入再分享");
    mount();
    await screen.findByLabelText("ERP");
    fireEvent.click(screen.getByRole("button", { name: "分享 session ERP" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("請先在它的瀏覽器視窗登入");
  });

  it("刪除 session 與刪除系統：後者要先確認", async () => {
    vi.mocked(deleteServerSystemSession).mockResolvedValue(undefined);
    vi.mocked(deleteServerSystem).mockResolvedValue(undefined);
    mount();
    await screen.findByLabelText("ERP");
    fireEvent.click(screen.getByRole("button", { name: "刪除 session ERP" }));
    await waitFor(() => expect(deleteServerSystemSession).toHaveBeenCalledWith("erp"));
    dialog.confirm.mockResolvedValueOnce(false);
    fireEvent.click(screen.getByRole("button", { name: "刪除 ERP" }));
    await waitFor(() => expect(dialog.confirm).toHaveBeenCalledTimes(1));
    expect(deleteServerSystem).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "刪除 ERP" }));
    await waitFor(() => expect(deleteServerSystem).toHaveBeenCalledWith("erp"));
  });

  it("成員：只能看，不能勾選、分享、刪除或儲存，也不去讀本機系統清單", async () => {
    role.value = "member";
    mount(["erp"]);
    expect(await screen.findByLabelText("ERP")).toBeDisabled();
    expect(screen.queryByTestId("server-systems-share")).toBeNull();
    expect(screen.queryByRole("button", { name: /分享 session/ })).toBeNull();
    expect(screen.queryByRole("button", { name: "儲存" })).toBeNull();
    expect(screen.getByText(/只有管理員能分享系統與 session/)).toBeInTheDocument();
    expect(systemsList).not.toHaveBeenCalled();
  });

  it("已停用的系統不能勾選", async () => {
    vi.mocked(listServerSystems).mockResolvedValue([srv("a", "停用的", { enabled: false }), srv("b", "正常的")]);
    mount();
    expect(await screen.findByLabelText("停用的")).toBeDisabled();
    expect(screen.getByLabelText("正常的")).toBeEnabled();
  });

  it("一位 Agent 最多 12 個：第 13 個勾不上並提示", async () => {
    vi.mocked(listServerSystems).mockResolvedValue(Array.from({ length: 13 }, (_, i) => srv(`s${i}`, `系統${i}`)));
    mount(Array.from({ length: 12 }, (_, i) => `s${i}`));
    const last = await screen.findByLabelText("系統12");
    fireEvent.click(last);
    expect(last).not.toBeChecked();
    expect(screen.getByRole("alert")).toHaveTextContent("最多 12 個");
  });

  it("在 server 上登入：先開瀏覽器再顯示畫面；已經有人在登入就直接去看；成員沒有這個按鈕", async () => {
    vi.mocked(startRemoteLogin).mockResolvedValue(undefined);
    mount();
    await screen.findByLabelText("ERP");
    fireEvent.click(screen.getByRole("button", { name: "在 server 上登入 ERP" }));
    expect(await screen.findByTestId("live-view-dialog")).toBeInTheDocument();
    expect(startRemoteLogin).toHaveBeenCalledWith("erp");
    expect(screen.getAllByText(/帳號密碼會經過 server 的記憶體/).length).toBeGreaterThan(0);
    cleanup();
    vi.mocked(startRemoteLogin).mockRejectedValue("這個系統已經有人在遠端登入了");
    mount();
    await screen.findByLabelText("CRM");
    fireEvent.click(screen.getByRole("button", { name: "在 server 上登入 CRM" }));
    expect(await screen.findByTestId("live-view-dialog")).toBeInTheDocument();
    cleanup();
    vi.mocked(startRemoteLogin).mockRejectedValue("這個系統連線已停用");
    mount();
    await screen.findByLabelText("ERP");
    fireEvent.click(screen.getByRole("button", { name: "在 server 上登入 ERP" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("已停用");
    expect(screen.queryByTestId("live-view-dialog")).toBeNull();
    cleanup();
    role.value = "member";
    mount();
    await screen.findByLabelText("ERP");
    expect(screen.queryByRole("button", { name: /在 server 上登入/ })).toBeNull();
  });
});
