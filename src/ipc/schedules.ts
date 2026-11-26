import { invoke } from "@tauri-apps/api/core";

export type ScheduleKind = "daily" | "weekly" | "interval";

export interface ScheduleSpec {
  times?: string[];
  /** 1=週一 … 7=週日 */
  days?: number[];
  every_minutes?: number;
  window?: { from: string; to: string } | null;
  /** IANA 時區（例如 Asia/Taipei）；只有共用看板（由 server 觸發）用到，沒填＝執行那台機器的本地時間。 */
  tz?: string | null;
}

export interface ScheduleRun {
  schedule_id: string;
  fired_at: number;
  module_id: string | null;
  result: "success" | "failed" | "skipped";
}

/** 鏡射 Rust `ScheduleView`（ScheduleRow 攤平 ＋ runs）。 */
export interface ScheduleView {
  id: string;
  name: string;
  agent_id: string;
  instruction: string;
  schedule_kind: ScheduleKind;
  schedule_json: string;
  enabled: boolean;
  failures_count_as_failed: boolean;
  canvas_x: number | null;
  canvas_y: number | null;
  last_fired_at: number;
  last_module_id: string | null;
  fail_streak: number;
  skip_streak: number;
  disabled_reason: string | null;
  runs: ScheduleRun[];
}

export interface NewSchedule {
  name: string;
  agent_id: string;
  instruction: string;
  schedule_kind: ScheduleKind;
  spec: ScheduleSpec;
  failures_count_as_failed: boolean;
  canvas_x?: number | null;
  canvas_y?: number | null;
}

export const listSchedules = (projectId: string): Promise<ScheduleView[]> => invoke("schedules_list", { projectId });

// 頂層參數 Tauri 會把 camelCase 轉成 snake_case；`args` 裡的欄位原樣傳，所以維持 snake_case。
export const saveSchedule = (projectId: string, id: string | null, args: NewSchedule): Promise<string> =>
  invoke("schedules_save", { projectId, id, args });

export const deleteSchedule = (projectId: string, id: string): Promise<void> => invoke("schedules_delete", { projectId, id });

export const setScheduleEnabled = (projectId: string, id: string, enabled: boolean): Promise<void> =>
  invoke("schedules_set_enabled", { projectId, id, enabled });

/** 回傳新卡 id；上一輪還沒結案被擋下時是空字串。 */
export const runScheduleNow = (projectId: string, id: string): Promise<string> => invoke("schedules_run_now", { projectId, id });

export function parseSpec(json: string): ScheduleSpec {
  try {
    return JSON.parse(json) as ScheduleSpec;
  } catch {
    return {};
  }
}
