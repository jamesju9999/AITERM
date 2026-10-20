import type { ActivityNode } from "../../ipc/claudeActivity";

export interface ActivityRow {
  node: ActivityNode;
  depth: number;
}

/** 依開始順序做深度優先展開；孤兒（父節點不在清單內）與循環中的節點當根，保證每個節點恰好出現一次。 */
export function flattenTree(nodes: ActivityNode[]): ActivityRow[] {
  const ids = new Set(nodes.map((n) => n.id));
  const children = new Map<string, ActivityNode[]>();
  const roots: ActivityNode[] = [];
  for (const n of nodes) {
    if (n.parent && ids.has(n.parent) && n.parent !== n.id) {
      const list = children.get(n.parent) ?? [];
      list.push(n);
      children.set(n.parent, list);
    } else {
      roots.push(n);
    }
  }
  const rows: ActivityRow[] = [];
  const seen = new Set<string>();
  const visit = (n: ActivityNode, depth: number) => {
    if (seen.has(n.id)) return;
    seen.add(n.id);
    rows.push({ node: n, depth });
    for (const c of children.get(n.id) ?? []) visit(c, depth + 1);
  };
  for (const r of roots) visit(r, 0);
  // 循環中的節點沒有任何根可達：補上，避免消失。
  for (const n of nodes) visit(n, 0);
  return rows;
}
