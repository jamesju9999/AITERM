import { describe, it, expect } from "vitest";
import { formatDuration, taskDurationSecs } from "./duration";

describe("taskDurationSecs", () => {
  const done = { status: "done", dispatched_at: 1000, finished_at: 1450 };

  it("已完成且兩個時間都有：完成減派工（秒）", () => {
    expect(taskDurationSecs(done)).toBe(450);
  });

  it("還沒完成的卡片不算", () => {
    expect(taskDurationSecs({ ...done, status: "running" })).toBeNull();
  });

  it("缺任一個時間就不算（舊資料或沒經過派工直接標完成）", () => {
    expect(taskDurationSecs({ ...done, dispatched_at: null })).toBeNull();
    expect(taskDurationSecs({ ...done, finished_at: null })).toBeNull();
  });

  it("完成早於派工（時鐘被調過）不顯示負數", () => {
    expect(taskDurationSecs({ ...done, finished_at: 900 })).toBeNull();
  });

  it("剛好 0 秒是合法的", () => {
    expect(taskDurationSecs({ ...done, finished_at: 1000 })).toBe(0);
  });
});

describe("formatDuration", () => {
  it.each([
    [0, "0s"],
    [45, "45s"],
    [59, "59s"],
    [60, "1m 0s"],
    [252, "4m 12s"],
    [3599, "59m 59s"],
    [3600, "1h 0m"],
    [4980, "1h 23m"],
    [86399, "23h 59m"],
    [86400, "1d 0h"],
    [183600, "2d 3h"],
  ])("%i 秒 → %s", (secs, out) => {
    expect(formatDuration(secs)).toBe(out);
  });

  it("小數秒無條件捨去、負數當 0", () => {
    expect(formatDuration(59.9)).toBe("59s");
    expect(formatDuration(-5)).toBe("0s");
  });
});
