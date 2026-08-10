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
   * 表單上只能有**一個**模型欄位。
   *
   * 通用的「端點與模型 → Model」輸入框與這裡的下拉綁同一個 state，兩個都顯示
   * 時使用者看到的是兩個「模型」欄位、值還長得不一樣（一個顯示 title、一個
   * 顯示 slug），不知道該設哪一個。實測時就是這樣被發現的。
   *
   * API Key 同理：這個供應商沒有金鑰可填，登入狀態在 webview 自己身上。而那
   * 個欄位的顯示條件是「排除清單」——新增的供應商型別預設會被包含進來，所以
   * 這條測試守的是「有人動那份清單時會有人叫」。
   */
  it("不顯示重複的模型欄位與用不到的 API Key", async () => {
    vi.mocked(chatgptWebModels).mockResolvedValue([
      { slug: "gpt-5-6", title: "GPT-5.6 Luna", max_tokens: 34834 },
    ]);
    await selectChatgptWeb();
    await userEvent.click(screen.getByRole("button", { name: /登入 ChatGPT/ }));
    await waitFor(() => expect(chatgptWebModels).toHaveBeenCalled());

    // 通用 Model 欄位的 placeholder 是 DEFAULT_MODELS[providerType]，對
    // chatgpt-web 是空字串——所以改用「有幾個文字輸入框」來判斷。表單此時
    // 應該只有 ID 與顯示名稱兩個 textbox；模型下拉是 combobox 不算在內。
    expect(
      screen.getAllByRole("textbox"),
      "多出來的文字框就是通用 Model 欄位或 API Key 又冒出來了",
    ).toHaveLength(2);
    // API Key 的 <label> 沒有 htmlFor，抓不到 label 關聯，只能用 placeholder。
    expect(screen.queryByPlaceholderText(/貼上你的 API Key|Paste your API key/i))
      .not.toBeInTheDocument();
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
