import { describe, expect, it } from "vitest";

import type { ScheduleView } from "../../ipc/schedules";
import { scheduleStatus } from "./scheduleStatus";

const base = { enabled: true, disabled_reason: null, skip_streak: 0 } as ScheduleView;

describe("scheduleStatus", () => {
  it("啟用中", () => expect(scheduleStatus(base)).toBe("enabled"));
  it("手動停用", () => expect(scheduleStatus({ ...base, enabled: false })).toBe("disabled"));
  it("連續失敗停用", () => expect(scheduleStatus({ ...base, enabled: false, disabled_reason: "連續失敗" })).toBe("failed"));
  it("角色消失", () => expect(scheduleStatus({ ...base, enabled: false, disabled_reason: "角色已不存在" })).toBe("agent_gone"));
  it("角色因卡片失敗被暫停", () => expect(scheduleStatus({ ...base, enabled: false, disabled_reason: "角色已暫停" })).toBe("agent_paused"));
  it("連續跳過 3 次才算卡住，且只對啟用中的", () => {
    expect(scheduleStatus({ ...base, skip_streak: 3 })).toBe("stuck");
    expect(scheduleStatus({ ...base, skip_streak: 2 })).toBe("enabled");
    expect(scheduleStatus({ ...base, enabled: false, skip_streak: 5 })).toBe("disabled");
  });
});
