import { describe, expect, it } from "vitest";
import { CHART_PALETTE_LIGHT, CHART_PALETTE_DARK } from "./chartPalette";

const HEX = /^#[0-9a-f]{6}$/i;

describe("chartPalette", () => {
  it("light and dark categorical palettes each have exactly 8 distinct hex colors", () => {
    for (const palette of [CHART_PALETTE_LIGHT, CHART_PALETTE_DARK]) {
      expect(palette.categorical).toHaveLength(8);
      for (const hex of palette.categorical) expect(hex).toMatch(HEX);
      expect(new Set(palette.categorical).size).toBe(8);
    }
  });

  it("sequential ramp is 7 distinct hex steps, same across light and dark", () => {
    expect(CHART_PALETTE_LIGHT.sequential).toHaveLength(7);
    expect(CHART_PALETTE_LIGHT.sequential).toEqual(CHART_PALETTE_DARK.sequential);
    expect(new Set(CHART_PALETTE_LIGHT.sequential).size).toBe(7);
  });

  it("status colors are identical in light and dark (fixed, never themed)", () => {
    expect(CHART_PALETTE_LIGHT.status).toEqual(CHART_PALETTE_DARK.status);
  });

  it("light and dark use different surface/text tokens", () => {
    expect(CHART_PALETTE_LIGHT.surface).not.toBe(CHART_PALETTE_DARK.surface);
    expect(CHART_PALETTE_LIGHT.textPrimary).not.toBe(CHART_PALETTE_DARK.textPrimary);
  });
});
