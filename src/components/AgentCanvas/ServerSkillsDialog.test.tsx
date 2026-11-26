import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";

const role = vi.hoisted(() => ({ value: "admin" as "admin" | "member" }));
vi.mock("../../hooks/useIntakeRemote", () => ({ useIntakeRemote: () => ({ connected: true, online: true, addr: "x", name: "我", role: role.value, error: null }) }));
vi.mock("../../ipc/serverSkills", () => ({ listServerSkills: vi.fn(), uploadServerSkill: vi.fn(), deleteServerSkill: vi.fn(), setAgentServerSkills: vi.fn() }));
vi.mock("../../ipc/agents", () => ({ readSkill: vi.fn() }));
const dialog = vi.hoisted(() => ({ open: vi.fn(), confirm: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => dialog);

import { readSkill } from "../../ipc/agents";
import { deleteServerSkill, listServerSkills, uploadServerSkill, type ServerSkill } from "../../ipc/serverSkills";
import { ServerSkillsDialog } from "./ServerSkillsDialog";

const sk = (id: string, name: string, description = ""): ServerSkill => ({ id, name, description, files: 2, bytes: 4096, uploaded_by: "小華", updated_at: 1 });

const mount = (current: string[] = []) => {
  const onSave = vi.fn().mockResolvedValue(undefined);
  const onClose = vi.fn();
  render(
    <LocaleProvider>
      <ServerSkillsDialog agentName="工程師1" current={current} onSave={onSave} onClose={onClose} />
    </LocaleProvider>,
  );
  return { onSave, onClose };
};

beforeEach(() => {
  role.value = "admin";
  vi.mocked(listServerSkills).mockResolvedValue([sk("a", "寫測試", "先寫紅燈"), sk("b", "審查清單")]);
  dialog.confirm.mockResolvedValue(true);
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("ServerSkillsDialog", () => {
  it("管理員：列出 server 技能、依清單順序儲存勾選", async () => {
    const { onSave, onClose } = mount(["b"]);
    expect(await screen.findByLabelText("審查清單")).toBeChecked();
    expect(screen.getByText("先寫紅燈")).toBeInTheDocument();
    fireEvent.click(screen.getByLabelText("寫測試"));
    fireEvent.click(screen.getByRole("button", { name: "儲存" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith(["a", "b"]));
    expect(onClose).toHaveBeenCalled();
  });

  it("成員：只能看，不能勾選、上傳、刪除或儲存", async () => {
    role.value = "member";
    mount(["a"]);
    expect(await screen.findByLabelText("寫測試")).toBeDisabled();
    expect(screen.getByText(/只有管理員能上傳技能/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /從本機資料夾加入/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /^刪除/ })).toBeNull();
    expect(screen.queryByRole("button", { name: "儲存" })).toBeNull();
  });

  it("從本機資料夾加入：找到幾個就上傳幾個、更新清單並自動勾選", async () => {
    dialog.open.mockResolvedValue("/skills");
    vi.mocked(readSkill).mockResolvedValue([
      { name: "新技能", description: "", path: "/skills/new", source: "folder" },
    ]);
    vi.mocked(uploadServerSkill).mockResolvedValue(sk("c", "新技能"));
    vi.mocked(listServerSkills).mockResolvedValueOnce([sk("a", "寫測試")]).mockResolvedValueOnce([sk("a", "寫測試"), sk("c", "新技能")]);
    const { onSave } = mount();
    await screen.findByLabelText("寫測試");
    fireEvent.click(screen.getByRole("button", { name: /從本機資料夾加入/ }));
    expect(await screen.findByLabelText("新技能")).toBeChecked();
    expect(uploadServerSkill).toHaveBeenCalledWith("/skills/new");
    expect(screen.getByText(/已上傳並勾選：新技能/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "儲存" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith(["c"]));
  });

  it("資料夾裡沒有技能：說明原因、不上傳", async () => {
    dialog.open.mockResolvedValue("/empty");
    vi.mocked(readSkill).mockResolvedValue([]);
    mount();
    await screen.findByLabelText("寫測試");
    fireEvent.click(screen.getByRole("button", { name: /從本機資料夾加入/ }));
    expect(await screen.findByRole("alert")).toHaveTextContent("找不到含 SKILL.md");
    expect(uploadServerSkill).not.toHaveBeenCalled();
  });

  it("server 拒絕上傳時顯示原因", async () => {
    dialog.open.mockResolvedValue("/skills");
    vi.mocked(readSkill).mockResolvedValue([{ name: "太大", description: "", path: "/skills/big", source: "folder" }]);
    vi.mocked(uploadServerSkill).mockRejectedValue("太大.bin 太大：3000 KB（單檔上限 2048 KB）");
    mount();
    await screen.findByLabelText("寫測試");
    fireEvent.click(screen.getByRole("button", { name: /從本機資料夾加入/ }));
    expect(await screen.findByRole("alert")).toHaveTextContent("單檔上限");
  });

  it("刪除要先確認；確認後從清單與勾選移除，取消就什麼都不做", async () => {
    mount(["a"]);
    await screen.findByLabelText("寫測試");
    dialog.confirm.mockResolvedValueOnce(false);
    fireEvent.click(screen.getByRole("button", { name: "刪除 寫測試" }));
    await waitFor(() => expect(dialog.confirm).toHaveBeenCalledTimes(1));
    expect(deleteServerSkill).not.toHaveBeenCalled();
    vi.mocked(deleteServerSkill).mockResolvedValue(undefined);
    fireEvent.click(screen.getByRole("button", { name: "刪除 寫測試" }));
    await waitFor(() => expect(deleteServerSkill).toHaveBeenCalledWith("a"));
    await waitFor(() => expect(screen.queryByLabelText("寫測試")).toBeNull());
  });

  it("一位 Agent 最多 12 個技能：第 13 個勾不上並提示", async () => {
    vi.mocked(listServerSkills).mockResolvedValue(Array.from({ length: 13 }, (_, i) => sk(`s${i}`, `技能${i}`)));
    mount(Array.from({ length: 12 }, (_, i) => `s${i}`));
    const last = await screen.findByLabelText("技能12");
    fireEvent.click(last);
    expect(last).not.toBeChecked();
    expect(screen.getByRole("alert")).toHaveTextContent("最多 12 個");
  });
});
