import { describe, expect, it } from "vitest";

import { buildDoneMessage, buildDoneSystem, MAX_CRITERIA, parseDoneCriteria, withDoneCriteria } from "./doneCriteria";

describe("parseDoneCriteria", () => {
  it("取出 JSON 裡的條目，容忍程式碼區塊與前後多餘的字", () => {
    const reply = '好的：\n```json\n{"criteria":["按開始會倒數 25 分鐘","重新整理後紀錄還在"]}\n```';
    expect(parseDoneCriteria(reply)).toEqual(["按開始會倒數 25 分鐘", "重新整理後紀錄還在"]);
  });

  it("去掉空白、重複與非字串，最多七條", () => {
    const many = Array.from({ length: 10 }, (_, i) => `條件 ${i}`);
    const reply = JSON.stringify({ criteria: ["  ", "a", "a", 3, null, ...many] });
    const out = parseDoneCriteria(reply);
    expect(out[0]).toBe("a");
    expect(out).toHaveLength(MAX_CRITERIA);
    expect(new Set(out).size).toBe(out.length);
  });

  it("壞掉的回覆回空陣列，不丟例外", () => {
    expect(parseDoneCriteria("不是 JSON")).toEqual([]);
    expect(parseDoneCriteria('{"criteria":"字串"}')).toEqual([]);
    expect(parseDoneCriteria('{"criteria":[]}')).toEqual([]);
    expect(parseDoneCriteria("{壞掉")).toEqual([]);
  });
});

describe("withDoneCriteria", () => {
  it("附在內文後面，並要求規格與驗收涵蓋", () => {
    const body = withDoneCriteria("做計時器\n", ["按開始會倒數", "結束會響鈴"]);
    expect(body).toBe(
      "做計時器\n\n## 完成定義\n- 按開始會倒數\n- 結束會響鈴\n（規格的驗收條件要涵蓋以上每一條；驗收時逐條實際驗證並附上證據。）",
    );
  });

  it("沒有條目就原樣回傳", () => {
    expect(withDoneCriteria("做計時器", [])).toBe("做計時器");
  });
});

describe("prompts", () => {
  it("系統提示要求可驗證，並限制條數", () => {
    const s = buildDoneSystem();
    expect(s).toContain("能實際驗證");
    expect(s).toContain(`${MAX_CRITERIA} 條以內`);
  });

  it("簡介與需求相同時不重複，不同時帶上簡介", () => {
    expect(buildDoneMessage("做計時器", "做計時器")).not.toContain("專案簡介");
    expect(buildDoneMessage("加暫停鍵", "番茄鐘 App")).toContain("專案簡介：\n番茄鐘 App");
  });
});
