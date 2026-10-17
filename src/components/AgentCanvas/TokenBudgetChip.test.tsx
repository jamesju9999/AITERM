import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { LocaleProvider } from "../../contexts/LocaleContext";
import { TokenBudgetChip } from "./TokenBudgetChip";

const sum = (n: number) => ({ usage: { input: n, cache_write: 0, cache_read: 0, output: 0, calls: 1 }, cards: 1, missing: 0 });
const view = (budget: number | null, used: number, onSave = vi.fn().mockResolvedValue(undefined)) => {
  render(
    <LocaleProvider>
      <TokenBudgetChip sum={sum(used)} budget={budget} onSave={onSave} />
    </LocaleProvider>,
  );
  return onSave;
};

describe("TokenBudgetChip", () => {
  it("沒設上限只顯示用量；設了顯示 用量 / 上限，用到上限變紅並說明暫停派工", () => {
    view(null, 1_250_000);
    expect(screen.getByTestId("ac-tokens").textContent).toBe("Σ 1.25M token");
  });

  it("用到上限", () => {
    view(1_000_000, 1_250_000);
    const chip = screen.getByTestId("ac-tokens");
    expect(chip.textContent).toBe("Σ 1.25M / 1.00M token · 已達上限，Agent 卡暫停派工");
    expect(chip.className).toContain("ac-tokens--over");
  });

  it("輸入 M 為單位的上限並儲存；可以取消上限", async () => {
    const onSave = view(2_000_000, 100);
    fireEvent.click(screen.getByTestId("ac-tokens"));
    const input = screen.getByRole("spinbutton", { name: "token 上限" });
    expect((input as HTMLInputElement).value).toBe("2");
    fireEvent.change(input, { target: { value: "5.5" } });
    fireEvent.click(screen.getByRole("button", { name: "儲存" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith(5_500_000));
    fireEvent.click(screen.getByTestId("ac-tokens"));
    fireEvent.click(screen.getByRole("button", { name: "取消上限" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith(null));
  });
});
