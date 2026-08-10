import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { ProviderForm } from "./ProviderForm";

vi.mock("../../ipc/chatgptWeb", () => ({ chatgptWebModels: vi.fn() }));
vi.mock("../../ipc/provider", async () => {
  const actual = await vi.importActual<typeof import("../../ipc/provider")>(
    "../../ipc/provider",
  );
  return {
    ...actual,
    getGoogleAiModels: vi.fn().mockResolvedValue([]),
    getOpenRouterModels: vi.fn().mockResolvedValue([]),
    getCodexOAuthModels: vi.fn().mockResolvedValue([]),
    getAnthropicOAuthStatus: vi.fn().mockResolvedValue(false),
  };
});

import { chatgptWebModels } from "../../ipc/chatgptWeb";

async function selectChatgptWeb() {
  render(<ProviderForm onSave={async () => {}} onCancel={() => {}} />);
  // 供應商型別的 <label> 沒有 htmlFor，抓不到 label 關聯；它是表單上的第一個
  // 下拉，用角色定位比用文字穩（文字會隨語系變）。
  const select = screen.getAllByRole("combobox")[0];
  await userEvent.selectOptions(select, "chatgpt-web");
}

describe("ProviderForm — ChatGPT Web", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  /**
   * 三項風險刻意都寫在表單上：這條路徑違反上游服務條款、工具呼叫是 prompt
   * 模擬的、認證是逆向而來。使用者要在啟用前就知道，而不是壞掉之後才從一個
   * 403 錯誤訊息裡拼湊。少了任何一項都是資訊揭露不足。
   */
  it("選擇 chatgpt-web 時三項風險都要顯示", async () => {
    await selectChatgptWeb();
    expect(screen.getByText(/服務條款/)).toBeInTheDocument();
    expect(screen.getByText(/模擬/)).toBeInTheDocument();
    expect(screen.getByText(/逆向/)).toBeInTheDocument();
  });

  /**
   * 模型清單是動態取得的（`/backend-api/models` 回該帳號實際可用的），所以
   * 表單上不該有寫死的 slug。這條同時守住「按鈕真的會去叫那個 IPC」。
   */
  it("按下登入會取得模型清單並填進下拉選單", async () => {
    vi.mocked(chatgptWebModels).mockResolvedValue([
      { slug: "gpt-5-5", title: "GPT-5.5", max_tokens: 196000 },
    ]);
    await selectChatgptWeb();

    await userEvent.click(screen.getByRole("button", { name: /登入 ChatGPT/ }));

    await waitFor(() => {
      expect(chatgptWebModels).toHaveBeenCalled();
      expect(screen.getByRole("option", { name: "GPT-5.5" })).toBeInTheDocument();
    });
  });

  /**
   * 尚未登入時後端會回錯誤或空清單。要給出可行動的訊息，不能只是安靜地
   * 什麼都不顯示——那會讓使用者以為按鈕壞了。
   */
  it("拿不到模型時要顯示提示而不是靜默", async () => {
    vi.mocked(chatgptWebModels).mockResolvedValue([]);
    await selectChatgptWeb();

    await userEvent.click(screen.getByRole("button", { name: /登入 ChatGPT/ }));

    await waitFor(() => {
      expect(screen.getByText(/尚未取得模型清單/)).toBeInTheDocument();
    });
  });
});
