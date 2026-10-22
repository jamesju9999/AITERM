import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

import { LocaleProvider } from "../../contexts/LocaleContext";
import { StartNotice } from "./StartNotice";

const view = (result: Parameters<typeof StartNotice>[0]["result"], onClose = vi.fn()) =>
  render(
    <LocaleProvider>
      <StartNotice result={result} onClose={onClose} />
    </LocaleProvider>,
  );

describe("StartNotice", () => {
  it("預設只露出摘要，理由收起來，按了才展開", () => {
    view({ summary: "先分析再實作", reasons: [{ label: "需求分析師", reason: "規格要先釐清" }], usedDefault: false, criteria: [] });
    expect(screen.getByText("先分析再實作")).toBeTruthy();
    expect(screen.queryByTestId("ac-req-notice-reasons")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /各角色理由|Why these roles/ }));
    expect(screen.getByTestId("ac-req-notice-reasons").textContent).toContain("規格要先釐清");
    fireEvent.click(screen.getByRole("button", { name: /收起理由|Hide reasons/ }));
    expect(screen.queryByTestId("ac-req-notice-reasons")).toBeNull();
  });

  it("用了預設團隊時不顯示摘要與理由，只說明原因", () => {
    view({ summary: "標準", reasons: [], usedDefault: true, criteria: [] });
    expect(screen.queryByRole("button", { name: /各角色理由|Why these roles/ })).toBeNull();
    expect(screen.getByTestId("ac-req-notice").textContent).toMatch(/預設團隊|default team/);
  });

  it("有完成定義時列出來，讓使用者知道驗收會檢查什麼", () => {
    view({ summary: "x", reasons: [], usedDefault: false, criteria: ["按開始會倒數", "重新整理後紀錄還在"] });
    const box = screen.getByTestId("ac-req-notice-criteria");
    expect(box.textContent).toContain("按開始會倒數");
    expect(box.textContent).toContain("重新整理後紀錄還在");
  });

  it("沒有完成定義就不顯示那一段", () => {
    view({ summary: "x", reasons: [], usedDefault: false, criteria: [] });
    expect(screen.queryByTestId("ac-req-notice-criteria")).toBeNull();
  });

  it("有還沒派出去的需求時顯示「開始執行」，按了才呼叫；失敗顯示原因並可再按", async () => {
    const pending = { taskId: "t1", stationId: "s1" };
    const onStart = vi.fn().mockRejectedValueOnce(new Error("boom")).mockResolvedValueOnce(undefined);
    render(
      <LocaleProvider>
        <StartNotice result={{ summary: "x", reasons: [], usedDefault: false, criteria: [], pending }} onClose={vi.fn()} onStart={onStart} />
      </LocaleProvider>,
    );
    expect(onStart).not.toHaveBeenCalled();
    fireEvent.click(screen.getByTestId("ac-req-start-run"));
    expect(await screen.findByText("boom")).toBeTruthy();
    fireEvent.click(screen.getByTestId("ac-req-start-run"));
    await vi.waitFor(() => expect(onStart).toHaveBeenCalledTimes(2));
    expect(onStart).toHaveBeenLastCalledWith(pending);
  });

  it("沒有待派的需求就沒有開始按鈕", () => {
    view({ summary: "x", reasons: [], usedDefault: false, criteria: [] });
    expect(screen.queryByTestId("ac-req-start-run")).toBeNull();
  });

  it("× 會關閉", () => {
    const onClose = vi.fn();
    view({ summary: "x", reasons: [], usedDefault: false, criteria: [] }, onClose);
    fireEvent.click(screen.getByRole("button", { name: /知道了|Got it/ }));
    expect(onClose).toHaveBeenCalled();
  });
});
