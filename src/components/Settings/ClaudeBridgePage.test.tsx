import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { ClaudeBridgePage } from "./ClaudeBridgePage";

vi.mock("../../ipc/bridge", () => ({
  bridgeStatus: vi.fn(),
  bridgeSetConfig: vi.fn(),
}));
vi.mock("../../ipc/config", () => ({ getConfig: vi.fn() }));
vi.mock("../../ipc/provider", () => ({ listProviders: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn() }));

import { bridgeStatus, bridgeSetConfig } from "../../ipc/bridge";
import { confirm } from "@tauri-apps/plugin-dialog";
import type { ClaudeBridgeConfig } from "../../ipc/bridge";
import { getConfig } from "../../ipc/config";
import type { AppConfig } from "../../ipc/config";
import { listProviders } from "../../ipc/provider";
import type { ProviderInfo } from "../../ipc/provider";
import { translations } from "../../lib/i18n";

const t = translations["zh-TW"];

// getConfig() 的 claude_bridge 以外欄位在這個元件裡完全不會被讀取，但
// AppConfig 是既有的完整型別，用完整假資料湊齊比亂寫斷言型別安全。
const BASE_CONFIG: AppConfig = {
  default_provider: null,
  providers: [],
  execution_mode: "graded",
  submit_shortcut: "enter",
  doc_convert_engine: "auto",
  onboarding_done: true,
  max_agent_steps: 5,
  default_tab: "terminal",
  enterprise_server_url: null,
  enterprise_device_id: null,
  enterprise_policy: null,
  claude_bridge: {
    enabled: false,
    port: 8317,
    default_on_new_tab: false,
    opus: null,
    sonnet: null,
    haiku: null,
  },
  mcp_tool_server: { enabled: false, port: 8318, coordination_enabled: false },
};

// provider 列表改走 listProviders()（ipc/provider.ts 的 ProviderInfo），
// 跟既有的 ProvidersPage.tsx 用同一個來源，不用 getConfig().providers ——
// 後者的 AppConfig.providers 欄位在前端目前完全沒人讀，型別跟後端實際
// JSON（provider_type 欄位序列化後其實是 "type"）已經對不上。
const PROVIDERS: ProviderInfo[] = [
  {
    id: "qwen",
    display_name: "本地 Qwen",
    provider_type: "openai-compatible",
    base_url: "http://localhost:8000/v1",
    oauth_client_id: null,
    model: "Qwen3.6-35B",
    supports_json_mode: true,
    has_api_key: false,
    is_default: false,
    auth_method: null,
  },
  {
    id: "cdx",
    display_name: "Codex",
    provider_type: "codex",
    base_url: null,
    oauth_client_id: null,
    model: "gpt-5",
    supports_json_mode: true,
    has_api_key: true,
    is_default: false,
    auth_method: null,
  },
  {
    id: "gemini",
    display_name: "Gemini",
    provider_type: "google-ai",
    base_url: null,
    oauth_client_id: null,
    model: "gemini-3-pro",
    supports_json_mode: true,
    has_api_key: false,
    is_default: false,
    auth_method: "oauth",
  },
];

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(getConfig).mockResolvedValue(BASE_CONFIG);
  vi.mocked(listProviders).mockResolvedValue(PROVIDERS);
  vi.mocked(bridgeStatus).mockResolvedValue({
    running: false,
    port: null,
    token: null,
    error: null,
  });
  vi.mocked(bridgeSetConfig).mockImplementation((value: ClaudeBridgeConfig) =>
    Promise.resolve({ running: true, port: value.port, token: "tok", error: null }),
  );
  vi.mocked(confirm).mockResolvedValue(true);
});

