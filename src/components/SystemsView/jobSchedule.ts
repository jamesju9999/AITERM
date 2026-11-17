import type { Translations } from "../../lib/i18n";
import type { JobKind, JobSpec, SystemJob } from "../../ipc/systems";

export function parseSpec(job: Pick<SystemJob, "spec_json">): JobSpec {
  try {
    const s = JSON.parse(job.spec_json) as Partial<JobSpec>;
    return { times: s.times ?? [], days: s.days ?? [], every_minutes: s.every_minutes ?? 0, window: s.window ?? null };
  } catch {
    return { times: [], days: [], every_minutes: 0, window: null };
  }
}

/** 一句話描述週期，給列表顯示。 */
export function scheduleText(kind: JobKind, spec: JobSpec, t: Translations): string {
  const times = spec.times.join("、");
  if (kind === "daily") return t.sys_job_schedule_daily(times);
  if (kind === "weekly") return t.sys_job_schedule_weekly([...spec.days].sort().map((d) => t.sys_job_weekday(d)).join("、"), times);
  return t.sys_job_schedule_interval(spec.every_minutes);
}

/** 表單輸入 → 週期；時間字串以逗號分隔。 */
export function buildSpec(kind: JobKind, timesText: string, days: number[], every: number): JobSpec {
  const times = timesText.split(/[,，、\s]+/).map((x) => x.trim()).filter(Boolean);
  return { times: kind === "interval" ? [] : times, days: kind === "weekly" ? days : [], every_minutes: kind === "interval" ? every : 0, window: null };
}
