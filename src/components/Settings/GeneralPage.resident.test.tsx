import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { GeneralPage } from "./GeneralPage";

const BASE_CONFIG = {
  execution_mode: "graded",
  submit_shortcut: "enter",
  max_agent_steps: 5,
  default_tab: "terminal",
};

function mockConfig(extra: Record<string, unknown> = {}, overrides: Record<string, unknown> = {}) {
  const table: Record<string, unknown> = {
    get_config: { ...BASE_CONFIG, ...extra },
    telegram_get_config: { bot_token: null, chat_id: null },
    appimage_integration_state: { state: "not_appimage" },
    ...overrides,
  };
  invokeMock.mockImplementation((cmd: string) => {
    const v = table[cmd];
    if (v instanceof Error) return Promise.reject(v.message);
    return Promise.resolve(cmd in table ? v : null);
  });
}

beforeEach(() => { invokeMock.mockReset(); });

describe("GeneralPage — 常駐與快捷鍵", () => {
  it("顯示設定檔裡的快捷鍵與開關；舊設定缺欄位時用預設（Ctrl+`、縮到托盤、不自啟）", async () => {
    mockConfig();
    render(<GeneralPage />);
    expect(await screen.findByLabelText("全域快捷鍵")).toHaveValue("Ctrl+`");
    expect(screen.getByRole("checkbox", { name: /關閉視窗時縮到系統匣/ })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: /開機時自動啟動/ })).not.toBeChecked();
  });

  it("套用新快捷鍵：呼叫 set_global_hotkey", async () => {
    mockConfig();
    render(<GeneralPage />);
    const box = await screen.findByLabelText("全域快捷鍵");
    await userEvent.clear(box);
    await userEvent.type(box, "Alt+Shift+K");
    await userEvent.click(screen.getByRole("button", { name: "套用" }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("set_global_hotkey", { hotkey: "Alt+Shift+K" }),
    );
  });

  it("後端拒絕（被占用／無效）：顯示錯誤，輸入框還原成已生效的舊值", async () => {
    mockConfig({}, { set_global_hotkey: new Error("無法註冊快捷鍵") });
    render(<GeneralPage />);
    const box = await screen.findByLabelText("全域快捷鍵");
    await userEvent.clear(box);
    await userEvent.type(box, "Alt+Z");
    await userEvent.click(screen.getByRole("button", { name: "套用" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("無法註冊快捷鍵");
    expect(box).toHaveValue("Ctrl+`");
  });

  it("切換「縮到系統匣」與「開機自啟」：各自呼叫對應指令", async () => {
    mockConfig();
    render(<GeneralPage />);
    await userEvent.click(await screen.findByRole("checkbox", { name: /關閉視窗時縮到系統匣/ }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("set_keep_in_tray", { enabled: false }),
    );
    await userEvent.click(screen.getByRole("checkbox", { name: /開機時自動啟動/ }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("set_autostart", { enabled: true }),
    );
  });
});
