import { describe, expect, it } from "vitest";

import { translations } from "../../lib/i18n";
import { summarizeSchedule } from "./scheduleSummary";

const t = translations["zh-TW"];

describe("summarizeSchedule", () => {
  it("每天多個時間點", () => {
    expect(summarizeSchedule(t, "daily", { times: ["08:00", "18:30"] })).toBe("每天 08:00、18:30");
  });
  it("每週平日", () => {
    expect(summarizeSchedule(t, "weekly", { times: ["09:00"], days: [1, 2, 3, 4, 5] })).toBe("週一、週二、週三、週四、週五 09:00");
  });
  it("間隔：整小時用小時，其餘用分鐘", () => {
    expect(summarizeSchedule(t, "interval", { every_minutes: 30 })).toBe("每 30 分鐘");
    expect(summarizeSchedule(t, "interval", { every_minutes: 120 })).toBe("每 2 小時");
    expect(summarizeSchedule(t, "interval", { every_minutes: 90 })).toBe("每 90 分鐘");
  });
  it("間隔帶時段", () => {
    expect(summarizeSchedule(t, "interval", { every_minutes: 120, window: { from: "09:00", to: "18:00" } })).toBe("每 2 小時（09:00–18:00）");
  });
});
