import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MessageBubble } from "./MessageBubble";

describe("MessageBubble", () => {
  it("renders a user bubble with plain text", () => {
    render(
      <MessageBubble
        role="user"
        content="列出所有檔案"
        onExecuteCommand={vi.fn()}
      />,
    );
    expect(screen.getByText("列出所有檔案")).toBeInTheDocument();
  });

  it("renders an assistant bubble splitting text and cmd tags", () => {
    const onExec = vi.fn();
    render(
      <MessageBubble
        role="assistant"
        content="建議執行 <cmd>ls -la</cmd> 試試"
        onExecuteCommand={onExec}
      />,
    );
    // Text fragments present
    expect(screen.getByText("建議執行 ")).toBeInTheDocument();
    expect(screen.getByText(" 試試")).toBeInTheDocument();
    // Cmd button present and clickable
    const btn = screen.getByRole("button", { name: /ls -la/ });
    expect(btn).toBeInTheDocument();
  });

  it("clicking an assistant cmd calls onExecuteCommand", async () => {
    const onExec = vi.fn();
    render(
      <MessageBubble
        role="assistant"
        content="<cmd>pwd</cmd>"
        onExecuteCommand={onExec}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: /pwd/ }));
    expect(onExec).toHaveBeenCalledWith("pwd");
  });

  it("renders a streaming bubble (in-progress assistant output)", () => {
    render(
      <MessageBubble
        role="assistant"
        content="正在生成..."
        onExecuteCommand={vi.fn()}
        streaming
      />,
    );
    expect(screen.getByText("正在生成...")).toBeInTheDocument();
    // Streaming bubble has a distinguishing class — assert via aria-busy.
    const bubble = screen.getByText("正在生成...").closest("[aria-busy]");
    expect(bubble).not.toBeNull();
    expect(bubble).toHaveAttribute("aria-busy", "true");
  });
});
