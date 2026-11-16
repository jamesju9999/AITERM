import type { ScheduleView } from "../../ipc/schedules";

export type ScheduleStatus = "enabled" | "disabled" | "failed" | "agent_gone" | "agent_paused" | "stuck";

/** `disabled_reason` 的值要跟後端 `tasks/schedules.rs` 寫入的字串一致。 */
export const REASON_FAILURES = "連續失敗";
export const REASON_AGENT_GONE = "角色已不存在";
export const REASON_AGENT_PAUSED = "角色已暫停";
/** 連續跳過幾次就提示「上一輪卡住了」。 */
export const STUCK_SKIPS = 3;

export function scheduleStatus(s: Pick<ScheduleView, "enabled" | "disabled_reason" | "skip_streak">): ScheduleStatus {
  if (!s.enabled) {
    if (s.disabled_reason === REASON_FAILURES) return "failed";
    if (s.disabled_reason === REASON_AGENT_GONE) return "agent_gone";
    if (s.disabled_reason === REASON_AGENT_PAUSED) return "agent_paused";
    return "disabled";
  }
  return s.skip_streak >= STUCK_SKIPS ? "stuck" : "enabled";
}
