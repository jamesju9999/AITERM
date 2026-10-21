import { describe, expect, it, vi, beforeEach } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { ActivityNode, ActivitySnapshot, ToolCall } from "../../ipc/claudeActivity";

vi.mock("../../contexts/LocaleContext", async () => {
  const { translations } = await vi.importActual<typeof import("../../lib/i18n")>("../../lib/i18n");
  return { useLocale: () => ({ locale: "zh-TW" as const, t: translations["zh-TW"], setLocale: () => {} }) };
});

const getToolDetail = vi.fn();
vi.mock("../../ipc/claudeActivity", async () => {
  const actual = await vi.importActual<typeof import("../../ipc/claudeActivity")>("../../ipc/claudeActivity");
  return { ...actual, getClaudeToolDetail: (...a: unknown[]) => getToolDetail(...a) };
});

import { ClaudeActivityPanel } from "./ClaudeActivityPanel";

const node = (over: Partial<ActivityNode> & { id: string }): ActivityNode => ({
  parent: null, agent_type: "general-purpose", description: null, status: "running",
  started_ms: 1_000, ended_ms: null, current_tool: null, summary: null, tokens: null, tool_count: null, last_event_ms: 1_000, turn: 1,
  ...over,
});
const done = (id: string, over: Partial<ActivityNode> = {}) =>
  node({ id, description: id, status: "done", ended_ms: 2_000, ...over });
const snap = (nodes: ActivityNode[], over: Partial<ActivitySnapshot> = {}): ActivitySnapshot => ({
  nodes, main_tool: null, main_tool_count: 0, main_recent: [], tools: [], turn: Math.max(0, ...nodes.map((n) => n.turn)), ...over,
});
const renderPanel = (nodes: ActivityNode[], over: Partial<ActivitySnapshot> = {}) =>
  render(<ClaudeActivityPanel snapshot={snap(nodes, over)} fill sessionId="sess-1" />);
const card = (id: string) => document.querySelector(`[data-node-id="${id}"]`) as HTMLElement;

// 預設是收合；其餘測試要看圖，所以先記成「展開過」。
beforeEach(() => {
  getToolDetail.mockReset();
  getToolDetail.mockResolvedValue(null);
  localStorage.clear();
  localStorage.setItem("aiterm-claude-activity-collapsed", "0");
});

