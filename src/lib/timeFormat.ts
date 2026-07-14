export function formatTime(ms: number): string {
  return new Date(ms).toLocaleTimeString("zh-TW", { hour12: false });
}

export function formatDuration(startMs: number, endMs: number): string {
  const s = Math.max(0, Math.round((endMs - startMs) / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  const rem = s % 60;
  return rem === 0 ? `${m}分` : `${m}分${rem}s`;
}
