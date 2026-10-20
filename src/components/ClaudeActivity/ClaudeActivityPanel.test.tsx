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
const card = (id: string) => document.querySelector(`[data-node-id="${id}"]`) as HTMLElement;

beforeEach(() => localStorage.clear());

describe("ClaudeActivityPanel（流程圖）", () => {
  it("標題顯示執行中數量與總數；圖上有 Claude 根節點", () => {
    render(<ClaudeActivityPanel nodes={[node({ id: "a" }), node({ id: "b", status: "done", ended_ms: 2_000 })]} fill />);
    expect(screen.getAllByText("執行中 1").length).toBeGreaterThan(0);
    expect(screen.getAllByText("共 2 個").length).toBeGreaterThan(0);
    expect(card("__root__").textContent).toContain("Claude");
  });

  it("每個 subagent 一張卡，連線從根指向它；沒有描述用佔位文字", () => {
    render(<ClaudeActivityPanel nodes={[node({ id: "a", description: "跑 ls" }), node({ id: "b" })]} fill />);
    expect(card("a").textContent).toContain("跑 ls");
    expect(card("b").textContent).toContain("（未命名任務）");
    expect(screen.getByTestId("edge-__root__-a")).toBeInTheDocument();
    expect(screen.getByTestId("edge-__root__-b")).toBeInTheDocument();
  });

  it("巢狀：子節點在父節點右邊一欄，連線接在父子之間", () => {
    render(
      <ClaudeActivityPanel
        fill
        nodes={[node({ id: "p", description: "外層" }), node({ id: "c", parent: "p", description: "內層" })]}
      />,
    );
    expect(card("p").getAttribute("data-col")).toBe("1");
    expect(card("c").getAttribute("data-col")).toBe("2");
    expect(screen.getByTestId("edge-p-c")).toBeInTheDocument();
  });

  it("連線狀態跟著子節點：執行中的會流動（data-status=running）", () => {
    render(
      <ClaudeActivityPanel
        fill
        nodes={[node({ id: "r", description: "跑中" }), node({ id: "d", description: "做完", status: "done", ended_ms: 2_000 })]}
      />,
    );
    expect(screen.getByTestId("edge-__root__-r").getAttribute("data-status")).toBe("running");
    expect(screen.getByTestId("edge-__root__-d").getAttribute("data-status")).toBe("done");
  });

  it("卡片：執行中顯示目前工具；結束後顯示耗時與 token", () => {
    render(
      <ClaudeActivityPanel
        fill
        nodes={[
          node({ id: "r", description: "跑中", current_tool: "Bash: ls -la" }),
          node({ id: "d", description: "做完", status: "done", started_ms: 1_000, ended_ms: 5_900, tokens: 20484, tool_count: 2 }),
        ]}
      />,
    );
    expect(card("r").textContent).toContain("Bash: ls -la");
    expect(card("d").textContent).toContain("4.9s");
    expect(card("d").textContent).toContain("20.5k tokens");
  });

  it("點節點在下方顯示詳情（含結果摘要），再點一次取消；沒選時顯示提示", () => {
    render(
      <ClaudeActivityPanel
        fill
        nodes={[node({ id: "c", description: "內層", status: "done", ended_ms: 2_000, summary: "檔名是 events.jsonl", tool_count: 3 })]}
      />,
    );
    expect(screen.getByText("點選節點查看詳情")).toBeInTheDocument();
    expect(screen.queryByText("檔名是 events.jsonl")).toBeNull();
    fireEvent.click(card("c"));
    expect(screen.getByText("檔名是 events.jsonl")).toBeInTheDocument();
    expect(screen.getByText("3 次工具呼叫")).toBeInTheDocument();
    expect(card("c").getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(card("c"));
    expect(screen.queryByText("檔名是 events.jsonl")).toBeNull();
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
    expect(card("f").textContent).toContain("失敗");
    expect(card("i").textContent).toContain("已中斷");
  });

  it("收合只留標題，並記在 localStorage", () => {
    const { unmount } = render(<ClaudeActivityPanel nodes={[node({ id: "a", description: "跑 ls" })]} fill />);
    fireEvent.click(screen.getByRole("button", { name: "收合" }));
    expect(card("a")).toBeNull();
    expect(screen.getByRole("button", { name: "展開" })).toBeInTheDocument();
    unmount();
    render(<ClaudeActivityPanel nodes={[node({ id: "a", description: "跑 ls" })]} fill />);
    expect(card("a")).toBeNull();
  });

  it("放大：開出全視窗覆蓋層（同一張圖），Esc 或按鈕關閉", () => {
    render(<ClaudeActivityPanel nodes={[node({ id: "a", description: "跑 ls" })]} fill />);
    expect(screen.queryByRole("dialog")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "放大" }));
    const dialog = screen.getByRole("dialog");
    expect(dialog.querySelector('[data-node-id="a"]')).not.toBeNull();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "放大" }));
    fireEvent.click(screen.getByRole("button", { name: "關閉" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