describe("ClaudeActivityPanel（流程圖）", () => {
  it("標題顯示執行中數量與總數；圖上有 Claude 根節點", () => {
    renderPanel([node({ id: "a" }), done("b")]);
    expect(screen.getAllByText("執行中 1").length).toBeGreaterThan(0);
    expect(screen.getAllByText("共 2 個").length).toBeGreaterThan(0);
    expect(card("__root__").textContent).toContain("Claude");
  });

  it("每個 subagent 一張卡，連線從根指向它；沒有描述用佔位文字", () => {
    renderPanel([node({ id: "a", description: "跑 ls" }), node({ id: "b" })]);
    expect(card("a").textContent).toContain("跑 ls");
    expect(card("b").textContent).toContain("（未命名任務）");
    expect(screen.getByTestId("edge-__root__-a")).toBeInTheDocument();
    expect(screen.getByTestId("edge-__root__-b")).toBeInTheDocument();
  });

  it("標題超長時卡片有 title 屬性放全文", () => {
    const long = "整理專案檔案結構並檢查未使用的匯出與重複的相依";
    renderPanel([node({ id: "a", description: long })]);
    expect(card("a").getAttribute("title")).toBe(long);
  });

  it("巢狀：子節點在父節點右邊一欄，連線接在父子之間", () => {
    renderPanel([node({ id: "p", description: "外層" }), node({ id: "c", parent: "p", description: "內層" })]);
    expect(card("p").getAttribute("data-col")).toBe("1");
    expect(card("c").getAttribute("data-col")).toBe("2");
    expect(screen.getByTestId("edge-p-c")).toBeInTheDocument();
  });

  it("連線狀態跟著子節點；只有執行中的連線有移動的小點", () => {
    renderPanel([node({ id: "r", description: "跑中" }), done("d")]);
    expect(screen.getByTestId("edge-__root__-r").getAttribute("data-status")).toBe("running");
    expect(screen.getByTestId("edge-__root__-d").getAttribute("data-status")).toBe("done");
    expect(screen.getByTestId("fly-__root__-r")).toBeInTheDocument();
    expect(screen.queryByTestId("fly-__root__-d")).toBeNull();
  });

  it("有子節點的 subagent 有外框；沒有子節點的沒有；jsdom 量不到視窗時不顯示小地圖", () => {
    renderPanel([node({ id: "p", description: "外層" }), node({ id: "c", parent: "p", description: "內層" })]);
    expect(screen.getByTestId("frame-p")).toBeInTheDocument();
    expect(screen.queryByTestId("frame-c")).toBeNull();
    expect(screen.queryByTestId("minimap")).toBeNull();
  });

  it("標題的「執行中／共幾個」也算進工具，不只 subagent", () => {
    renderPanel([node({ id: "a", description: "跑 ls" })], {
      tools: ["running", "done"].map((status, i) => ({
        id: `t${i}`, parent: null, tool: "Bash", hint: `Bash: t${i}`, category: "shell", status: status as "running" | "done",
        started_ms: 1_000, ended_ms: status === "done" ? 1_500 : null, turn: 1,
      })),
    });
    const header = document.querySelector(".aiterm-cact__header")!.textContent!;
    expect(header).toContain("執行中 2");
    expect(header).toContain("共 3 個");
  });

  it("執行中的連線有彗星尾巴（小點之外還有兩個漸淡的點）", () => {
    renderPanel([node({ id: "r", description: "跑中" })]);
    expect(document.querySelectorAll("circle.aiterm-cact__fly").length).toBe(3);
  });

  it("卡片：執行中顯示目前工具；結束後顯示耗時與 token", () => {
    renderPanel([
      node({ id: "r", description: "跑中", current_tool: "Bash: ls -la" }),
      done("d", { started_ms: 1_000, ended_ms: 5_900, tokens: 20484, tool_count: 2 }),
    ]);
    expect(card("r").textContent).toContain("Bash: ls -la");
    expect(card("d").textContent).toContain("4.9s");
    expect(card("d").textContent).toContain("20.5k tokens");
  });

  it("根節點顯示主 Claude 目前的工具", () => {
    renderPanel([node({ id: "a", description: "x" })], { main_tool: "Agent: 整理檔案" });
    expect(card("__root__").textContent).toContain("Agent: 整理檔案");
  });

  it("點節點在下方顯示詳情（含結果摘要），再點一次取消；沒選時顯示提示", () => {
    renderPanel([done("c", { description: "內層", summary: "檔名是 events.jsonl", tool_count: 3 })]);
    expect(screen.getByText("點選節點查看詳情")).toBeInTheDocument();
    fireEvent.click(card("c"));
    expect(screen.getByText("檔名是 events.jsonl")).toBeInTheDocument();
    expect(screen.getByText("3 次工具呼叫")).toBeInTheDocument();
    expect(card("c").getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(card("c"));
    expect(screen.queryByText("檔名是 events.jsonl")).toBeNull();
  });

  it("選中節點：到根的路徑高亮，其餘淡化；取消後恢復", () => {
    renderPanel([
      node({ id: "p", description: "外層" }),
      node({ id: "c", parent: "p", description: "內層" }),
      node({ id: "other", description: "旁支" }),
    ]);
    fireEvent.click(card("c"));
    expect(card("p").className).toContain("is-path");
    expect(card("other").className).toContain("is-dim");
    expect(screen.getByTestId("edge-p-c").getAttribute("class")).toContain("is-path");
    expect(screen.getByTestId("edge-__root__-other").getAttribute("class")).toContain("is-dim");
    fireEvent.click(card("c"));
    expect(card("other").className).not.toContain("is-dim");
  });

  it("狀態用文字標示（不只靠顏色）；失敗的排在同層最前面", () => {
    renderPanel([done("ok", { started_ms: 1 }), done("f", { status: "failed", started_ms: 9 })]);
    expect(card("f").textContent).toContain("失敗");
    expect(Number.parseFloat(card("f").style.top)).toBeLessThan(Number.parseFloat(card("ok").style.top));
  });

  it("第一次（沒存過偏好）：獨佔整欄時預設展開，否則整欄只剩一塊空白", () => {
    localStorage.clear();
    renderPanel([node({ id: "a", description: "跑 ls" })]);
    expect(card("a")).not.toBeNull();
  });

  it("第一次（沒存過偏好）：與建議側欄並排時預設收合；展開後記住展開", () => {
    localStorage.clear();
    const nodes = [node({ id: "a", description: "跑 ls" })];
    const { unmount } = render(<ClaudeActivityPanel snapshot={snap(nodes)} fill={false} sessionId="sess-1" />);
    expect(card("a")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "展開" }));
    expect(card("a")).not.toBeNull();
    unmount();
    render(<ClaudeActivityPanel snapshot={snap(nodes)} fill={false} sessionId="sess-1" />);
    expect(card("a")).not.toBeNull();
  });

  it("與建議側欄並排時：收合只留標題，並記在 localStorage", () => {
    const nodes = [node({ id: "a", description: "跑 ls" })];
    const { unmount } = render(<ClaudeActivityPanel snapshot={snap(nodes)} fill={false} sessionId="sess-1" />);
    fireEvent.click(screen.getByRole("button", { name: "收合" }));
    expect(card("a")).toBeNull();
    expect(screen.getByRole("button", { name: "展開" })).toBeInTheDocument();
    unmount();
    render(<ClaudeActivityPanel snapshot={snap(nodes)} fill={false} sessionId="sess-1" />);
    expect(card("a")).toBeNull();
  });

  it("獨佔整欄時沒有收合按鈕，且忽略先前存的收合偏好（否則會卡在空白、無法展開）", () => {
    localStorage.setItem("aiterm-claude-activity-collapsed", "1");
    renderPanel([node({ id: "a", description: "跑 ls" })]);
    expect(screen.queryByRole("button", { name: "收合" })).toBeNull();
    expect(screen.queryByRole("button", { name: "展開" })).toBeNull();
    expect(card("a")).not.toBeNull();
  });

  it("放大：開出全視窗覆蓋層（同一張圖），Esc 或按鈕關閉", () => {
    renderPanel([node({ id: "a", description: "跑 ls" })]);
    fireEvent.click(screen.getByRole("button", { name: "放大" }));
    expect(screen.getByRole("dialog").querySelector('[data-node-id="a"]')).not.toBeNull();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "放大" }));
    fireEvent.click(screen.getByRole("button", { name: "關閉" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});

describe("折疊已完成", () => {
  const many = () => [done("a"), done("b"), done("c"), node({ id: "run", description: "跑中", started_ms: 5_000 })];

  it("3 棵以上完成的子樹折成一張「已完成 N 個」卡，執行中的留著", () => {
    renderPanel(many());
    expect(card("a")).toBeNull();
    expect(card("__done__").textContent).toContain("已完成 3 個");
    expect(card("run")).not.toBeNull();
  });

  it("點群組卡或工具列按鈕展開；再按「收起已完成」折回去", () => {
    renderPanel(many());
    fireEvent.click(card("__done__"));
    expect(card("a")).not.toBeNull();
    expect(card("__done__")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "收起已完成" }));
    expect(card("a")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "展開已完成 3 個" }));
    expect(card("a")).not.toBeNull();
  });

  it("不到門檻就不折，也沒有按鈕", () => {
    renderPanel([done("a"), done("b")]);
    expect(card("a")).not.toBeNull();
    expect(screen.queryByRole("button", { name: /已完成/ })).toBeNull();
  });
});

