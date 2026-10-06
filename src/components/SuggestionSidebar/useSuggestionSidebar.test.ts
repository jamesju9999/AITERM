import { describe, expect, it } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { useSuggestionSidebar, type SidebarBlock } from "./useSuggestionSidebar";

const blk = (id: string, command: string, status: SidebarBlock["status"] = "running"): SidebarBlock => ({ id, command, status });

describe("useSuggestionSidebar", () => {
  it("is closed by default and reports no AI tool when nothing is running", () => {
    const { result } = renderHook(() => useSuggestionSidebar([], []));
    expect(result.current.open).toBe(false);
    expect(result.current.aiCliRunning).toBe(false);
  });

  it("detects a running AI tool from the latest block only", () => {
    const { result, rerender } = renderHook(({ blocks }) => useSuggestionSidebar(blocks, []), {
      initialProps: { blocks: [blk("1", "claude")] },
    });
    expect(result.current.aiCliRunning).toBe(true);
    rerender({ blocks: [blk("1", "claude", "completed"), blk("2", "ls")] });
    expect(result.current.aiCliRunning).toBe(false);
    rerender({ blocks: [blk("1", "ls", "completed"), blk("2", "codex")] });
    expect(result.current.aiCliRunning).toBe(true);
  });

  it("a finished AI tool block does not count as running", () => {
    const { result } = renderHook(() => useSuggestionSidebar([blk("1", "claude", "completed")], []));
    expect(result.current.aiCliRunning).toBe(false);
  });

  it("recognises custom tool names", () => {
    const { result } = renderHook(() => useSuggestionSidebar([blk("1", "mytool go")], ["mytool"]));
    expect(result.current.aiCliRunning).toBe(true);
  });

  it("never opens by itself when an AI tool starts (it would resize the terminal mid-startup)", () => {
    const { result, rerender } = renderHook(({ blocks }) => useSuggestionSidebar(blocks, []), {
      initialProps: { blocks: [] as SidebarBlock[] },
    });
    rerender({ blocks: [blk("1", "claude")] });
    expect(result.current.aiCliRunning).toBe(true);
    expect(result.current.open).toBe(false);
  });

  it("closes automatically when the AI tool ends, if it was opened while the tool was running", () => {
    const { result, rerender } = renderHook(({ blocks }) => useSuggestionSidebar(blocks, []), {
      initialProps: { blocks: [blk("1", "claude")] },
    });
    act(() => result.current.toggle());
    expect(result.current.open).toBe(true);
    rerender({ blocks: [blk("1", "claude", "completed")] });
    expect(result.current.open).toBe(false);
  });

  it("stays open when it was opened manually with no AI tool running, even if one starts and ends", () => {
    const { result, rerender } = renderHook(({ blocks }) => useSuggestionSidebar(blocks, []), {
      initialProps: { blocks: [] as SidebarBlock[] },
    });
    act(() => result.current.toggle());
    expect(result.current.open).toBe(true);
    rerender({ blocks: [blk("1", "claude")] });
    rerender({ blocks: [blk("1", "claude", "completed")] });
    expect(result.current.open).toBe(true);
  });

  it("toggle closes an open sidebar and reopening works", () => {
    const { result } = renderHook(() => useSuggestionSidebar([], []));
    act(() => result.current.toggle());
    act(() => result.current.toggle());
    expect(result.current.open).toBe(false);
    act(() => result.current.toggle());
    expect(result.current.open).toBe(true);
  });

  it("close() closes", () => {
    const { result } = renderHook(() => useSuggestionSidebar([], []));
    act(() => result.current.toggle());
    act(() => result.current.close());
    expect(result.current.open).toBe(false);
  });
});
