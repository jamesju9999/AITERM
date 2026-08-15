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

  it("有 detail 時只顯示 detail，標籤讓給 tooltip", () => {
    // 徽章擠在標題列裡，寬度是稀缺資源。「premium 142/300」會把 provider
    // 名稱擠成兩行並被裁掉（實機發生過），所以標籤不進徽章本文。
    render(<QuotaBadge window={win({ label: "premium", detail: "142/300" })} />);
    const badge = screen.getByTestId("quota-badge");
    expect(badge).toHaveTextContent("142/300");
    expect(badge.textContent).not.toContain("premium");
    // 但脈絡不能消失——它要在 tooltip 裡。
    expect(badge.title).toContain("premium");
  });

  it("tooltip 帶出百分比與重置時間", () => {
    const inTwoHours = Math.floor(Date.now() / 1000) + 2 * 3600;
    render(<QuotaBadge window={win({ label: "5h", used_percent: 7, resets_at: inTwoHours })} />);
    const badge = screen.getByTestId("quota-badge");
    expect(badge.title).toContain("已用 7%");
    expect(badge.title).toContain("2 小時後重置");
  });

  it("重置時間已過或未提供時 tooltip 不提它", () => {
    render(<QuotaBadge window={win({ resets_at: null })} />);
    expect(screen.getByTestId("quota-badge").title).not.toContain("重置");
  });

  it("百分比四捨五入到整數", () => {
    render(<QuotaBadge window={win({ used_percent: 52.5 })} />);
    expect(screen.getByTestId("quota-badge")).toHaveTextContent("53%");
  });
});
