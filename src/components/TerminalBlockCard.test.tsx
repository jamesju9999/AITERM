import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TerminalBlockCard } from "./TerminalBlockCard";
import type { TerminalBlock } from "../hooks/useTerminalBlocks";

function makeBlock(overrides: Partial<TerminalBlock> = {}): TerminalBlock {
  return {
    id: "b1",
    command: "echo hi",
    status: "completed",
    exitCode: 0,
    startTime: 1000,
    endTime: 1500,
    cwd: "/Users/test/project",
    rawOutput: "hi\n",
    renderedLines: [{ spans: [{ text: "hi" }] }],
    gitInfo: { branch: "main", insertions: 2, deletions: 1 },
    ...overrides,
  };
}

describe("TerminalBlockCard", () => {
  it("renders command, cwd, duration, and git info in the header", () => {
    render(<TerminalBlockCard block={makeBlock()} />);
    expect(screen.getByText("echo hi")).toBeInTheDocument();
    expect(screen.getByText(/project/)).toBeInTheDocument();
    expect(screen.getByText(/main/)).toBeInTheDocument();
    expect(screen.getByText(/500ms|0\.5s/)).toBeInTheDocument();
  });

  it("renders output lines from renderedLines", () => {
    render(<TerminalBlockCard block={makeBlock()} />);
    expect(screen.getByText("hi")).toBeInTheDocument();
  });

  it("toggles collapse when the header is clicked", () => {
    render(<TerminalBlockCard block={makeBlock()} />);
    const header = screen.getByTestId("block-header");
    expect(screen.queryByTestId("block-body")).toBeInTheDocument();
    fireEvent.click(header);
    expect(screen.queryByTestId("block-body")).not.toBeInTheDocument();
    fireEvent.click(header);
    expect(screen.queryByTestId("block-body")).toBeInTheDocument();
  });

  it("truncates output beyond 500 lines with an expand affordance", () => {
    const manyLines = Array.from({ length: 600 }, (_, i) => ({ spans: [{ text: `line ${i}` }] }));
    render(<TerminalBlockCard block={makeBlock({ renderedLines: manyLines })} />);
    expect(screen.getByText(/還有 100 行|100 more/i)).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("block-expand"));
    expect(screen.getByText("line 599")).toBeInTheDocument();
  });

  it("calls onAskAi with the command and exit code for failed blocks", () => {
    const onAskAi = vi.fn();
    render(<TerminalBlockCard block={makeBlock({ status: "failed", exitCode: 1 })} onAskAi={onAskAi} />);
    fireEvent.click(screen.getByText(/Ask AI/));
    expect(onAskAi).toHaveBeenCalledWith("echo hi", 1);
  });

  describe("running 中的卡片：持續更新的耗時，不顯示 exit code", () => {
    beforeEach(() => {
      vi.useFakeTimers();
      vi.setSystemTime(1000);
    });
    afterEach(() => {
      vi.useRealTimers();
    });

    it("耗時會隨時間跳動，不用等指令結束才顯示", () => {
      render(
        <TerminalBlockCard
          block={makeBlock({ status: "running", exitCode: undefined, endTime: undefined, startTime: 1000 })}
        />,
      );
      expect(screen.getByText(/0ms/)).toBeInTheDocument();

      act(() => {
        vi.advanceTimersByTime(2300);
      });
      expect(screen.getByText(/2\.3s/)).toBeInTheDocument();
      // running 中不顯示失敗標記（exitCode 是 undefined，不是真的失敗）。
      expect(screen.queryByText(/^exit /)).not.toBeInTheDocument();
    });
  });
});

describe("Bookmark 按鈕的就地回饋", () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  it("按下後呼叫 onBookmark、按鈕短暫顯示「已加入書籤」，之後恢復", () => {
    const onBookmark = vi.fn();
    render(<TerminalBlockCard block={makeBlock()} onBookmark={onBookmark} />);

    fireEvent.click(screen.getByRole("button", { name: "Bookmark" }));
    expect(onBookmark).toHaveBeenCalledWith("echo hi");
    expect(screen.getByRole("button", { name: /已加入書籤/ })).toBeInTheDocument();

    act(() => { vi.advanceTimersByTime(1600); });
    expect(screen.getByRole("button", { name: "Bookmark" })).toBeInTheDocument();
  });
});

describe("書籤去重與複製回饋", () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  it("onBookmark 回傳 false（本來就在書籤裡）時顯示「已在書籤中」", () => {
    render(<TerminalBlockCard block={makeBlock()} onBookmark={() => false} />);
    fireEvent.click(screen.getByRole("button", { name: "Bookmark" }));
    expect(screen.getByRole("button", { name: "已在書籤中" })).toBeInTheDocument();
  });

  it("複製成功顯示「已複製」並在之後恢復", async () => {
    const onCopy = vi.fn().mockResolvedValue(undefined);
    render(<TerminalBlockCard block={makeBlock()} onCopy={onCopy} />);
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "Copy" })); });
    expect(onCopy).toHaveBeenCalledWith("echo hi");
    expect(screen.getByRole("button", { name: /已複製/ })).toBeInTheDocument();
    act(() => { vi.advanceTimersByTime(1600); });
    expect(screen.getByRole("button", { name: "Copy" })).toBeInTheDocument();
  });

  it("複製失敗（Promise reject）顯示「複製失敗」，不是假裝成功", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    const onCopy = vi.fn().mockRejectedValue(new Error("denied"));
    render(<TerminalBlockCard block={makeBlock()} onCopy={onCopy} />);
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "Copy" })); });
    expect(screen.getByRole("button", { name: "複製失敗" })).toBeInTheDocument();
  });
});
