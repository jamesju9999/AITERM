import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { invoke } from "@tauri-apps/api/core";
import { deleteSchedule, listSchedules, parseSpec, runScheduleNow, saveSchedule, setScheduleEnabled } from "./schedules";

beforeEach(() => vi.mocked(invoke).mockReset());

describe("ipc/schedules", () => {
  it("listSchedules", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    await listSchedules("p1");
    expect(invoke).toHaveBeenCalledWith("schedules_list", { projectId: "p1" });
  });

  it("saveSchedule 沒有 id 就是新增，args 欄位保持 snake_case", async () => {
    vi.mocked(invoke).mockResolvedValue("s1");
    const args = {
      name: "n",
      agent_id: "a",
      instruction: "i",
      schedule_kind: "daily" as const,
      spec: { times: ["08:00"] },
      failures_count_as_failed: false,
    };
    await saveSchedule("p1", null, args);
    expect(invoke).toHaveBeenCalledWith("schedules_save", { projectId: "p1", id: null, args });
  });

  it("deleteSchedule／setScheduleEnabled／runScheduleNow 帶對參數", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);
    await deleteSchedule("p1", "s1");
    expect(invoke).toHaveBeenLastCalledWith("schedules_delete", { projectId: "p1", id: "s1" });
    await setScheduleEnabled("p1", "s1", true);
    expect(invoke).toHaveBeenLastCalledWith("schedules_set_enabled", { projectId: "p1", id: "s1", enabled: true });
    await runScheduleNow("p1", "s1");
    expect(invoke).toHaveBeenLastCalledWith("schedules_run_now", { projectId: "p1", id: "s1" });
  });

  it("parseSpec 遇到壞 JSON 回空物件", () => {
    expect(parseSpec("{bad")).toEqual({});
    expect(parseSpec('{"times":["08:00"]}')).toEqual({ times: ["08:00"] });
  });
});
