import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { GeneralPage } from "./GeneralPage";

// 與 GeneralPage.appimage.test.tsx 相同的掛載方式：get_config 一定要回有效物件，
// 否則掛載時的讀取會丟例外，失敗看起來像出在被測的區塊。
const BASE_CONFIG = {
  execution_mode: "graded",
  submit_shortcut: "enter",
  max_agent_steps: 5,
  default_tab: "terminal",
};

function mockConfig(extra: Record<string, unknown> = {}) {
  const table: Record<string, unknown> = {
    get_config: { ...BASE_CONFIG, ...extra },
    telegram_get_config: { bot_token: null, chat_id: null },
    // GeneralPage 掛載時會讀這個，未 stub 會回 null 而在讀 `.state` 時丟例外。
    appimage_integration_state: { state: "not_appimage" },
  };
  invokeMock.mockImplementation((cmd: string) =>
    Promise.resolve(cmd in table ? table[cmd] : null),
  );
}

beforeEach(() => { invokeMock.mockReset(); });

const radio = (name: RegExp) => screen.getByRole("radio", { name });

describe("GeneralPage — 行內建議的接受鍵", () => {
  it("設定為 right：「→（向右鍵）」被選取", async () => {
    mockConfig({ suggestion_accept_key: "right" });
    render(<GeneralPage />);
    await waitFor(() => expect(radio(/^→/)).toBeChecked());
    expect(radio(/^Tab/)).not.toBeChecked();
  });

  it("舊版設定沒有這個欄位：預設選取 Tab", async () => {
    mockConfig();
    render(<GeneralPage />);
    expect(await screen.findByRole("radio", { name: /^Tab/ })).toBeChecked();
    expect(radio(/^→/)).not.toBeChecked();
    expect(radio(/^關閉建議/)).not.toBeChecked();
  });

  it("點「關閉建議」：呼叫 set_suggestion_accept_key 並選取它", async () => {
    mockConfig();
    render(<GeneralPage />);
    await userEvent.click(await screen.findByRole("radio", { name: /^關閉建議/ }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("set_suggestion_accept_key", { key: "off" }),
    );
    expect(radio(/^關閉建議/)).toBeChecked();
  });

  it("點「→」：呼叫 set_suggestion_accept_key 傳 right", async () => {
    mockConfig();
    render(<GeneralPage />);
    await userEvent.click(await screen.findByRole("radio", { name: /^→/ }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("set_suggestion_accept_key", { key: "right" }),
    );
  });
});
