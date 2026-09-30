/**
 * 已完成卡片的「共花費時間」＝ 派工到完成（`finished_at - dispatched_at`，
 * 兩者都是 Unix 秒）。不是「建立到完成」：卡片可能在計畫中放了好幾天才派工，
 * 那段時間不是工作花掉的。
 */

/** 只有兩個時間都有、且完成不早於派工時才算得出來；否則回 `null`。 */
export function taskDurationSecs(card: {
  status: string;
  dispatched_at: number | null;
  finished_at: number | null;
}): number | null {
  if (card.status !== "done") return null;
  const { dispatched_at: start, finished_at: end } = card;
  if (start === null || end === null || end < start) return null;
  return end - start;
}

/**
 * 精簡、與語系無關的寫法：`45s`、`4m 12s`、`1h 23m`、`2d 3h`。
 * 只顯示最大的兩個單位——超過一小時還顯示秒只是雜訊。
 */
export function formatDuration(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (s < 3600) return `${m}m ${s % 60}s`;
  const h = Math.floor(s / 3600);
  if (s < 86400) return `${h}h ${m % 60}m`;
  return `${Math.floor(s / 86400)}d ${h % 24}h`;
}
