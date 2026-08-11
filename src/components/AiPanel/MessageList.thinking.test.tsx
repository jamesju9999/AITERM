import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { MessageList } from "./MessageList";
import type { McpChatMessage } from "../../hooks/useMcpChat";

function renderList(overrides: {
  streamBuf?: string;
  isStreaming?: boolean;
  messages?: McpChatMessage[];
}) {
  return render(
    <MessageList
      messages={overrides.messages ?? []}
      streamBuf={overrides.streamBuf ?? ""}
      isStreaming={overrides.isStreaming ?? false}
      error={null}
      onExecuteCommand={vi.fn()}
      onRetry={vi.fn()}
    />,
  );
}

describe("MessageList — 等待第一個字時的指示", () => {
  /**
   * 送出之後到第一個 delta 之間，`streamBuf` 是空的。原本這段時間畫面完全
   * 空白——使用者合理地以為沒在運作（實測回報）。
   *
   * 這段空窗在 ChatGPT Web 那條路徑上特別長：要先跑 sentinel 的兩段握手與
   * 工作量證明，模型才開始吐字。
   */
  it("串流中但還沒有任何文字時，要顯示思考指示", () => {
    renderList({ isStreaming: true, streamBuf: "" });
    expect(screen.getByText(/思考中|Thinking/)).toBeInTheDocument();
  });

  /**
   * 有字之後就該交給串流氣泡，兩個同時出現會變成「思考中」黏在已經開始的
   * 回覆上方，看起來像卡住了。
   */
  it("第一個字進來之後，指示要讓位給串流內容", () => {
    renderList({ isStreaming: true, streamBuf: "好的，我來" });
    expect(screen.getByText("好的，我來")).toBeInTheDocument();
    expect(screen.queryByText(/思考中|Thinking/)).not.toBeInTheDocument();
  });

  it("沒有在串流時不顯示指示", () => {
    renderList({ isStreaming: false, streamBuf: "" });
    expect(screen.queryByText(/思考中|Thinking/)).not.toBeInTheDocument();
  });

  /**
   * 螢幕閱讀器要知道這裡正在等待，否則對它的使用者來說畫面依然是「什麼都
   * 沒發生」——那正是這個指示要解決的問題。
   */
  it("指示要標記成 busy，讓螢幕閱讀器也知道在等待", () => {
    const { container } = renderList({ isStreaming: true, streamBuf: "" });
    expect(container.querySelector('[aria-busy="true"]')).not.toBeNull();
  });
});
