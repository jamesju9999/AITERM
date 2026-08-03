import { render, screen } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { NotebookCreateDialog } from "./NotebookCreateDialog";
import { LocaleProvider } from "../../contexts/LocaleContext";

vi.mock("../../ipc/vcs", () => ({ pickFolder: vi.fn() }));
vi.mock("../../ipc/provider", () => ({ listProviders: vi.fn() }));
vi.mock("../../ipc/knowledgeBase", () => ({ kbListEmbeddingModels: vi.fn() }));

import { listProviders } from "../../ipc/provider";
import { kbListEmbeddingModels } from "../../ipc/knowledgeBase";

const PROVIDERS = [
  {
    id: "lmstudio", display_name: "Qwen3.6-35B-A3B-4bit",
    provider_type: "openai-compatible", base_url: "http://localhost:1234/v1",
    oauth_client_id: null, model: "Qwen3.6-35B-A3B-4bit",
    supports_json_mode: false, has_api_key: false, is_default: false, auth_method: null,
  },
  {
    id: "ollama-local", display_name: "本機 Ollama",
    provider_type: "ollama", base_url: null,
    oauth_client_id: null, model: "llama3.1:8b",
    supports_json_mode: false, has_api_key: false, is_default: false, auth_method: null,
  },
];

function renderDialog() {
  return render(
    <LocaleProvider>
      <NotebookCreateDialog onCreate={vi.fn()} onClose={vi.fn()} />
    </LocaleProvider>
  );
}

describe("NotebookCreateDialog provider labelling", () => {
  beforeEach(() => {
    vi.mocked(listProviders).mockResolvedValue(PROVIDERS as never);
    vi.mocked(kbListEmbeddingModels).mockResolvedValue([]);
  });

  it("shows the provider type and endpoint so the name is not read as a model", async () => {
    renderDialog();
    const option = await screen.findByRole("option", { name: /Qwen3\.6-35B-A3B-4bit/ });
    expect(option.textContent).toContain("OpenAI-Compatible");
    expect(option.textContent).toContain("localhost:1234");
  });

  it("falls back to the default endpoint when base_url is null", async () => {
    renderDialog();
    const option = await screen.findByRole("option", { name: /本機 Ollama/ });
    expect(option.textContent).toContain("localhost:11434");
  });
});
