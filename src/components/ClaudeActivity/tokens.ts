import type { ActivityNode } from "../../ipc/claudeActivity";

export interface TokenStats {
  total: number;
  /** token 最多的節點；有 token 的節點少於 2 個時為 null（沒有可比較的對象）。 */
  heaviestId: string | null;
  /** 該節點佔合計的整數百分比；沒有 token 或合計為 0 時為 null。 */
  share: (node: ActivityNode) => number | null;
}

export function tokenStats(nodes: ActivityNode[]): TokenStats {
  const withTokens = nodes.filter((n) => n.tokens != null);
  const total = withTokens.reduce((sum, n) => sum + (n.tokens ?? 0), 0);
  let heaviest: ActivityNode | null = null;
  for (const n of withTokens) if (!heaviest || (n.tokens ?? 0) > (heaviest.tokens ?? 0)) heaviest = n;
  return {
    total,
    heaviestId: withTokens.length >= 2 && heaviest && total > 0 ? heaviest.id : null,
    share: (n) => (n.tokens == null || total === 0 ? null : Math.round((n.tokens / total) * 100)),
  };
}
