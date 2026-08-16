import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { LaunchGrid } from "./LaunchGrid";

function renderGrid(onOpenTab = vi.fn()) {
  render(
    <LocaleProvider>
      <LaunchGrid onOpenTab={onOpenTab} />
    </LocaleProvider>,
  );
  return onOpenTab;
}

describe("LaunchGrid", () => {
  // LocaleProvider 預設 zh-TW，所以斷言 zh-TW 的字串。
  it("列出可見的分頁類型", () => {
    renderGrid();
    expect(screen.getByText("終端機")).toBeInTheDocument();
    expect(screen.getByText("資料庫")).toBeInTheDocument();
  });

  // 原計畫點的是「終端機」，但那是清單第一項，無法分辨 onOpenTab 是否真的
  // 帶對了 type（例如硬編成 "terminal" 也會通過）。改點「資料庫」才能區分。
  it("點某一項會用對應的 type 呼叫 onOpenTab", () => {
    const onOpenTab = renderGrid();
    fireEvent.click(screen.getByText("資料庫"));
    expect(onOpenTab).toHaveBeenCalledWith("database");
  });

  // mail 與 api-docs 的後端完整但尚未對使用者開放，首頁不能變成它們的後門。
  it("不顯示 hidden 的分頁類型", () => {
    renderGrid();
    expect(screen.queryByText("信箱")).not.toBeInTheDocument();
    // api_docs_tab 的 zh-TW 值直接就是 "API Docs"（未翻譯）。
    expect(screen.queryByText("API Docs")).not.toBeInTheDocument();
  });
});
