import { useLocale } from "../../contexts/LocaleContext";
import { parseSpec, type ScheduleView } from "../../ipc/schedules";
import { scheduleStatus } from "./scheduleStatus";
import { summarizeSchedule } from "./scheduleSummary";

export const SCHEDULE_NODE_W = 200;

/** 畫布上的定時器節點：名稱、週期摘要、狀態與最近一輪結果。點下去開設定。 */
export function ScheduleNode({ schedule, x, y, onEdit }: { schedule: ScheduleView; x: number; y: number; onEdit: () => void }) {
  const { t } = useLocale();
  const status = scheduleStatus(schedule);
  const last = schedule.runs[0];
  const label =
    status === "failed"
      ? t.sched_disabled_failures
      : status === "agent_gone"
        ? t.sched_disabled_agent_gone
        : status === "agent_paused"
          ? t.sched_disabled_agent_paused
          : status === "stuck"
          ? t.sched_stuck
          : status === "disabled"
            ? t.sched_status_disabled
            : t.sched_status_enabled;
  return (
    <div
      className={`ac-sched-node ac-sched-node--${status}`}
      data-testid="schedule-node"
      data-schedule-id={schedule.id}
      style={{ left: x, top: y, width: SCHEDULE_NODE_W }}
      role="button"
      tabIndex={0}
      onClick={onEdit}
      onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && onEdit()}
    >
      <div className="ac-sched-node-title">⏰ {schedule.name}</div>
      <div className="ac-sched-node-sum">{summarizeSchedule(t, schedule.schedule_kind, parseSpec(schedule.schedule_json))}</div>
      <div className="ac-sched-node-status">{label}</div>
      {last && (
        <div className={`ac-sched-node-last ac-sched-node-last--${last.result}`}>
          {last.result === "success" ? t.sched_run_success : last.result === "failed" ? t.sched_run_failed : t.sched_run_skipped}
        </div>
      )}
    </div>
  );
}
