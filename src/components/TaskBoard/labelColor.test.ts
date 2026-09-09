import { describe, expect, it } from "vitest";
import { hashLabelHue } from "./labelColor";

describe("hashLabelHue", () => {
  it("回傳 0-359 之間的整數", () => {
    const hue = hashLabelHue("緊急");
    expect(Number.isInteger(hue)).toBe(true);
    expect(hue).toBeGreaterThanOrEqual(0);
    expect(hue).toBeLessThan(360);
  });

  it("同一個字串永遠得到同一個色相", () => {
    expect(hashLabelHue("文件")).toBe(hashLabelHue("文件"));
  });

  it("不同字串通常得到不同色相", () => {
    expect(hashLabelHue("緊急")).not.toBe(hashLabelHue("文件"));
  });

  it("空字串也有一個穩定的結果，不會噴錯", () => {
    expect(() => hashLabelHue("")).not.toThrow();
    expect(hashLabelHue("")).toBe(hashLabelHue(""));
  });
});
