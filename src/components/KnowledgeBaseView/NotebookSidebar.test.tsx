import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { NotebookSidebar } from "./NotebookSidebar";
import { LocaleProvider } from "../../contexts/LocaleContext";
import type { Notebook } from "../../ipc/knowledgeBase";

// Tauri's webview does not implement the JS dialog panels, so window.confirm()
// returns without ever showing anything — a delete guarded by it just happens.
// These tests assert against the plugin's confirm precisely because asserting
// on window.confirm would keep passing in jsdom while production deletes
// without asking.
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn() }));
import { confirm } from "@tauri-apps/plugin-dialog";

const NOTEBOOK: Notebook = {
  id: "nb1",
  name: "TEST",
  folder_path: "/tmp/docs",
  embed_provider_id: "ollama-local",
  embed_model: "nomic-embed-text",
  embed_dim: 768,
  last_synced_at: 1700000000,
  created_at: "2026-01-01T00:00:00Z",
};

function renderSidebar(onDelete = vi.fn()) {
  render(
    <LocaleProvider>
      <NotebookSidebar
        notebooks={[NOTEBOOK]}
        activeId={null}
        syncingId={null}
        syncProgress={null}
        onSelect={vi.fn()}
        onSync={vi.fn()}
        onDelete={onDelete}
        onAddClick={vi.fn()}
      />
    </LocaleProvider>
  );
  return onDelete;
}

function clickDelete() {
  fireEvent.click(screen.getByRole("button", { name: "✕" }));
}

describe("NotebookSidebar delete confirmation", () => {
  beforeEach(() => {
    vi.mocked(confirm).mockReset();
  });

  it("asks before deleting, naming the notebook", async () => {
    vi.mocked(confirm).mockResolvedValue(true);
    renderSidebar();

    clickDelete();

    await waitFor(() => {
      expect(vi.mocked(confirm)).toHaveBeenCalledWith(
        expect.stringContaining("TEST"),
        expect.anything()
      );
    });
  });

  // Without explicit labels the buttons come from the OS, not the app's locale
  // setting — an English UI on a Chinese macOS would read "Delete notebook…?"
  // over 好/取消. "刪除" also beats "確定" for saying what the button does.
  it("labels the buttons from the app's locale, not the OS", async () => {
    vi.mocked(confirm).mockResolvedValue(true);
    renderSidebar();

    clickDelete();

    await waitFor(() => {
      expect(vi.mocked(confirm)).toHaveBeenCalledWith(
        expect.any(String),
        expect.objectContaining({ okLabel: "刪除", cancelLabel: "取消" })
      );
    });
  });

  it("does not delete when the user cancels", async () => {
    vi.mocked(confirm).mockResolvedValue(false);
    const onDelete = renderSidebar();

    clickDelete();

    await waitFor(() => expect(vi.mocked(confirm)).toHaveBeenCalled());
    expect(onDelete).not.toHaveBeenCalled();
  });

  it("deletes when the user confirms", async () => {
    vi.mocked(confirm).mockResolvedValue(true);
    const onDelete = renderSidebar();

    clickDelete();

    await waitFor(() => expect(onDelete).toHaveBeenCalledWith("nb1"));
  });
});
