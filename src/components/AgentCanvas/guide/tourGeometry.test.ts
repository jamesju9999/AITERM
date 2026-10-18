import { describe, expect, it } from "vitest";

import { clampSpot } from "./tourGeometry";

describe("clampSpot", () => {
  it("目標在畫面中間：每邊大 4px", () => {
    expect(clampSpot({ left: 100, top: 100, right: 200, bottom: 150 }, 1000, 800)).toEqual({ left: 96, top: 96, width: 108, height: 58 });
  });

  it("目標貼著視窗邊（角色列延伸到右緣）：框夾在視窗內，四邊都看得到", () => {
    // 實機：角色列從 120 延伸到視窗右緣 1000，框的右邊原本畫在 1004，被切掉。
    expect(clampSpot({ left: 120, top: 0, right: 1000, bottom: 60 }, 1000, 800)).toEqual({ left: 116, top: 8, width: 876, height: 56 });
    expect(clampSpot({ left: 0, top: 700, right: 300, bottom: 800 }, 1000, 800)).toEqual({ left: 8, top: 696, width: 296, height: 96 });
  });
});
