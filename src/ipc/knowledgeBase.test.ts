import { describe, it, expect, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import {
  kbCreateNotebook, kbListNotebooks, kbDeleteNotebook, kbSyncNotebook, invokeKbChat,
} from "./knowledgeBase";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

describe("knowledgeBase ipc", () => {
  it("kbCreateNotebook invokes kb_create_notebook with camelCase args", async () => {
    vi.mocked(invoke).mockResolvedValue({ id: "nb-1" });
    await kbCreateNotebook("My Notes", "/tmp/docs", "ollama-local", "nomic-embed-text");
    expect(invoke).toHaveBeenCalledWith("kb_create_notebook", {
      name: "My Notes",
      folderPath: "/tmp/docs",
      embedProviderId: "ollama-local",
      embedModel: "nomic-embed-text",
    });
  });

  it("kbListNotebooks invokes kb_list_notebooks with no args", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    await kbListNotebooks();
    expect(invoke).toHaveBeenCalledWith("kb_list_notebooks");
  });

  it("kbDeleteNotebook invokes kb_delete_notebook with id", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);
    await kbDeleteNotebook("nb-1");
    expect(invoke).toHaveBeenCalledWith("kb_delete_notebook", { id: "nb-1" });
  });

  it("kbSyncNotebook invokes kb_sync_notebook with notebookId", async () => {
    vi.mocked(invoke).mockResolvedValue({ indexed: 0, failed: 0, deleted: 0 });
    await kbSyncNotebook("nb-1");
    expect(invoke).toHaveBeenCalledWith("kb_sync_notebook", { notebookId: "nb-1" });
  });

  it("invokeKbChat invokes kb_chat with full arg set", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);
    await invokeKbChat("nb-1", [{ role: "user", content: "hi" }], "sess-1", "openai-1", "en");
    expect(invoke).toHaveBeenCalledWith("kb_chat", {
      notebookId: "nb-1",
      messages: [{ role: "user", content: "hi" }],
      sessionId: "sess-1",
      providerId: "openai-1",
      locale: "en",
    });
  });
});
