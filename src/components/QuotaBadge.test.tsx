import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { QuotaBadge } from "./QuotaBadge";
import type { QuotaWindow } from "../ipc/usage";

const win = (over: Partial<QuotaWindow> = {}): QuotaWindow => ({
  label: "5h", used_percent: 7, resets_at: null,
  severity: "normal", detail: null, is_primary: true, ...over,
});

describe("QuotaBadge", () => {
  it("normal 時仍然顯示（A 案的核心，不是只有超標才出現）", () => {
    render(<QuotaBadge window={win({ severity: "normal", used_percent: 7 })} />);
    expect(screen.getByTestId("quota-badge")).toHaveTextContent("5h 7%");
  });

  it("依 severity 套用對應的 class", () => {
    const { rerender } = render(<QuotaBadge window={win({ severity: "normal" })} />);
    expect(screen.getByTestId("quota-badge").className).toContain("normal");
    rerender(<QuotaBadge window={win({ severity: "warning" })} />);
    expect(screen.getByTestId("quota-badge").className).toContain("warning");
    rerender(<QuotaBadge window={win({ severity: "critical" })} />);
    expect(screen.getByTestId("quota-badge").className).toContain("critical");
  });

  it("有 detail 時優先顯示原始語意", () => {
    render(<QuotaBadge window={win({ label: "premium", detail: "142 / 300" })} />);
    expect(screen.getByTestId("quota-badge")).toHaveTextContent("premium 142 / 300");
  });

  it("百分比四捨五入到整數", () => {
    render(<QuotaBadge window={win({ used_percent: 52.5 })} />);
    expect(screen.getByTestId("quota-badge")).toHaveTextContent("53%");
  });
});
