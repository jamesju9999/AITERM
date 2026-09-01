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

  it("renders a two-column split with the artifact panel when a message contains an artifact-html block", () => {
    const messages = [
      { role: "assistant" as const, content: "```artifact-html\n<title>Brief</title><p>hi</p>\n```" },
    ];
    render(<ChatPanelShell {...base({ messages })} />);
    // "Brief" 合理地出現兩處：聊天泡泡裡的 artifact 卡片、以及文件面板的標題列。
    expect(screen.getAllByText("Brief").length).toBeGreaterThanOrEqual(1);
    expect(document.querySelector(".aiterm-ai-panel--split")).not.toBeNull();
    expect(document.querySelector("iframe")).not.toBeNull();
  });

  it("does not render the split layout when no message has an artifact block", () => {
    render(<ChatPanelShell {...base()} />);
    expect(document.querySelector(".aiterm-ai-panel--split")).toBeNull();
    expect(document.querySelector(".aiterm-artifact-panel")).toBeNull();
  });

  // 有 artifact 時面板會自動撐寬，但「縮小面板」必須真的能縮——先前的寫法是
  // effectiveExpanded = expanded || !!activeArtifact，按鈕只切換 expanded，
  // 於是 artifact 開著時不管怎麼點，effectiveExpanded 都是 true：一顆按了沒
  // 反應的死按鈕。使用者想把終端機要回來時無路可走。
  it("minimize actually narrows the panel while an artifact is open", () => {
    const messages = [
      { role: "assistant" as const, content: "```artifact-html\n<title>Brief</title>\n```" },
    ];
    render(<ChatPanelShell {...base({ messages })} />);

    const panel = document.querySelector(".aiterm-ai-panel")!;
    expect(panel.className).toContain("aiterm-ai-panel--expanded");

    fireEvent.click(screen.getByTitle("縮小面板"));
    expect(panel.className).not.toContain("aiterm-ai-panel--expanded");
    // 文件仍然開著——縮小的是面板寬度，不是把文件關掉。
    expect(document.querySelector(".aiterm-artifact-panel")).not.toBeNull();
  });

  it("closing the artifact panel collapses back to a single column", () => {
    const messages = [
      { role: "assistant" as const, content: "```artifact-html\n<title>Brief</title>\n```" },
    ];
    render(<ChatPanelShell {...base({ messages })} />);
    fireEvent.click(screen.getByTitle("關閉文件面板"));
    expect(document.querySelector(".aiterm-ai-panel--split")).toBeNull();
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
