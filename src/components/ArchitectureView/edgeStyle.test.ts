import { describe, expect, it } from "vitest";
// Vitest 會把 .css 的 ?raw／?inline 匯入換成空字串，所以只好讀檔；tsconfig.app 的 types 沒有 node，故忽略型別。
// @ts-expect-error node:fs 沒有型別宣告
import { readFileSync } from "node:fs";

const css: string = readFileSync("src/components/ArchitectureView/index.css", "utf8");

const rule = (sel: string) => {
  const m = new RegExp(`(?:^|\\n)${sel.replace(/[.]/g, "\\.")}\\s*\\{([^}]*)\\}`).exec(css);
  expect(m, `找不到 ${sel}`).not.toBeNull();
  return m![1];
};
const num = (body: string, prop: string) => parseFloat(new RegExp(`${prop}:\\s*([\\d.]+)`).exec(body)![1]);

describe("連線樣式", () => {
  it("稠密圖安靜連線約 0.35 不透明、細線、沒有光暈濾鏡", () => {
    const q = rule(".arch-edge-quiet");
    expect(num(q, "opacity")).toBeCloseTo(0.35);
    expect(num(q, "stroke-width")).toBeLessThanOrEqual(1);
    expect(q).not.toMatch(/filter/);
  });
  it("強調連線仍明顯比安靜連線突出（更粗、完全不透明）", () => {
    const q = rule(".arch-edge-quiet");
    const on = rule(".arch-edge-on");
    expect(num(on, "opacity")).toBe(1);
    expect(num(on, "stroke-width")).toBeGreaterThanOrEqual(num(q, "stroke-width") * 3);
    expect(num(rule(".arch-edge-dim"), "opacity")).toBeLessThan(num(q, "opacity"));
  });
});
