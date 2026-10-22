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

  it("× 會關閉", () => {
    const onClose = vi.fn();
    view({ summary: "x", reasons: [], usedDefault: false, criteria: [] }, onClose);
    fireEvent.click(screen.getByRole("button", { name: /知道了|Got it/ }));
    expect(onClose).toHaveBeenCalled();
  });
});
