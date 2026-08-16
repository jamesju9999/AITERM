import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { LocaleProvider } from "../contexts/LocaleContext";
import { RouteHint } from "./RouteHint";

function renderHint(onPick = vi.fn(), onDismiss = vi.fn()) {
  render(
    <LocaleProvider>
      <RouteHint pickedType="database" onPick={onPick} onDismiss={onDismiss} />
    </LocaleProvider>,
  );
  return { onPick, onDismiss };
}

describe("RouteHint", () => {
  // LocaleProvider 預設 zh-TW。
  it("說明 AI 判斷的是哪一種分頁", () => {
    renderHint();
    expect(screen.getByText(/AI 判斷你要的是「資料庫」分頁/)).toBeInTheDocument();
  });

  it("關掉提示會呼叫 onDismiss", () => {
    const { onDismiss } = renderHint();
    fireEvent.click(screen.getByRole("button", { name: "關閉提示" }));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it("選另一種分頁會用該 type 呼叫 onPick", () => {
    const { onPick } = renderHint();
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "terminal" } });
    expect(onPick).toHaveBeenCalledWith("terminal");
  });

  // 已經開的那一種不該出現在「換成」清單裡——選它等於什麼都沒做。
  it("換成清單不含目前這一種", () => {
    renderHint();
    const options = Array.from(
      screen.getByRole("combobox").querySelectorAll("option"),
    ).map((o) => (o as HTMLOptionElement).value);
    expect(options).not.toContain("database");
    expect(options).toContain("terminal");
  });

  // Claude Code 與一般終端機的 type 都是 "terminal"，清單裡會重複。
  it("換成清單裡每個類型只出現一次", () => {
    renderHint();
    const values = Array.from(
      screen.getByRole("combobox").querySelectorAll("option"),
    ).map((o) => (o as HTMLOptionElement).value).filter(Boolean);
    expect(new Set(values).size).toBe(values.length);
  });
});
