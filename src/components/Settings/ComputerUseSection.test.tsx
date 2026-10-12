import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const status = vi.fn();
const audit = vi.fn();
const revoke = vi.fn((_k: string) => Promise.resolve());
const openPane = vi.fn((_p: string) => Promise.resolve());
vi.mock("../../ipc/computer", () => ({
  computerStatus: () => status(),
  computerAuditTail: () => audit(),
  computerRevokeApp: (k: string) => revoke(k),
  computerOpenPrivacySettings: (p: string) => openPane(p),
}));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { ComputerUseSection } from "./ComputerUseSection";

function mount(enabled = false, onChange = vi.fn()) {
  render(<LocaleProvider><ComputerUseSection enabled={enabled} onEnabledChange={onChange} /></LocaleProvider>);
  return onChange;
}

beforeEach(() => {
  status.mockReset();
  audit.mockReset();
  revoke.mockClear();
  openPane.mockClear();
  status.mockResolvedValue({ enabled: false, accessibility: true, screen_recording: false, allowed_apps: ["com.apple.Notes"] });
  audit.mockResolvedValue([{ ts: 1760000000, tool: "computer_click", app: "Notes", detail: "element 0.1", outcome: "denied" }]);
});

describe("ComputerUseSection", () => {
  it("總開關反映 enabled 並把變更交給上層（上層負責存檔）", async () => {
    const onChange = mount(false);
    const box = await screen.findByRole("checkbox", { name: /允許 AI 操作這台電腦/ });
    expect(box).not.toBeChecked();
    await userEvent.click(box);
    expect(onChange).toHaveBeenCalledWith(true);
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
    expect(await screen.findByText(/click · Notes · element 0\.1 · denied/)).toBeInTheDocument();
  });

  it("後端讀取失敗：不崩潰，開關仍可用", async () => {
    const err = vi.spyOn(console, "error").mockImplementation(() => {});
    status.mockRejectedValue("boom");
    mount();
    expect(await screen.findByRole("checkbox", { name: /允許 AI 操作這台電腦/ })).toBeInTheDocument();
    err.mockRestore();
  });
});
