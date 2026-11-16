import type { ScheduleKind, ScheduleSpec } from "../../ipc/schedules";
import type { Translations } from "../../lib/i18n";

type T = Pick<Translations, "sched_weekdays" | "sched_sum_daily" | "sched_sum_every_hours" | "sched_sum_every_minutes">;

/** 節點上的一行週期摘要，例如「每天 08:00」「週一、週二 09:00」「每 2 小時（09:00–18:00）」。 */
export function summarizeSchedule(t: T, kind: ScheduleKind, spec: ScheduleSpec): string {
  const times = (spec.times ?? []).join("、");
  if (kind === "daily") return t.sched_sum_daily(times);
  if (kind === "weekly") {
    const days = (spec.days ?? []).map((d) => t.sched_weekdays[d - 1] ?? String(d)).join("、");
    return `${days} ${times}`.trim();
  }
  const m = spec.every_minutes ?? 0;
  const every = m > 0 && m % 60 === 0 ? t.sched_sum_every_hours(m / 60) : t.sched_sum_every_minutes(m);
  return spec.window ? `${every}（${spec.window.from}–${spec.window.to}）` : every;
}
