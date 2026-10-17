import type { TaskRow, TokenUsage } from "../../ipc/tasks";

/** 一組卡片加總後的用量：`cards` 是有記錄的張數，`missing` 是跑過卻沒有記錄的張數。 */
export interface UsageSum {
  usage: TokenUsage;
  cards: number;
  missing: number;
}

export const totalTokens = (u: TokenUsage): number => u.input + u.cache_write + u.cache_read + u.output;

/** 1234 → 1.2K、1234567 → 1.23M。 */
export function formatTokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}K`;
  return `${(n / 1_000_000).toFixed(2)}M`;
}

/** 加總這些卡片的用量；還沒派出去的卡片不算。一張都沒跑過就是 `null`。 */
export function sumUsage(cards: Pick<TaskRow, "id" | "dispatched_at">[], usage: Record<string, TokenUsage>): UsageSum | null {
  const ran = cards.filter((c) => c.dispatched_at != null);
  if (!ran.length) return null;
  const sum: UsageSum = { usage: { input: 0, cache_write: 0, cache_read: 0, output: 0, calls: 0 }, cards: 0, missing: 0 };
  for (const c of ran) {
    const u = usage[c.id];
    if (!u) {
      sum.missing += 1;
      continue;
    }
    sum.cards += 1;
    sum.usage.input += u.input;
    sum.usage.cache_write += u.cache_write;
    sum.usage.cache_read += u.cache_read;
    sum.usage.output += u.output;
    sum.usage.calls += u.calls;
  }
  return sum;
}

/** 一個模組的全部成本：工作卡本身＋它的審查、修正卡。 */
export const moduleCards = <T extends Pick<TaskRow, "id" | "module_id">>(moduleId: string, tasks: T[]): T[] =>
  tasks.filter((c) => c.id === moduleId || c.module_id === moduleId);

/** 給 tooltip 的分項。 */
export const usageBreakdown = (s: UsageSum) => ({
  input: formatTokens(s.usage.input),
  cacheWrite: formatTokens(s.usage.cache_write),
  cacheRead: formatTokens(s.usage.cache_read),
  output: formatTokens(s.usage.output),
  calls: s.usage.calls,
  cards: s.cards,
  missing: s.missing,
});
