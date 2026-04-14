import { describe, expect, it, vi, beforeEach } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// Mock Tauri before importing AiPanel (which imports useAiChat).
const invokeMock = vi.fn();
const listenMock = vi.fn().mockResolvedValue(() => {});
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

import { AiPanel } from "./index";

beforeEach(() => {
  invokeMock.mockReset();
  listenMock.mockClear();
  listenMock.mockResolvedValue(() => {});
});

describe("AiPanel", () => {
  it("hides the panel via CSS class when isOpen=false", () => {
    const { container } = render(
      <AiPanel
        sessionId="s1"
        isOpen={false}
        providerName="Ollama (llama3)"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );
    // Panel element exists but has the hidden class — content stays mounted
    // so the chat hook keeps its listener alive while the user toggles.
    const panel = container.querySelector(".aiterm-ai-panel");
    expect(panel).not.toBeNull();
    expect(panel).toHaveClass("aiterm-ai-panel-hidden");
    // Textarea exists in DOM (not unmounted).
    expect(screen.getByRole("textbox")).toBeInTheDocument();
  });

  it("autofocuses the textarea when transitioning to open", () => {
    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Ollama"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );
    const textbox = screen.getByRole("textbox");
    expect(textbox).toHaveFocus();
  });

  it("calls onClose when Escape pressed", async () => {
    const onClose = vi.fn();
    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Ollama"
        onClose={onClose}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalled();
  });

  it("sends a message when Enter pressed", async () => {
    invokeMock.mockResolvedValueOnce({ content: "好的" });
    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Ollama"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );
    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;
    await userEvent.type(textbox, "列出檔案");
    await userEvent.keyboard("{Enter}");

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "ai_chat",
        expect.objectContaining({
          messages: [{ role: "user", content: "列出檔案" }],
          sessionId: "s1",
        }),
      ),
    );
    await waitFor(() => expect(screen.getByText("好的")).toBeInTheDocument());
  });

  it("🗑 New Chat button clears messages", async () => {
    invokeMock.mockResolvedValueOnce({ content: "ok" });
    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Ollama"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );
    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;
    await userEvent.type(textbox, "hi");
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(screen.getByText("ok")).toBeInTheDocument());

    await userEvent.click(screen.getByRole("button", { name: /New Chat/ }));
    expect(screen.queryByText("ok")).toBeNull();
  });

  it("provider badge calls onOpenProviderPalette when clicked", async () => {
    const onOpenProviderPalette = vi.fn();
    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Claude"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={onOpenProviderPalette}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: /Claude/ }));
    expect(onOpenProviderPalette).toHaveBeenCalled();
  });
});
