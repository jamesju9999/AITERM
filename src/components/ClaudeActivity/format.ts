/** 4.9s／59s／1m 5s／1h 2m。 */
export function formatDuration(ms: number): string {
  const total = Math.max(0, ms);
  if (total < 10_000) {
    const tenths = Math.floor(total / 100);
    return tenths % 10 === 0 ? `${tenths / 10}s` : `${(tenths / 10).toFixed(1)}s`;
  }
  const s = Math.floor(total / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${s % 60}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

/** 999／1.0k／20.5k／1.3M。 */
export function formatTokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}
