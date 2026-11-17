import { describe, expect, it } from "vitest";

import { translations } from "../../lib/i18n";
import { buildSpec, parseSpec, scheduleText } from "./jobSchedule";

const t = translations["zh-TW"];

describe("jobSchedule", () => {
  it("describes each kind in a sentence", () => {
    expect(scheduleText("daily", { times: ["09:00", "18:00"], days: [], every_minutes: 0 }, t)).toBe("每天 09:00、18:00");
    expect(scheduleText("weekly", { times: ["08:30"], days: [5, 1, 3], every_minutes: 0 }, t)).toBe("每週一、三、五 08:30");
    expect(scheduleText("interval", { times: [], days: [], every_minutes: 30 }, t)).toBe("每 30 分鐘");
  });

  it("builds only the fields that belong to the chosen kind", () => {
    expect(buildSpec("daily", "09:00, 18:00", [1], 30)).toEqual({ times: ["09:00", "18:00"], days: [], every_minutes: 0, window: null });
    expect(buildSpec("weekly", "09:00", [1, 3], 30)).toEqual({ times: ["09:00"], days: [1, 3], every_minutes: 0, window: null });
    expect(buildSpec("interval", "09:00", [1], 45)).toEqual({ times: [], days: [], every_minutes: 45, window: null });
    expect(buildSpec("daily", "09:00，18:00 、 07:15", [], 0).times).toEqual(["09:00", "18:00", "07:15"]);
  });

  it("survives a damaged spec_json", () => {
    expect(parseSpec({ spec_json: "not json" })).toEqual({ times: [], days: [], every_minutes: 0, window: null });
    expect(parseSpec({ spec_json: '{"times":["09:00"]}' }).times).toEqual(["09:00"]);
  });
});
