import type { ActivityNode } from "../../ipc/claudeActivity";

export const DONE_GROUP_ID = "__done__";
/** 至少幾棵「整棵都已完成」的子樹才折疊；只有一兩棵時折起來反而多一個動作。 */
export const FOLD_THRESHOLD = 3;

export interface Folded {
  /** 折疊後要畫的節點（含代表已折疊節點的群組卡）。沒達門檻時就是原本的節點。 */
  nodes: ActivityNode[];
  /** 被折起來的節點數（0＝沒折）。 */
  folded: number;
}

/**
 * 把「從根往下整棵都已完成」的子樹折成一張群組卡。
 * 只折 `done`：失敗、中斷與執行中的永遠留著，因為那些才是需要看的。
 */
export function foldFinished(nodes: ActivityNode[], label: (count: number) => string): Folded {
  const ids = new Set(nodes.map((n) => n.id));
  const kids = new Map<string, ActivityNode[]>();
  const roots: ActivityNode[] = [];
  for (const n of nodes) {
    if (n.parent && ids.has(n.parent) && n.parent !== n.id) {
      const list = kids.get(n.parent) ?? [];
      list.push(n);
      kids.set(n.parent, list);
    } else {
      roots.push(n);
    }
  }
  const subtree = (root: ActivityNode): ActivityNode[] => {
    const out: ActivityNode[] = [];
    const seen = new Set<string>();
    const walk = (n: ActivityNode) => {
      if (seen.has(n.id)) return;
      seen.add(n.id);
      out.push(n);
      (kids.get(n.id) ?? []).forEach(walk);
    };
    walk(root);
    return out;
  };
  const foldable = roots.map(subtree).filter((tree) => tree.every((n) => n.status === "done"));
  if (foldable.length < FOLD_THRESHOLD) return { nodes, folded: 0 };

  const hidden = new Set(foldable.flat().map((n) => n.id));
  const members = foldable.flat();
  const group: ActivityNode = {
    id: DONE_GROUP_ID,
    parent: null,
    agent_type: "",
    description: label(members.length),
    status: "done",
    started_ms: Math.min(...members.map((n) => n.started_ms)),
    ended_ms: Math.max(...members.map((n) => n.ended_ms ?? n.started_ms)),
    current_tool: null,
    summary: null,
    tokens: null,
    tool_count: null, last_event_ms: 0,
    turn: members[0].turn,
  };
  return { nodes: [group, ...nodes.filter((n) => !hidden.has(n.id))], folded: members.length };
}
