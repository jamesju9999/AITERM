import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ChatPanelShell, type ChatPanelShellProps } from "./ChatPanelShell";

// 用**真實**的 translations，理由同 AiPanel.test.tsx：手寫替身容易漏 key，
// 元件渲染成空白但測試還是綠的。
vi.mock("../../contexts/LocaleContext", async () => {
  const { translations } = await vi.importActual<typeof import("../../lib/i18n")>(
    "../../lib/i18n",
  );
  return {
    useLocale: () => ({
      locale: "zh-TW" as const,
      t: translations["zh-TW"],
      setLocale: () => {},
    }),
  };
});

function base(over: Partial<ChatPanelShellProps> = {}): ChatPanelShellProps {
  return {
    isOpen: true, onClose: vi.fn(),
    messages: [], streamBuf: "", isStreaming: false, thinkingLabel: null, error: null,
    onRetry: vi.fn(), onExecuteCommand: vi.fn(),
    agentMode: false, onToggleAgentMode: vi.fn(), onSend: vi.fn(), onSubmitAgent: vi.fn(),
    mode: "suggest", maxAgentSteps: 5,
    agentRunning: false, agentPhase: "thinking", agentStep: 0, onAbortAgent: vi.fn(),
    providerName: "TestProv", onOpenProviderPalette: vi.fn(),
    sessions: [], onLoadSession: vi.fn(), onNewChat: vi.fn(), onDeleteSession: vi.fn(),
    ...over,
  };
}

describe("ChatPanelShell", () => {
  it("routes plain submit to onSend", async () => {
    const onSend = vi.fn();
    render(<ChatPanelShell {...base({ onSend })} />);
    await userEvent.type(screen.getByRole("textbox"), "hello");
    await userEvent.keyboard("{Enter}");
    expect(onSend).toHaveBeenCalledWith("hello");
  });

  it("routes submit to onSubmitAgent when agentMode", async () => {
    const onSubmitAgent = vi.fn();
    render(<ChatPanelShell {...base({ agentMode: true, onSubmitAgent })} />);
    await userEvent.type(screen.getByRole("textbox"), "do it{Enter}");
    expect(onSubmitAgent).toHaveBeenCalledWith("do it");
  });

  it("provider badge opens palette", async () => {
    const onOpenProviderPalette = vi.fn();
    render(<ChatPanelShell {...base({ onOpenProviderPalette })} />);
    await userEvent.click(screen.getByText("TestProv"));
    expect(onOpenProviderPalette).toHaveBeenCalled();
  });

  it("New Chat calls onNewChat", async () => {
    const onNewChat = vi.fn();
    render(<ChatPanelShell {...base({ onNewChat })} />);
    await userEvent.click(screen.getByTitle(/清空當前對話|New Chat/i));
    expect(onNewChat).toHaveBeenCalled();
  });

  it("shows the stop button and calls onAbortAgent while agentRunning", async () => {
    const onAbortAgent = vi.fn();
    render(<ChatPanelShell {...base({ agentRunning: true, onAbortAgent })} />);
    await userEvent.click(screen.getByTitle("停止"));
    expect(onAbortAgent).toHaveBeenCalled();
  });

  it("renders extraInputControls and extraAboveInput slots", () => {
    render(<ChatPanelShell {...base({
      extraInputControls: <button>MCP-SLOT</button>,
      extraAboveInput: <div>ABOVE-SLOT</div>,
    })} />);
    expect(screen.getByText("MCP-SLOT")).toBeInTheDocument();
    expect(screen.getByText("ABOVE-SLOT")).toBeInTheDocument();
  });

  it('submitShortcut="shift-enter": plain Enter does nothing, Shift+Enter submits', async () => {
    const onSend = vi.fn();
    render(<ChatPanelShell {...base({ onSend, submitShortcut: "shift-enter" })} />);
    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;
    await userEvent.type(textbox, "hello");

    fireEvent.keyDown(textbox, { key: "Enter" });
    expect(onSend).not.toHaveBeenCalled();

    fireEvent.keyDown(textbox, { key: "Enter", shiftKey: true });
    expect(onSend).toHaveBeenCalledWith("hello");
  });

  it('submitShortcut="ctrl-enter": plain Enter does nothing, Ctrl+Enter submits', async () => {
    const onSend = vi.fn();
    render(<ChatPanelShell {...base({ onSend, submitShortcut: "ctrl-enter" })} />);
    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;
    await userEvent.type(textbox, "hello");

    fireEvent.keyDown(textbox, { key: "Enter" });
    expect(onSend).not.toHaveBeenCalled();

    fireEvent.keyDown(textbox, { key: "Enter", ctrlKey: true });
    expect(onSend).toHaveBeenCalledWith("hello");
  });

  it("allowEmptySubmit lets Enter submit with empty text; default blocks it", () => {
    const onSend = vi.fn();
    const { rerender } = render(<ChatPanelShell {...base({ onSend })} />);
    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;

    // default (allowEmptySubmit=false): empty input, Enter does nothing.
    fireEvent.keyDown(textbox, { key: "Enter" });
    expect(onSend).not.toHaveBeenCalled();

    rerender(<ChatPanelShell {...base({ onSend, allowEmptySubmit: true })} />);
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter" });
    expect(onSend).toHaveBeenCalledWith("");
  });

  it("inputPrefixControls renders inside the input pill container", () => {
    render(<ChatPanelShell {...base({
      inputPrefixControls: <button>PREFIX-SLOT</button>,
    })} />);
    expect(screen.getByText("PREFIX-SLOT")).toBeInTheDocument();
  });

  it("forwards paste events on the textarea to onPaste", () => {
    const onPaste = vi.fn();
    render(<ChatPanelShell {...base({ onPaste })} />);
    const textbox = screen.getByRole("textbox");
    fireEvent.paste(textbox, { clipboardData: { getData: () => "", files: [] } });
    expect(onPaste).toHaveBeenCalled();
  });
});
