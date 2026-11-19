import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { ProviderForm } from "./ProviderForm";
import type { ProviderInput } from "../../ipc/provider";

vi.mock("../../ipc/chatgptWeb", () => ({
  chatgptWebModels: vi.fn(),
  chatgptWebLogin: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("../../ipc/provider", async () => {
  const actual = await vi.importActual<typeof import("../../ipc/provider")>("../../ipc/provider");
  return {
    ...actual,
    getGoogleAiModels: vi.fn().mockResolvedValue([]),
    getOpenRouterModels: vi.fn().mockResolvedValue([]),
    getCodexOAuthModels: vi.fn().mockResolvedValue([]),
    getAnthropicOAuthStatus: vi.fn().mockResolvedValue(false),
  };
});

const onSave = vi.fn(async (p: ProviderInput) => { void p; });

async function selectClaudeCli() {
  render(<ProviderForm onSave={onSave} onCancel={() => {}} />);
  await userEvent.selectOptions(screen.getAllByRole("combobox")[0], "claude-cli");
}

describe("ProviderForm — Claude Code CLI", () => {
  beforeEach(() => vi.clearAllMocks());

  it("只有模型欄位：沒有金鑰、沒有 Base URL 區塊", async () => {
    await selectClaudeCli();
    expect(document.getElementById("claude-cli-model")).toBeInTheDocument();
    expect(document.querySelector('input[type="password"]')).toBeNull();
    expect(screen.queryByText(/Base URL/i)).toBeNull();
  });

  it("預設模型是 sonnet，存檔時不帶金鑰與 base_url、型別正確", async () => {
    await selectClaudeCli();
    const inputs = screen.getAllByRole("textbox");
    // 前兩個文字框是 ID 與顯示名稱。
    await userEvent.type(inputs[0], "cli1");
    await userEvent.type(inputs[1], "Claude CLI");
    await userEvent.click(screen.getByRole("button", { name: /儲存|新增|Save|Add/ }));
    expect(onSave).toHaveBeenCalledTimes(1);
    const saved = onSave.mock.calls[0][0];
    expect(saved).toMatchObject({
      id: "cli1",
      provider_type: "claude-cli",
      model: "sonnet",
      api_key: null,
      base_url: null,
      auth_method: null,
    });
  });

  it("模型可以清空（用 CLI 預設），不會被「模型不能空白」擋下", async () => {
    await selectClaudeCli();
    const inputs = screen.getAllByRole("textbox");
    await userEvent.type(inputs[0], "cli2");
    await userEvent.type(inputs[1], "Claude CLI");
    await userEvent.clear(document.getElementById("claude-cli-model") as HTMLInputElement);
    await userEvent.click(screen.getByRole("button", { name: /儲存|新增|Save|Add/ }));
    expect(onSave).toHaveBeenCalledTimes(1);
    expect(onSave.mock.calls[0][0].model).toBe("");
  });
});
