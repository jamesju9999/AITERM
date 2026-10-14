import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const status = vi.fn();
const audit = vi.fn();
const revoke = vi.fn((_k: string) => Promise.resolve());
const setEnabled = vi.fn((_e: boolean) => Promise.resolve());
const openPane = vi.fn((_p: string) => Promise.resolve());
vi.mock("../../ipc/computer", () => ({
  computerStatus: () => status(),
  computerAuditTail: () => audit(),
  computerRevokeApp: (k: string) => revoke(k),
  computerSetEnabled: (e: boolean) => setEnabled(e),
  computerOpenPrivacySettings: (p: string) => openPane(p),
}));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { ComputerUsePage } from "./ComputerUsePage";

function mount() {
  render(<LocaleProvider><ComputerUsePage /></LocaleProvider>);
}

beforeEach(() => {
  status.mockReset();
  audit.mockReset();
  revoke.mockClear();
  setEnabled.mockClear();
  openPane.mockClear();
  status.mockResolvedValue({ enabled: false, accessibility: true, screen_recording: false, allowed_apps: ["com.apple.Notes"] });
  audit.mockResolvedValue([{ ts: 1760000000, tool: "computer_click", app: "Notes", detail: "element 0.1", outcome: "denied" }]);
});

describe("ComputerUsePage", () => {
  it("總開關反映後端狀態，切換時直接存檔（不靠 MCP 工具伺服器頁的儲存）", async () => {
    mount();
    const box = await screen.findByRole("checkbox", { name: /允許 AI 操作這台電腦/ });
    expect(box).not.toBeChecked();
    status.mockResolvedValue({ enabled: true, accessibility: true, screen_recording: false, allowed_apps: [] });
    await userEvent.click(box);
    expect(setEnabled).toHaveBeenCalledWith(true);
    await waitFor(() => expect(box).toBeChecked());
  });

  it("說明這個開關同時管 Ask AI 與 MCP 工具伺服器", async () => {
    mount();
    expect(await screen.findByText(/MCP 工具伺服器停用時，Ask AI 仍然可以操作電腦/)).toBeInTheDocument();
  });

  it("缺的權限顯示「尚未授權」與前往按鈕；已授權的沒有按鈕", async () => {
    mount();
    expect(await screen.findByText(/螢幕錄製權限：尚未授權/)).toBeInTheDocument();
    expect(screen.getByText(/輔助使用權限：已授權/)).toBeInTheDocument();
    const buttons = screen.getAllByRole("button", { name: "前往系統設定" });
    expect(buttons).toHaveLength(1);
    await userEvent.click(buttons[0]);
    expect(openPane).toHaveBeenCalledWith("screen");
  });

  it("撤銷已允許的 App：呼叫後端並重新讀取", async () => {
    mount();
    await userEvent.click(await screen.findByRole("button", { name: "撤銷" }));
    expect(revoke).toHaveBeenCalledWith("com.apple.Notes");
    await waitFor(() => expect(status).toHaveBeenCalledTimes(2));
  });

  it("顯示稽核紀錄，含結果", async () => {
    mount();
    const row = (await screen.findByText("element 0.1")).closest("li")!;
    expect(row).toHaveTextContent(/click.*Notes.*element 0\.1.*denied/);
  });

  it("出錯的紀錄：結果欄只標 error，錯誤訊息另起一行", async () => {
    audit.mockResolvedValue([{ ts: 1760000000, tool: "computer_click_text", app: "Safari", detail: "text=Safari", outcome: "error: 畫面上找不到「Safari」" }]);
    mount();
    expect(await screen.findByText("畫面上找不到「Safari」")).toHaveClass("computer-audit-error");
    expect(screen.getByText("error")).toHaveClass("computer-audit-outcome--error");
  });

  it("後端讀取失敗：不崩潰，仍顯示頁面標題", async () => {
    const err = vi.spyOn(console, "error").mockImplementation(() => {});
    status.mockRejectedValue("boom");
    mount();
    expect(await screen.findByRole("heading", { name: /電腦操作/ })).toBeInTheDocument();
    err.mockRestore();
  });
});
