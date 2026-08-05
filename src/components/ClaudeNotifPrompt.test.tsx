import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { ClaudeNotifPrompt } from "./ClaudeNotifPrompt";

const DEFAULTS: Record<string, unknown> = {
  claude_notif_needs_prompt: true,
  is_claude_notif_declined: false,
  is_onboarding_done: true,
};

function mockCommands(overrides: Record<string, unknown> = {}) {
  const table = { ...DEFAULTS, ...overrides };
  invokeMock.mockImplementation((cmd: string) =>
    Promise.resolve(cmd in table ? table[cmd] : null),
  );
}

const TITLE = "讓 Claude Code 在背景分頁提醒你？";

beforeEach(() => { invokeMock.mockReset(); });

describe("ClaudeNotifPrompt", () => {
  it("在偵測到 claude 且設定缺失時提示", async () => {
    mockCommands();
    render(<ClaudeNotifPrompt claudeSeen blocked={false} />);

    expect(await screen.findByText(TITLE)).toBeInTheDocument();
  });

  it("還沒偵測到 claude 就不提示", async () => {
    mockCommands();
    render(<ClaudeNotifPrompt claudeSeen={false} blocked={false} />);

    // 連查都不該查——沒偵測到就沒有理由碰使用者的設定檔。
    await waitFor(() => expect(invokeMock).not.toHaveBeenCalled());
    expect(screen.queryByText(TITLE)).not.toBeInTheDocument();
  });

  it("設定已經有值時不提示", async () => {
    mockCommands({ claude_notif_needs_prompt: false });
    render(<ClaudeNotifPrompt claudeSeen blocked={false} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByText(TITLE)).not.toBeInTheDocument();
  });

  it("婉拒過就不再問", async () => {
    mockCommands({ is_claude_notif_declined: true });
    render(<ClaudeNotifPrompt claudeSeen blocked={false} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByText(TITLE)).not.toBeInTheDocument();
  });

  it("onboarding 還沒完成就不提示", async () => {
    mockCommands({ is_onboarding_done: false });
    render(<ClaudeNotifPrompt claudeSeen blocked={false} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByText(TITLE)).not.toBeInTheDocument();
  });

  it("讓位給更優先的角落卡片", async () => {
    // 三張卡片都固定在右下角同一個位置，同時出現會完全重疊。
    mockCommands();
    render(<ClaudeNotifPrompt claudeSeen blocked />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByText(TITLE)).not.toBeInTheDocument();
  });

  it("接受後寫入設定，並告訴使用者要開新分頁", async () => {
    mockCommands();
    render(<ClaudeNotifPrompt claudeSeen blocked={false} />);

    await userEvent.click(await screen.findByRole("button", { name: "幫我設定" }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("claude_notif_enable_bell"));
    // 這句是使用者最容易誤判成「設了也沒用」的地方，必須真的出現。
    expect(await screen.findByText(/請開一個新的終端機分頁/)).toBeInTheDocument();
  });

  it("寫入失敗時顯示錯誤，不假裝成功", async () => {
    mockCommands();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "claude_notif_enable_bell") return Promise.reject("權限不足");
      return Promise.resolve(cmd in DEFAULTS ? DEFAULTS[cmd] : null);
    });
    render(<ClaudeNotifPrompt claudeSeen blocked={false} />);

    await userEvent.click(await screen.findByRole("button", { name: "幫我設定" }));

    expect(await screen.findByText(/權限不足/)).toBeInTheDocument();
    expect(screen.queryByText(/請開一個新的終端機分頁/)).not.toBeInTheDocument();
  });

  it("婉拒會被記錄下來", async () => {
    mockCommands();
    render(<ClaudeNotifPrompt claudeSeen blocked={false} />);

    await userEvent.click(await screen.findByRole("button", { name: "不用了" }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("set_claude_notif_declined"));
    expect(screen.queryByText(TITLE)).not.toBeInTheDocument();
  });
});
