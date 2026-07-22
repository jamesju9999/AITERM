import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
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
});
