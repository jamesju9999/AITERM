import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";

const role = vi.hoisted(() => ({ value: "admin" as "admin" | "member" }));
vi.mock("../../hooks/useIntakeRemote", () => ({ useIntakeRemote: () => ({ connected: true, online: true, addr: "x", name: "我", role: role.value, error: null }) }));
vi.mock("../../ipc/serverDb", () => ({ listServerDbConns: vi.fn(), shareDbConnection: vi.fn(), deleteServerDbConn: vi.fn(), testServerDbConn: vi.fn() }));
vi.mock("../../ipc/db", () => ({ dbListConnections: vi.fn() }));
const dialog = vi.hoisted(() => ({ confirm: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => dialog);

import { dbListConnections, type DbConnectionInfo } from "../../ipc/db";
import { deleteServerDbConn, listServerDbConns, shareDbConnection, testServerDbConn } from "../../ipc/serverDb";
import { ServerDbSkillDialog } from "./ServerDbSkillDialog";

const local = (id: string, name: string, db_type: string): DbConnectionInfo => ({ id, name, db_type: db_type as DbConnectionInfo["db_type"], host: "h", port: 1, database: "app", username: "u", is_connected: false });

const mount = (current: { connections: string[]; mode: "read" | "approve" | "write" } | null = null) => {
  const onSave = vi.fn().mockResolvedValue(undefined);
  const onClose = vi.fn();
  render(
    <LocaleProvider>
      <ServerDbSkillDialog agentName="工程師1" current={current} onSave={onSave} onClose={onClose} />
    </LocaleProvider>,
  );
  return { onSave, onClose };
};

beforeEach(() => {
  role.value = "admin";
  vi.mocked(listServerDbConns).mockResolvedValue([{ id: "s1", name: "正式機", db_type: "postgresql", database: "app" }]);
  vi.mocked(dbListConnections).mockResolvedValue([local("l1", "測試機", "mysql"), local("l2", "舊系統", "mssql"), local("l3", "正式機", "postgresql")]);
  dialog.confirm.mockResolvedValue(true);
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("ServerDbSkillDialog", () => {
  it("管理員：勾選 server 連線、選權限並儲存", async () => {
    const { onSave, onClose } = mount();
    fireEvent.click(await screen.findByLabelText("正式機"));
    fireEvent.click(screen.getByLabelText("寫入每次核准"));
    fireEvent.click(screen.getByRole("button", { name: "儲存" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith({ connections: ["s1"], mode: "approve" }));
    expect(onClose).toHaveBeenCalled();
  });

  it("沒選任何連線不能儲存；已有技能時可以移除", async () => {
    const { onSave } = mount({ connections: ["s1"], mode: "read" });
    await screen.findByLabelText("正式機");
    fireEvent.click(screen.getByLabelText("正式機"));
    expect(screen.getByRole("button", { name: "儲存" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "移除技能" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith(null));
  });

  it("分享本機連線：已在 server 的同名連線不再列出，不支援的類型停用，分享後自動勾選", async () => {
    vi.mocked(shareDbConnection).mockResolvedValue({ id: "s2", name: "測試機", db_type: "mysql", database: "app" });
    vi.mocked(listServerDbConns).mockResolvedValueOnce([{ id: "s1", name: "正式機", db_type: "postgresql", database: "app" }]).mockResolvedValueOnce([
      { id: "s1", name: "正式機", db_type: "postgresql", database: "app" },
      { id: "s2", name: "測試機", db_type: "mysql", database: "app" },
    ]);
    const { onSave } = mount();
    await screen.findByLabelText("正式機");
    const share = await screen.findByTestId("server-db-share");
    expect(share.textContent).toContain("測試機");
    expect(share.textContent).not.toContain("· postgresql");
    expect(screen.getByRole("button", { name: "分享 舊系統" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "分享 測試機" }));
    expect(await screen.findByLabelText("測試機")).toBeChecked();
    expect(shareDbConnection).toHaveBeenCalledWith("l1");
    fireEvent.click(screen.getByRole("button", { name: "儲存" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith({ connections: ["s2"], mode: "read" }));
  });

  it("測試連線顯示成功或原因；刪除要先確認", async () => {
    vi.mocked(testServerDbConn).mockRejectedValueOnce("連不上 正式機：connection refused");
    mount();
    await screen.findByLabelText("正式機");
    fireEvent.click(screen.getByRole("button", { name: "測試 正式機" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("connection refused");
    vi.mocked(testServerDbConn).mockResolvedValueOnce(undefined);
    fireEvent.click(screen.getByRole("button", { name: "測試 正式機" }));
    expect(await screen.findByText(/正式機 連線成功/)).toBeInTheDocument();
    dialog.confirm.mockResolvedValueOnce(false);
    fireEvent.click(screen.getByRole("button", { name: "刪除 正式機" }));
    await waitFor(() => expect(dialog.confirm).toHaveBeenCalledTimes(1));
    expect(deleteServerDbConn).not.toHaveBeenCalled();
    vi.mocked(deleteServerDbConn).mockResolvedValue(undefined);
    vi.mocked(listServerDbConns).mockResolvedValue([]);
    fireEvent.click(screen.getByRole("button", { name: "刪除 正式機" }));
    await waitFor(() => expect(deleteServerDbConn).toHaveBeenCalledWith("s1"));
  });

  it("成員：只能看，不能勾選、分享、測試、刪除或儲存", async () => {
    role.value = "member";
    mount({ connections: ["s1"], mode: "approve" });
    expect(await screen.findByLabelText("正式機")).toBeDisabled();
    expect(screen.getByLabelText("寫入每次核准")).toBeDisabled();
    expect(screen.queryByTestId("server-db-share")).toBeNull();
    expect(screen.queryByRole("button", { name: /^測試/ })).toBeNull();
    expect(screen.queryByRole("button", { name: "儲存" })).toBeNull();
    expect(screen.getByText(/只有管理員能分享連線/)).toBeInTheDocument();
    expect(dbListConnections).not.toHaveBeenCalled();
  });
});