describe("ClaudeBridgePage", () => {
  it("顯示停止中的狀態", async () => {
    render(<ClaudeBridgePage />);
    expect(await screen.findByText(/未啟動|Stopped/)).toBeInTheDocument();
  });

  it("Codex 選項可選", async () => {
    render(<ClaudeBridgePage />);
    const select = await screen.findByLabelText(/Opus/);
    const codex = Array.from(select.querySelectorAll("option")).find((o) =>
      o.textContent?.includes("Codex"),
    );
    expect(codex).toBeDefined();
    expect(codex).not.toBeDisabled();
  });

  it("Antigravity（google-ai + oauth）選項可選", async () => {
    // M3 之後 google-ai 的 oauth 模式走 Antigravity，已無不支援的 provider
    // type——這裡改成驗證它可選，取代 M2 時「不支援」的舊斷言。
    render(<ClaudeBridgePage />);
    const select = await screen.findByLabelText(/Opus/);
    const gemini = Array.from(select.querySelectorAll("option")).find((o) =>
      o.textContent?.includes("Gemini"),
    );
    expect(gemini).toBeDefined();
    expect(gemini).not.toBeDisabled();
  });

  it("選了供應商就帶入它的預設模型", async () => {
    const user = userEvent.setup();
    render(<ClaudeBridgePage />);
    const select = await screen.findByLabelText(/Sonnet/);
    await user.selectOptions(select, "qwen");
    await waitFor(() => {
      expect(screen.getByDisplayValue("Qwen3.6-35B")).toBeInTheDocument();
    });
  });

  it("存檔時把設定送給後端", async () => {
    const user = userEvent.setup();
    render(<ClaudeBridgePage />);
    // 等資料載入完成，按鈕才會出現且可點——直接點會點在還沒 render 出來的按鈕上。
    const save = await screen.findByRole("button", { name: /儲存|Save/ });
    await waitFor(() => expect(save).toBeEnabled());
    await user.click(save);
    await waitFor(() => expect(bridgeSetConfig).toHaveBeenCalledTimes(1));
  });

  it("存檔後按鈕顯示已儲存，再改動設定就撤回", async () => {
    const user = userEvent.setup();
    render(<ClaudeBridgePage />);
    const save = await screen.findByRole("button", { name: /儲存|Save/ });
    await waitFor(() => expect(save).toBeEnabled());
    await user.click(save);
    await screen.findByRole("button", { name: /已儲存|Saved/ });

    // 一改動就不該再宣稱存過了——否則按鈕會對著已經不同的內容說「已儲存」。
    await user.selectOptions(await screen.findByLabelText(/Haiku/), "qwen");
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: /已儲存|Saved/ })).not.toBeInTheDocument(),
    );
  });

  it("啟動失敗時顯示錯誤而不是拋例外", async () => {
    vi.mocked(bridgeStatus).mockResolvedValue({
      running: false,
      port: null,
      token: null,
      error: "無法綁定 127.0.0.1:8317",
    });
    render(<ClaudeBridgePage />);
    expect(await screen.findByText(/無法綁定/)).toBeInTheDocument();
  });

  describe("帳號組合", () => {
    beforeEach(() => {
      localStorage.clear();
    });

    it("沒有任何組合時顯示空狀態提示", async () => {
      render(<ClaudeBridgePage />);
      expect(await screen.findByText(/還沒有存過任何組合|No profiles saved yet/)).toBeInTheDocument();
    });

    it("另存目前設定為新組合後出現在清單裡，且標示使用中", async () => {
      const user = userEvent.setup();
      render(<ClaudeBridgePage />);

      // 用 role: combobox 而不是 findByLabelText(/Opus/)：一旦這個 tier 有值，
      // 它的模型輸入框 aria-label 也會含「Opus」（見 tierModel aria-label 組法），
      // 兩個元素都會命中純文字比對造成「找到多個元素」。role 篩到 <select> 才不會撞。
      await user.selectOptions(await screen.findByRole("combobox", { name: /Opus/ }), "qwen");

      const nameInput = await screen.findByPlaceholderText(/組合名稱|Profile name/);
      await user.type(nameInput, "個人帳號");
      await user.click(
        screen.getByRole("button", { name: /另存目前設定為新組合|Save current as new profile/ }),
      );

      expect(await screen.findByText("個人帳號")).toBeInTheDocument();
      expect(screen.getByText(/使用中|Active/)).toBeInTheDocument();
    });

    it("改動表格之後，原本標示使用中的組合就不再標示", async () => {
      const user = userEvent.setup();
      render(<ClaudeBridgePage />);

      await user.selectOptions(await screen.findByRole("combobox", { name: /Opus/ }), "qwen");
      await user.type(await screen.findByPlaceholderText(/組合名稱|Profile name/), "個人帳號");
      await user.click(
        screen.getByRole("button", { name: /另存目前設定為新組合|Save current as new profile/ }),
      );
      await screen.findByText(/使用中|Active/);

      await user.selectOptions(screen.getByRole("combobox", { name: /Opus/ }), "cdx");

      await waitFor(() => {
        expect(screen.queryByText(/使用中|Active/)).not.toBeInTheDocument();
      });
    });

    it("套用組合時立即呼叫 bridgeSetConfig，帶入該組合的三個 tier", async () => {
      const user = userEvent.setup();
      render(<ClaudeBridgePage />);

      await user.selectOptions(await screen.findByRole("combobox", { name: /Opus/ }), "qwen");
      await user.type(await screen.findByPlaceholderText(/組合名稱|Profile name/), "個人帳號");
      await user.click(
        screen.getByRole("button", { name: /另存目前設定為新組合|Save current as new profile/ }),
      );

      await user.selectOptions(screen.getByRole("combobox", { name: /Opus/ }), "cdx");
      vi.mocked(bridgeSetConfig).mockClear();

      await user.click(screen.getByRole("button", { name: t.bridge_profile_apply }));

      await waitFor(() => expect(bridgeSetConfig).toHaveBeenCalledTimes(1));
      const payload = vi.mocked(bridgeSetConfig).mock.calls[0][0];
      expect(payload.opus?.provider_id).toBe("qwen");
    });

    it("套用組合後畫面上的 tier 表格也跟著換", async () => {
      const user = userEvent.setup();
      render(<ClaudeBridgePage />);

      await user.selectOptions(await screen.findByRole("combobox", { name: /Opus/ }), "qwen");
      await user.type(await screen.findByPlaceholderText(/組合名稱|Profile name/), "個人帳號");
      await user.click(
        screen.getByRole("button", { name: /另存目前設定為新組合|Save current as new profile/ }),
      );

      await user.selectOptions(screen.getByRole("combobox", { name: /Opus/ }), "cdx");
      await user.click(screen.getByRole("button", { name: t.bridge_profile_apply }));

      await waitFor(() => {
        expect(screen.getByRole("combobox", { name: /Opus/ })).toHaveValue("qwen");
      });
    });

    it("更新組合只覆蓋 localStorage，不呼叫 bridgeSetConfig", async () => {
      const user = userEvent.setup();
      render(<ClaudeBridgePage />);

      await user.selectOptions(await screen.findByRole("combobox", { name: /Opus/ }), "qwen");
      await user.type(await screen.findByPlaceholderText(/組合名稱|Profile name/), "個人帳號");
      await user.click(
        screen.getByRole("button", { name: /另存目前設定為新組合|Save current as new profile/ }),
      );

      await user.selectOptions(screen.getByRole("combobox", { name: /Opus/ }), "cdx");
      vi.mocked(bridgeSetConfig).mockClear();

      await user.click(screen.getByRole("button", { name: t.bridge_profile_update }));

      expect(bridgeSetConfig).not.toHaveBeenCalled();
      const stored = JSON.parse(localStorage.getItem("aiterm.bridgeProfiles") ?? "[]");
      expect(stored[0].opus.provider_id).toBe("cdx");
    });

    it("重新命名組合", async () => {
      const user = userEvent.setup();
      render(<ClaudeBridgePage />);

      await user.selectOptions(await screen.findByRole("combobox", { name: /Opus/ }), "qwen");
      await user.type(await screen.findByPlaceholderText(/組合名稱|Profile name/), "個人帳號");
      await user.click(
        screen.getByRole("button", { name: /另存目前設定為新組合|Save current as new profile/ }),
      );

      await user.click(screen.getByRole("button", { name: t.bridge_profile_rename }));
      const renameInput = await screen.findByPlaceholderText(/新名稱|New name/);
      await user.clear(renameInput);
      await user.type(renameInput, "公司帳號{Enter}");

      expect(await screen.findByText("公司帳號")).toBeInTheDocument();
      expect(screen.queryByText("個人帳號")).not.toBeInTheDocument();
    });

    it("刪除組合前會跳確認框，確認後才真的刪除", async () => {
      const user = userEvent.setup();
      render(<ClaudeBridgePage />);

      await user.selectOptions(await screen.findByRole("combobox", { name: /Opus/ }), "qwen");
      await user.type(await screen.findByPlaceholderText(/組合名稱|Profile name/), "個人帳號");
      await user.click(
        screen.getByRole("button", { name: /另存目前設定為新組合|Save current as new profile/ }),
      );

      await user.click(screen.getByRole("button", { name: t.bridge_profile_delete }));

      await waitFor(() =>
        expect(confirm).toHaveBeenCalledWith(t.bridge_profile_delete_confirm("個人帳號"), expect.anything()),
      );
      await waitFor(() => expect(screen.queryByText("個人帳號")).not.toBeInTheDocument());
    });

    it("取消刪除確認框時保留組合", async () => {
      vi.mocked(confirm).mockResolvedValue(false);
      const user = userEvent.setup();
      render(<ClaudeBridgePage />);

      await user.selectOptions(await screen.findByRole("combobox", { name: /Opus/ }), "qwen");
      await user.type(await screen.findByPlaceholderText(/組合名稱|Profile name/), "個人帳號");
      await user.click(
        screen.getByRole("button", { name: /另存目前設定為新組合|Save current as new profile/ }),
      );

      await user.click(screen.getByRole("button", { name: t.bridge_profile_delete }));

      await waitFor(() => expect(confirm).toHaveBeenCalled());
      expect(screen.getByText("個人帳號")).toBeInTheDocument();
    });
  });
});
