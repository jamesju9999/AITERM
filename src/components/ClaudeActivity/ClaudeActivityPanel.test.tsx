import { describe, expect, it, vi, beforeEach } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import type { ActivityNode } from "../../ipc/claudeActivity";

vi.mock("../../contexts/LocaleContext", async () => {
  const { translations } = await vi.importActual<typeof import("../../lib/i18n")>("../../lib/i18n");
  return { useLocale: () => ({ locale: "zh-TW" as const, t: translations["zh-TW"], setLocale: () => {} }) };
});

import { ClaudeActivityPanel } from "./ClaudeActivityPanel";

const node = (over: Partial<ActivityNode> & { id: string }): ActivityNode => ({
  parent: null, agent_type: "general-purpose", description: null, status: "running",
  started_ms: 1_000, ended_ms: null, current_tool: null, summary: null, tokens: null, tool_count: null,
  ...over,
});

beforeEach(() => localStorage.clear());

describe("ClaudeActivityPanel", () => {
  it("標題顯示執行中數量與總數", () => {
    render(<ClaudeActivityPanel nodes={[node({ id: "a" }), node({ id: "b", status: "done", ended_ms: 2_000 })]} fill />);
    expect(screen.getByText("Claude 協作")).toBeInTheDocument();
    expect(screen.getByText("執行中 1")).toBeInTheDocument();
    expect(screen.getByText("共 2 個")).toBeInTheDocument();
  });

  it("每列顯示任務描述、型別、狀態；沒有描述用佔位文字", () => {
    render(<ClaudeActivityPanel nodes={[node({ id: "a", description: "跑 ls" }), node({ id: "b" })]} fill />);
    expect(screen.getByText("跑 ls")).toBeInTheDocument();
    expect(screen.getByText("（未命名任務）")).toBeInTheDocument();
    expect(screen.getAllByText("general-purpose")).toHaveLength(2);
  });

  it("執行中的節點顯示目前工具；結束後顯示耗時、token 與工具次數", () => {
    render(
      <ClaudeActivityPanel
        fill
        nodes={[
          node({ id: "r", description: "跑中", current_tool: "Bash: ls -la" }),
          node({ id: "d", description: "做完", status: "done", started_ms: 1_000, ended_ms: 5_900, tokens: 20484, tool_count: 2 }),
        ]}
      />,
    );
    expect(screen.getByText("Bash: ls -la")).toBeInTheDocument();
    expect(screen.getByText("4.9s")).toBeInTheDocument();
    expect(screen.getByText("20.5k tokens")).toBeInTheDocument();
    expect(screen.getByText("2 次工具呼叫")).toBeInTheDocument();
  });

  it("子節點縮排；結果摘要預設收起，按鈕展開", () => {
    render(
      <ClaudeActivityPanel
        fill
        nodes={[
          node({ id: "p", description: "外層" }),
          node({ id: "c", parent: "p", description: "內層", status: "done", ended_ms: 2_000, summary: "檔名是 events.jsonl" }),
        ]}
      />,
    );
    const inner = screen.getByText("內層").closest("[data-depth]") as HTMLElement;
    expect(inner.getAttribute("data-depth")).toBe("1");
    expect(screen.queryByText("檔名是 events.jsonl")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "查看結果" }));
    expect(screen.getByText("檔名是 events.jsonl")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "收起結果" }));
    expect(screen.queryByText("檔名是 events.jsonl")).toBeNull();
  });

  it("收合只留標題，並記在 localStorage", () => {
    const { unmount } = render(<ClaudeActivityPanel nodes={[node({ id: "a", description: "跑 ls" })]} fill />);
    fireEvent.click(screen.getByRole("button", { name: "收合" }));
    expect(screen.queryByText("跑 ls")).toBeNull();
    expect(screen.getByRole("button", { name: "展開" })).toBeInTheDocument();
    unmount();
    render(<ClaudeActivityPanel nodes={[node({ id: "a", description: "跑 ls" })]} fill />);
    expect(screen.queryByText("跑 ls")).toBeNull();
  });

  it("狀態用文字標示（不只靠顏色）", () => {
    render(
      <ClaudeActivityPanel
        fill
        nodes={[
          node({ id: "f", description: "x", status: "failed", ended_ms: 2_000 }),
          node({ id: "i", description: "y", status: "interrupted", ended_ms: 2_000 }),
        ]}
      />,
    );
    expect(screen.getByText("失敗")).toBeInTheDocument();
    expect(screen.getByText("已中斷")).toBeInTheDocument();
  });
});
