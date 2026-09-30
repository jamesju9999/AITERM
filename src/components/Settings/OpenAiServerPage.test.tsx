import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { OpenAiServerPage } from "./OpenAiServerPage";

vi.mock("../../ipc/openaiServer", () => ({
  openaiServerStatus: vi.fn(),
  openaiServerSetConfig: vi.fn(),
  openaiServerRegenerateKey: vi.fn(),
}));
vi.mock("../../ipc/config", () => ({ getConfig: vi.fn() }));

import {
  openaiServerStatus,
  openaiServerSetConfig,
  openaiServerRegenerateKey,
} from "../../ipc/openaiServer";
import type { OpenAiServerStatus } from "../../ipc/openaiServer";
import { getConfig } from "../../ipc/config";
import type { AppConfig } from "../../ipc/config";

const PROVIDERS = [
  { id: "gem", display_name: "Gemini", provider_type: "google-ai", base_url: null, oauth_client_id: null, model: "gemini-2.5-pro", supports_json_mode: true },
  { id: "loc", display_name: "Local", provider_type: "ollama", base_url: null, oauth_client_id: null, model: "qwen3", supports_json_mode: true },
] as unknown as AppConfig["providers"];

function config(over: Partial<AppConfig["openai_server"]> = {}): AppConfig {
  return {
    default_provider: null,
    providers: PROVIDERS,
    execution_mode: "graded",
    submit_shortcut: "enter",
    doc_convert_engine: "auto",
    onboarding_done: true,
    max_agent_steps: 5,
    default_tab: "terminal",
    enterprise_server_url: null,
    enterprise_device_id: null,
    enterprise_policy: null,
    claude_bridge: { enabled: false, port: 8317, default_on_new_tab: false, opus: null, sonnet: null, haiku: null },
    mcp_tool_server: { enabled: false, port: 8318, coordination_enabled: false },
    openai_server: { enabled: false, port: 8319, allow_lan: false, aliases: [], ...over },
  };
}

const STOPPED: OpenAiServerStatus = { running: false, port: null, lan: false, token: null, error: null };
const RUNNING: OpenAiServerStatus = { running: true, port: 8319, lan: false, token: "sk-secret-123", error: null };

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(getConfig).mockResolvedValue(config());
  vi.mocked(openaiServerStatus).mockResolvedValue(STOPPED);
  vi.mocked(openaiServerSetConfig).mockResolvedValue(RUNNING);
  vi.mocked(openaiServerRegenerateKey).mockResolvedValue({ ...RUNNING, token: "sk-new-456" });
});

async function loaded() {
  const user = userEvent.setup();
  render(<OpenAiServerPage />);
  await waitFor(() => screen.getAllByRole("checkbox"));
  return user;
}

describe("OpenAiServerPage", () => {
  it("loads the saved config and shows the stopped status", async () => {
    await loaded();
    expect(screen.getAllByRole("checkbox")[0]).not.toBeChecked();
    expect(screen.getByDisplayValue("8319")).toBeInTheDocument();
  });

  it("enabling and saving sends enabled:true to the backend", async () => {
    const user = await loaded();
    await user.click(screen.getAllByRole("checkbox")[0]);
    await user.click(screen.getByRole("button", { name: /save|儲存/i }));
    await waitFor(() =>
      expect(openaiServerSetConfig).toHaveBeenCalledWith(expect.objectContaining({ enabled: true, port: 8319 })),
    );
  });

  it("shows the LAN risk warning only while allow-LAN is checked", async () => {
    const user = await loaded();
    expect(screen.queryByRole("note")).toBeNull();
    await user.click(screen.getAllByRole("checkbox")[1]);
    expect(screen.getByRole("note")).toBeInTheDocument();
    await user.click(screen.getAllByRole("checkbox")[1]);
    expect(screen.queryByRole("note")).toBeNull();
  });

  it("adds an alias row defaulting to the first provider and its model", async () => {
    const user = await loaded();
    await user.click(screen.getByRole("button", { name: /add alias|新增別名/i }));
    expect(screen.getByDisplayValue("gemini-2.5-pro")).toBeInTheDocument();
    await user.type(screen.getByRole("textbox", { name: /public name|對外名稱/i }), "gpt-4o");
    await user.click(screen.getByRole("button", { name: /save|儲存/i }));
    await waitFor(() =>
      expect(openaiServerSetConfig).toHaveBeenCalledWith(
        expect.objectContaining({
          aliases: [{ alias: "gpt-4o", provider_id: "gem", model: "gemini-2.5-pro" }],
        }),
      ),
    );
  });

  it("changing an alias's provider resets its model to that provider's default", async () => {
    vi.mocked(getConfig).mockResolvedValue(
      config({ aliases: [{ alias: "a", provider_id: "gem", model: "gemini-2.5-pro" }] }),
    );
    const user = await loaded();
    await user.selectOptions(screen.getByRole("combobox"), "loc");
    expect(screen.getByDisplayValue("qwen3")).toBeInTheDocument();
  });

  it("disables save while two aliases share a name", async () => {
    vi.mocked(getConfig).mockResolvedValue(
      config({
        aliases: [
          { alias: "same", provider_id: "gem", model: "m" },
          { alias: "same", provider_id: "loc", model: "n" },
        ],
      }),
    );
    await loaded();
    expect(screen.getByRole("button", { name: /save|儲存/i })).toBeDisabled();
    expect(screen.getAllByText(/duplicate alias|別名重複/i).length).toBe(2);
  });

  it("masks the API key until shown", async () => {
    vi.mocked(openaiServerStatus).mockResolvedValue(RUNNING);
    const user = await loaded();
    await waitFor(() => expect(screen.queryByText("sk-secret-123")).toBeNull());
    expect(screen.getByText("•".repeat(24))).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /^(show|顯示)$/i }));
    expect(screen.getByText("sk-secret-123")).toBeInTheDocument();
  });

  it("regenerating the key needs a second click and then shows the new key", async () => {
    vi.mocked(openaiServerStatus).mockResolvedValue(RUNNING);
    const user = await loaded();
    await waitFor(() => screen.getByRole("button", { name: /^(regenerate|重新產生)$/i }));
    await user.click(screen.getByRole("button", { name: /^(regenerate|重新產生)$/i }));
    expect(openaiServerRegenerateKey).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: /sure|確定/i }));
    await waitFor(() => expect(openaiServerRegenerateKey).toHaveBeenCalledTimes(1));
    await user.click(screen.getByRole("button", { name: /^(show|顯示)$/i }));
    expect(screen.getByText("sk-new-456")).toBeInTheDocument();
  });

  it("shows a backend validation rejection and does not claim it saved", async () => {
    vi.mocked(openaiServerSetConfig).mockRejectedValue("別名「x」重複了。");
    const user = await loaded();
    await user.click(screen.getByRole("button", { name: /save|儲存/i }));
    expect(await screen.findByRole("alert")).toHaveTextContent("別名「x」重複了。");
    expect(screen.queryByText(/✓/)).toBeNull();
  });

  it("shows a start-up error such as a busy port", async () => {
    vi.mocked(openaiServerStatus).mockResolvedValue({ ...STOPPED, error: "無法綁定 127.0.0.1:8319" });
    render(<OpenAiServerPage />);
    expect(await screen.findByText(/無法綁定/)).toBeInTheDocument();
  });
});
