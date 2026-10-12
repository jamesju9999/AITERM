import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ComputerApprovalRequest } from "../../ipc/computer";

let emit: ((r: ComputerApprovalRequest) => void) | undefined;
const resolveMock = vi.fn((_id: string, _d: string) => Promise.resolve(true));
vi.mock("../../ipc/computer", () => ({
  onComputerApproval: (cb: (r: ComputerApprovalRequest) => void) => {
    emit = cb;
    return Promise.resolve(() => {});
  },
  resolveComputerApproval: (id: string, d: string) => resolveMock(id, d),
}));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { ComputerApprovalDialog } from "./index";

const first: ComputerApprovalRequest = { id: "r1", kind: "first_use", app: "Notes", action: "點擊", detail: "" };
const cmdReq: ComputerApprovalRequest = { id: "r3", kind: "command", app: "終端機", action: "執行指令", detail: "sudo rm -rf /tmp/x\n風險：危險（含 sudo）" };
const danger: ComputerApprovalRequest = { id: "r2", kind: "destructive", app: "Mail", action: "點擊", detail: "按鈕「Delete」" };

async function mount() {
  render(<LocaleProvider><ComputerApprovalDialog /></LocaleProvider>);
  await act(async () => {});
}

beforeEach(() => { emit = undefined; resolveMock.mockClear(); });

describe("ComputerApprovalDialog", () => {
  it("沒有請求時不顯示任何東西", async () => {
    await mount();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("首次授權：三個選項，按「永遠允許」回覆 allow_always 並關閉", async () => {
    await mount();
    act(() => emit!(first));
    expect(screen.getByRole("dialog", { name: /Notes/ })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "永遠允許此 App" }));
    expect(resolveMock).toHaveBeenCalledWith("r1", "allow_always");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("破壞性確認：只有拒絕／允許，沒有「永遠允許」，並顯示被點的元件", async () => {
    await mount();
    act(() => emit!(danger));
    expect(screen.queryByRole("button", { name: "永遠允許此 App" })).toBeNull();
    expect(screen.getByText(/按鈕「Delete」/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "拒絕" }));
    expect(resolveMock).toHaveBeenCalledWith("r2", "deny");
  });

  it("多個請求排隊，一次一個；同 id 的重複事件只算一次", async () => {
    await mount();
    act(() => { emit!(first); emit!(first); emit!(danger); });
    await userEvent.click(screen.getByRole("button", { name: "僅此一次" }));
    expect(resolveMock).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("dialog", { name: "確認破壞性操作" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "允許" }));
    expect(resolveMock).toHaveBeenLastCalledWith("r2", "allow_once");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("回覆失敗（已逾時）也會關掉對話框，不卡住後面的請求", async () => {
    resolveMock.mockImplementationOnce(() => Promise.reject("timeout"));
    await mount();
    act(() => emit!(first));
    await userEvent.click(screen.getByRole("button", { name: "拒絕" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("終端機指令：顯示指令與風險，只有拒絕／允許這一次，沒有永遠允許", async () => {
    await mount();
    act(() => emit!(cmdReq));
    expect(screen.getByRole("dialog", { name: "允許 AI 在終端機執行這個指令？" })).toBeInTheDocument();
    expect(screen.getByText(/sudo rm -rf \/tmp\/x/)).toBeInTheDocument();
    expect(screen.getByText(/風險：危險/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "永遠允許此 App" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "允許" }));
    expect(resolveMock).toHaveBeenCalledWith("r3", "allow_once");
  });

  it("終端機指令：按拒絕回覆 deny", async () => {
    await mount();
    act(() => emit!(cmdReq));
    await userEvent.click(screen.getByRole("button", { name: "拒絕" }));
    expect(resolveMock).toHaveBeenCalledWith("r3", "deny");
  });
});
