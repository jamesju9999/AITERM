import type { ActivityNode, ToolCall } from "../../ipc/claudeActivity";

export const TOOL_PREFIX = "tool:";
export const TOOLS_MORE_PREFIX = "tools-more:";
/** 每個呼叫者最多顯示最近幾個已完成的工具；更早的收成一張群組卡。 */
export const TOOL_WINDOW = 6;

export const CATEGORY_ICON: Record<string, string> = {
  file: "📄", shell: "⌨️", web: "🌐", database: "🗄️", mcp: "🔌", other: "⚙️",
};

export const isToolId = (id: string): boolean => id.startsWith(TOOL_PREFIX);
export const isToolsMoreId = (id: string): boolean => id.startsWith(TOOLS_MORE_PREFIX);

interface Opts {
  /** true＝不收合，全部顯示。 */
  all: boolean;
  moreLabel: (hidden: number) => string;
}

const base = (over: Partial<ActivityNode> & Pick<ActivityNode, "id" | "parent" | "started_ms" | "turn">): ActivityNode => ({
  agent_type: "", description: null, status: "done", ended_ms: null, current_tool: null, summary: null,
  tokens: null, tool_count: null, last_event_ms: over.started_ms, ...over,
});

/**
 * 把工具呼叫轉成可以餵給 `layoutGraph` 的偽節點。每個呼叫者（主 Claude 或某個 subagent）
 * 各自只留最近 `TOOL_WINDOW` 個已完成的；失敗、執行中、中斷的永遠留著（那些才需要看）。
 */
export function toolsToNodes(tools: ToolCall[], turn: number, opts: Opts): ActivityNode[] {
  const byParent = new Map<string | null, ToolCall[]>();
  for (const c of tools.filter((c) => c.turn === turn)) {
    const list = byParent.get(c.parent) ?? [];
    list.push(c);
    byParent.set(c.parent, list);
  }
  const out: ActivityNode[] = [];
  for (const [parent, list] of byParent) {
    list.sort((a, b) => a.started_ms - b.started_ms);
    const recent = new Set(list.slice(-TOOL_WINDOW).map((c) => c.id));
    const hidden = opts.all ? [] : list.filter((c) => c.status === "done" && !recent.has(c.id));
    const hiddenIds = new Set(hidden.map((c) => c.id));
    if (hidden.length > 0) {
      out.push(base({
        id: `${TOOLS_MORE_PREFIX}${parent ?? "root"}`, parent, started_ms: hidden[0].started_ms, turn,
        description: opts.moreLabel(hidden.length),
      }));
    }
    for (const c of list) {
      if (hiddenIds.has(c.id)) continue;
      out.push(base({
        id: `${TOOL_PREFIX}${c.id}`, parent: c.parent, agent_type: c.category, description: c.hint,
        status: c.status, started_ms: c.started_ms, ended_ms: c.ended_ms, turn: c.turn,
      }));
    }
  }
  return out;
}

/** 把工具節點接到畫面上的 subagent 後面；呼叫者已被折疊（不在畫面上）的工具一併隱藏。 */
export function attachTools(agents: ActivityNode[], toolNodes: ActivityNode[]): ActivityNode[] {
  const ids = new Set(agents.map((n) => n.id));
  return [...agents, ...toolNodes.filter((n) => n.parent === null || ids.has(n.parent))];
}
