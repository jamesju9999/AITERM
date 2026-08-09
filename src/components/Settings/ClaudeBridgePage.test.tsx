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

import { bridgeStatus, bridgeSetConfig } from "../../ipc/bridge";
import type { ClaudeBridgeConfig } from "../../ipc/bridge";
import { getConfig } from "../../ipc/config";
import type { AppConfig } from "../../ipc/config";
import { listProviders } from "../../ipc/provider";
import type { ProviderInfo } from "../../ipc/provider";

// getConfig() 的 claude_bridge 以外欄位在這個元件裡完全不會被讀取，但
// AppConfig 是既有的完整型別，用完整假資料湊齊比亂寫斷言型別安全。
const BASE_CONFIG: AppConfig = {
  default_provider: null,
  providers: [],
  execution_mode: "graded",
  submit_shortcut: "enter",
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
});