describe("依輪次分組", () => {
  const twoTurns = () => [done("old", { turn: 1, description: "上一輪的事" }), node({ id: "new", turn: 2, description: "這一輪的事" })];

  it("預設看最新一輪；有多輪時工具列出現輪次切換", () => {
    renderPanel(twoTurns());
    expect(card("new")).not.toBeNull();
    expect(card("old")).toBeNull();
    expect(screen.getByText("第 2/2 輪")).toBeInTheDocument();
  });

  it("上一輪／下一輪切換，邊界時按鈕停用", () => {
    renderPanel(twoTurns());
    expect(screen.getByRole("button", { name: "下一輪" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "上一輪" }));
    expect(card("old")).not.toBeNull();
    expect(card("new")).toBeNull();
    expect(screen.getByText("第 1/2 輪")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "上一輪" })).toBeDisabled();
  });

  it("只有一輪時不顯示輪次切換", () => {
    renderPanel([node({ id: "a", description: "x" })]);
    expect(screen.queryByRole("button", { name: "上一輪" })).toBeNull();
  });
});

describe("縮放與時間軸", () => {
  it("縮放按鈕改變比例；符合寬度在量不到寬度時不亂動", () => {
    renderPanel([node({ id: "a", description: "x" })]);
    expect(screen.getByTestId("zoom-value").textContent).toBe("100%");
    fireEvent.click(screen.getByRole("button", { name: "縮小比例" }));
    expect(screen.getByTestId("zoom-value").textContent).toBe("90%");
    expect((screen.getByTestId("graph") as HTMLElement).style.transform).toBe("scale(0.9)");
    fireEvent.click(screen.getByRole("button", { name: "放大比例" }));
    fireEvent.click(screen.getByRole("button", { name: "放大比例" }));
    expect(screen.getByTestId("zoom-value").textContent).toBe("110%");
    fireEvent.click(screen.getByRole("button", { name: "符合寬度" }));
    expect(screen.getByTestId("zoom-value").textContent).toBe("110%");
  });

  it("縮放有上下限", () => {
    renderPanel([node({ id: "a", description: "x" })]);
    for (let i = 0; i < 12; i++) fireEvent.click(screen.getByRole("button", { name: "縮小比例" }));
    expect(screen.getByTestId("zoom-value").textContent).toBe("50%");
    for (let i = 0; i < 20; i++) fireEvent.click(screen.getByRole("button", { name: "放大比例" }));
    expect(screen.getByTestId("zoom-value").textContent).toBe("150%");
  });

  it("切到時間軸：每個 subagent 一條橫條，點選後詳情照常顯示；選擇記在 localStorage", () => {
    renderPanel([done("a", { started_ms: 0, ended_ms: 4_000, summary: "做完了" }), done("b", { started_ms: 0, ended_ms: 2_000 })]);
    fireEvent.click(screen.getByRole("button", { name: "時間軸" }));
    expect(screen.queryByTestId("graph")).toBeNull();
    expect(screen.getByTestId("bar-a").style.width).toBe("100%");
    expect(screen.getByTestId("bar-b").style.width).toBe("50%");
    fireEvent.click(card("a"));
    expect(screen.getByText("做完了")).toBeInTheDocument();
    expect(localStorage.getItem("aiterm-claude-activity-mode")).toBe("timeline");
    fireEvent.click(screen.getByRole("button", { name: "流程圖" }));
    expect(screen.getByTestId("graph")).toBeInTheDocument();
  });

  it("標頭顯示本輪 token 合計；沒有任何 token 資料時不顯示", () => {
    const { unmount } = renderPanel([done("a", { tokens: 1_000 }), done("b", { tokens: 3_000 })]);
    expect(screen.getByText("共 4.0k tokens")).toBeInTheDocument();
    unmount();
    renderPanel([done("a")]);
    expect(screen.queryByText(/共 .* tokens/)).toBeNull();
  });

  it("token 最多的節點標「最耗」，詳情顯示佔本輪百分比", () => {
    renderPanel([done("a", { tokens: 1_000 }), done("b", { tokens: 3_000 })]);
    expect(card("b").textContent).toContain("最耗");
    expect(card("a").textContent).not.toContain("最耗");
    fireEvent.click(card("b"));
    expect(screen.getByText("佔本輪 75%")).toBeInTheDocument();
  });

  it("只有一個節點有 token 時不標最耗", () => {
    renderPanel([done("a", { tokens: 1_000 }), done("b")]);
    expect(card("a").textContent).not.toContain("最耗");
  });

  it("根卡片顯示主 Claude 的工具次數；點它看最近的工具清單", () => {
    renderPanel([done("a")], { main_tool_count: 12, main_recent: ["Read: /a.rs", "Bash: ls"] });
    expect(card("__root__").textContent).toContain("12 次工具呼叫");
    fireEvent.click(card("__root__"));
    expect(screen.getByText("主 Claude 本輪 12 次工具呼叫")).toBeInTheDocument();
    expect(screen.getByText("Read: /a.rs")).toBeInTheDocument();
    expect(screen.getByText("Bash: ls")).toBeInTheDocument();
    // 選根不該把其他卡片全部淡掉
    expect(card("a").className).not.toContain("is-dim");
  });

  it("沒有 subagent 時只剩根節點，仍可點開看主 Claude 最近的工具", () => {
    renderPanel([], { main_tool_count: 2, main_recent: ["Read: a", "Bash: ls"] });
    expect(card("__root__").textContent).toContain("2 次工具呼叫");
    fireEvent.click(card("__root__"));
    expect(screen.getByText("Bash: ls")).toBeInTheDocument();
  });

  const tool = (id: string, over: Partial<ToolCall> = {}): ToolCall => ({
    id, parent: null, tool: "Bash", hint: `Bash: ${id}`, category: "shell", status: "done",
    started_ms: 1_000, ended_ms: 1_500, turn: 1, ...over,
  });

  it("沒有 subagent 時，主 Claude 的每次工具呼叫是根底下的一個節點（含類別）", () => {
    renderPanel([], { tools: [tool("u1", { category: "web", hint: "WebFetch: x" }), tool("u2", { category: "database", hint: "psql" })] });
    expect(card("tool:u1").textContent).toContain("WebFetch: x");
    expect(card("tool:u1").className).toContain("cat-web");
    expect(card("tool:u2").className).toContain("cat-database");
    expect(screen.getByTestId("edge-__root__-tool:u1")).toBeInTheDocument();
  });

  it("subagent 的工具掛在那個 subagent 底下", () => {
    renderPanel([node({ id: "ag1", description: "查檔" })], { tools: [tool("u1", { parent: "ag1" })] });
    expect(screen.getByTestId("edge-ag1-tool:u1")).toBeInTheDocument();
  });

  it("失敗的工具用文字標示失敗；點工具節點詳情顯示類別與完整提示", () => {
    renderPanel([], { tools: [tool("u1", { status: "failed", category: "file", hint: "Read: /x/y" })] });
    expect(card("tool:u1").textContent).toContain("失敗");
    fireEvent.click(card("tool:u1"));
    const detail = document.querySelector(".aiterm-cact__detail") as HTMLElement;
    expect(detail.textContent).toContain("Read: /x/y");
    expect(detail.textContent).toContain("檔案");
  });

  it("工具太多時收成「更早 N 個工具」；點它展開全部", () => {
    const tools = Array.from({ length: 9 }, (_, i) => tool(`u${i}`, { started_ms: 1_000 + i }));
    renderPanel([], { tools });
    expect(card("tool:u0")).toBeNull();
    fireEvent.click(screen.getByText("更早 3 個工具"));
    expect(card("tool:u0")).not.toBeNull();
  });

  it("工具開關：關掉就只剩 subagent，並記在 localStorage", () => {
    const { unmount } = renderPanel([done("a")], { tools: [tool("u1")] });
    expect(card("tool:u1")).not.toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "隱藏工具" }));
    expect(card("tool:u1")).toBeNull();
    expect(card("a")).not.toBeNull();
    unmount();
    renderPanel([done("a")], { tools: [tool("u1")] });
    expect(card("tool:u1")).toBeNull();
    expect(screen.getByRole("button", { name: "顯示工具" })).toBeInTheDocument();
  });

  it("已折進「已完成 N 個」的 subagent，它的工具一併隱藏", () => {
    const agents = ["a", "b", "c"].map((id) => done(id));
    renderPanel(agents, { tools: [tool("u1", { parent: "a" }), tool("u2")] });
    expect(card("tool:u1")).toBeNull();
    expect(card("tool:u2")).not.toBeNull();
  });

  it("點工具節點：向後端取完整輸入與輸出並顯示；結束後（狀態變了）會重取", async () => {
    getToolDetail.mockResolvedValue({ input: "echo hi\nls", output: null });
    const running = tool("u1", { status: "running", ended_ms: null });
    const { rerender } = render(<ClaudeActivityPanel snapshot={snap([], { tools: [running] })} fill sessionId="sess-1" />);
    fireEvent.click(card("tool:u1"));
    expect(await screen.findByText(/echo hi/)).toBeInTheDocument();
    expect(getToolDetail).toHaveBeenCalledWith("sess-1", "u1");
    expect(screen.queryByText("輸出")).toBeNull();

    getToolDetail.mockResolvedValue({ input: "echo hi\nls", output: "hi\nfile" });
    rerender(<ClaudeActivityPanel snapshot={snap([], { tools: [tool("u1")] })} fill sessionId="sess-1" />);
    await waitFor(() => {
      const pres = [...document.querySelectorAll(".aiterm-cact__pre")].map((e) => e.textContent);
      expect(pres).toEqual(["echo hi\nls", "hi\nfile"]);
    });
    expect(getToolDetail).toHaveBeenCalledTimes(2);
  });

  it("取不到明細（例如離開 claude 後）時不顯示輸入輸出區塊，也不報錯", async () => {
    getToolDetail.mockResolvedValue(null);
    renderPanel([], { tools: [tool("u1")] });
    fireEvent.click(card("tool:u1"));
    await Promise.resolve();
    expect(screen.queryByText("輸入")).toBeNull();
  });

  it("中斷的工具用文字標示「中斷」", () => {
    renderPanel([], { tools: [tool("u1", { status: "interrupted" })] });
    expect(card("tool:u1").textContent).toContain("中斷");
  });

  it("最新一輪還沒有活動時提示，並停在最近有活動的那一輪", () => {
    renderPanel([], { tools: [tool("u1", { turn: 2 })], turn: 3 });
    expect(screen.getByText("最新一輪還沒有活動")).toBeInTheDocument();
    expect(card("tool:u1")).not.toBeNull();
  });

  it("最新一輪就有活動時不顯示那則提示", () => {
    renderPanel([], { tools: [tool("u1", { turn: 3 })], turn: 3 });
    expect(screen.queryByText("最新一輪還沒有活動")).toBeNull();
  });
});
