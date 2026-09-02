import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { NotebookCreateDialog } from "./NotebookCreateDialog";
import { LocaleProvider } from "../../contexts/LocaleContext";

vi.mock("../../ipc/vcs", () => ({ pickFolder: vi.fn() }));
vi.mock("../../ipc/provider", () => ({ listProviders: vi.fn() }));
vi.mock("../../ipc/knowledgeBase", () => ({ kbListEmbeddingModels: vi.fn() }));

import { pickFolder } from "../../ipc/vcs";
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

describe("NotebookCreateDialog model list", () => {
  beforeEach(() => {
    vi.mocked(listProviders).mockResolvedValue(PROVIDERS as never);
  });

  it("sorts likely embedding models ahead of chat models", async () => {
    vi.mocked(kbListEmbeddingModels).mockResolvedValue([
      "Qwen3.6-35B-A3B-4bit",
      "llama3.1:8b",
      "nomic-embed-text",
      "bge-m3",
    ]);

    const { container } = renderDialog();

    await waitFor(() => {
      expect(container.querySelectorAll("datalist option").length).toBe(4);
    });

    const values = Array.from(
      container.querySelectorAll<HTMLOptionElement>("datalist option")
    ).map((o) => o.value);

    expect(values.slice(0, 2)).toEqual(["nomic-embed-text", "bge-m3"]);
    expect(values.slice(2)).toEqual(["Qwen3.6-35B-A3B-4bit", "llama3.1:8b"]);
  });

  it("still allows typing a name when listing fails", async () => {
    vi.mocked(kbListEmbeddingModels).mockRejectedValue(new Error("404"));

    const { container } = renderDialog();

    // The model field renders a disabled "loading…" input while the list is in
    // flight; only once the (rejected) fetch settles does it fall back to the
    // editable input. Poll for that input rather than assuming the rejection's
    // .catch/.finally microtasks have flushed by the next line — on a loaded CI
    // runner they sometimes have not (this test flaked there).
    const input = await screen.findByPlaceholderText("例如：nomic-embed-text");
    expect(input).not.toBeDisabled();
    expect(container.querySelectorAll("datalist option").length).toBe(0);
    expect(vi.mocked(kbListEmbeddingModels)).toHaveBeenCalled();
  });

  it("reloads the list when the provider changes", async () => {
    vi.mocked(kbListEmbeddingModels).mockResolvedValue(["nomic-embed-text"]);

    const { container } = renderDialog();

    await waitFor(() => {
      expect(vi.mocked(kbListEmbeddingModels)).toHaveBeenCalledWith("lmstudio");
    });

    // 不能用 getByRole("combobox")：<select> 與帶 list 屬性的 <input>
    // 兩者的 role 都是 combobox，清單載入後會匹配到兩個元素而拋錯。
    const select = container.querySelector("select")!;
    fireEvent.change(select, { target: { value: "ollama-local" } });

    await waitFor(() => {
      expect(vi.mocked(kbListEmbeddingModels)).toHaveBeenCalledWith("ollama-local");
    });
  });

  it("warns when the typed model does not look like an embedding model", async () => {
    vi.mocked(kbListEmbeddingModels).mockResolvedValue(["nomic-embed-text", "Qwen3.6-35B-A3B-4bit"]);

    renderDialog();

    const input = await screen.findByPlaceholderText("例如：nomic-embed-text");
    fireEvent.change(input, { target: { value: "Qwen3.6-35B-A3B-4bit" } });

    expect(await screen.findByText(/看起來不像 embedding 模型/)).toBeTruthy();
  });

  it("shows no warning for a plausible embedding model, or for an empty field", async () => {
    vi.mocked(kbListEmbeddingModels).mockResolvedValue(["nomic-embed-text"]);

    renderDialog();

    const input = await screen.findByPlaceholderText("例如：nomic-embed-text");
    expect(screen.queryByText(/看起來不像 embedding 模型/)).toBeNull();

    fireEvent.change(input, { target: { value: "nomic-embed-text" } });
    expect(screen.queryByText(/看起來不像 embedding 模型/)).toBeNull();
  });

  it("keeps creation enabled while the warning is showing", async () => {
    vi.mocked(kbListEmbeddingModels).mockResolvedValue(["nomic-embed-text"]);
    vi.mocked(pickFolder).mockResolvedValue("/tmp/docs");

    renderDialog();

    // 每個必填欄位都要真的填上，否則按鈕本來就是 disabled，
    // 「警告不擋建立」這句話就沒被測到。
    fireEvent.change(screen.getByPlaceholderText("例如：專案文件"), {
      target: { value: "我的筆記本" },
    });

    // 不能用 getByRole("button", { name: "選擇目錄" })：按鈕包在 <label> 裡，
    // 而 button 是 labelable element，可及名稱會變成 label 的文字「資料夾」，
    // 不是按鈕本身的內容。改用文字直接抓按鈕。
    fireEvent.click(screen.getByText("選擇目錄"));
    await screen.findByText("/tmp/docs");

    const input = await screen.findByPlaceholderText("例如：nomic-embed-text");
    fireEvent.change(input, { target: { value: "Qwen3.6-35B-A3B-4bit" } });

    expect(await screen.findByText(/看起來不像 embedding 模型/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "建立" })).not.toBeDisabled();
  });
});
