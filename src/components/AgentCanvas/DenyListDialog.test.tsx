import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";

vi.mock("../../ipc/serverVerify", () => ({
  getProjectDeny: vi.fn(),
  setProjectDeny: vi.fn(),
  setGlobalDeny: vi.fn(),
  resetGlobalDeny: vi.fn(),
}));
const confirmMock = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: (...a: unknown[]) => confirmMock(...a) }));

import { getProjectDeny, resetGlobalDeny, setGlobalDeny, setProjectDeny } from "../../ipc/serverVerify";
import { DenyListDialog } from "./DenyListDialog";
import { ruleProblem } from "./denyRule";

const GLOBAL = [
  { rule: "Bash(sudo:*)", note: "提權", enabled: true },
  { rule: "Bash(docker push:*)", note: "對外發佈", enabled: true },
];
const mount = () =>
  render(
    <LocaleProvider>
      <DenyListDialog projectId="remote:p1" onClose={() => {}} />
    </LocaleProvider>,
  );

beforeEach(() => {
  confirmMock.mockResolvedValue(true);
  vi.mocked(getProjectDeny).mockResolvedValue({ global: GLOBAL, overrides: [], effective: ["Bash(sudo:*)", "Bash(docker push:*)"] });
  vi.mocked(setProjectDeny).mockResolvedValue({ global: GLOBAL, overrides: [], effective: [] });
  vi.mocked(setGlobalDeny).mockResolvedValue(GLOBAL);
  vi.mocked(resetGlobalDeny).mockResolvedValue(GLOBAL);
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("ruleProblem", () => {
  it("只收 Bash(...) 的單一比對式，不能有逗號或換行", () => {
    expect(ruleProblem("Bash(make deploy:*)")).toBeNull();
    for (const bad of ["", "sudo", "Bash()", "Read(x)", "Bash(a,b:*)", "Bash(a\nb)"]) expect(ruleProblem(bad), bad).not.toBeNull();
  });
});

describe("DenyListDialog", () => {
  it("專案分頁：列出全域規則與目前生效的條數", async () => {
    mount();
    expect(await screen.findByLabelText("Bash(sudo:*)")).toBeChecked();
    expect(screen.getByTestId("deny-effective")).toHaveTextContent("2 條");
  });

  it("在這個專案移除一條全域規則（取消勾選）並儲存：送出 remove 覆寫", async () => {
    mount();
    fireEvent.click(await screen.findByLabelText("Bash(docker push:*)"));
    fireEvent.click(screen.getByRole("button", { name: "儲存" }));
    await waitFor(() => expect(setProjectDeny).toHaveBeenCalledWith("remote:p1", [{ rule: "Bash(docker push:*)", kind: "remove", enabled: true }]));
  });

  it("新增專案專屬規則；不合法的當場擋下、不呼叫 server", async () => {
    mount();
    const input = await screen.findByLabelText("新增規則（Bash(命令:*) 的形式）");
    fireEvent.change(input, { target: { value: "make deploy" } });
    fireEvent.click(screen.getByRole("button", { name: "新增" }));
    expect(screen.getByRole("alert")).toHaveTextContent("Bash(命令:*)");
    expect(setProjectDeny).not.toHaveBeenCalled();
    fireEvent.change(input, { target: { value: "Bash(make deploy:*)" } });
    fireEvent.click(screen.getByRole("button", { name: "新增" }));
    expect(await screen.findByLabelText("Bash(make deploy:*)")).toBeChecked();
    fireEvent.click(screen.getByRole("button", { name: "儲存" }));
    await waitFor(() => expect(setProjectDeny).toHaveBeenCalledWith("remote:p1", [{ rule: "Bash(make deploy:*)", kind: "add", enabled: true }]));
  });

  it("全域分頁：停用／刪除規則後儲存；還原預設要先確認", async () => {
    mount();
    fireEvent.click(await screen.findByRole("tab", { name: "全域預設" }));
    const list = screen.getByTestId("deny-global");
    fireEvent.click(list.querySelector("input[type=checkbox]")!);
    fireEvent.click(screen.getAllByRole("button", { name: "刪除" })[1]);
    fireEvent.click(screen.getByRole("button", { name: "儲存" }));
    await waitFor(() => expect(setGlobalDeny).toHaveBeenCalledWith([{ rule: "Bash(sudo:*)", note: "提權", enabled: false }]));
    confirmMock.mockResolvedValueOnce(false);
    fireEvent.click(screen.getByRole("button", { name: "還原預設" }));
    await waitFor(() => expect(confirmMock).toHaveBeenCalledTimes(1));
    expect(resetGlobalDeny).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "還原預設" }));
    await waitFor(() => expect(resetGlobalDeny).toHaveBeenCalledTimes(1));
  });

  it("server 回報哪一條不合法時顯示出來", async () => {
    vi.mocked(setProjectDeny).mockRejectedValue("Bash(x:*)：規則不能含逗號");
    mount();
    fireEvent.click(await screen.findByLabelText("Bash(sudo:*)"));
    fireEvent.click(screen.getByRole("button", { name: "儲存" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("規則不能含逗號");
  });
});
