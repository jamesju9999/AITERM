import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

vi.mock("../../ipc/bridge", () => ({ bridgeStatus: vi.fn() }));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { LaunchGrid } from "./LaunchGrid";
import { bridgeStatus } from "../../ipc/bridge";

beforeEach(() => {
  vi.mocked(bridgeStatus).mockReset();
});

// bridgeRunning 預設 true：大多數既有測試不在乎橋接選項，只有專門測停用
// 狀態的案例才傳 false。
function renderGrid(onOpenTab = vi.fn(), bridgeRunning = true) {
  vi.mocked(bridgeStatus).mockResolvedValue({ running: bridgeRunning, port: bridgeRunning ? 8317 : null, token: bridgeRunning ? "tok" : null, error: null });
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
    expect(onOpenTab).toHaveBeenCalledWith("database", undefined);
  });

  // mail 與 api-docs 的後端完整但尚未對使用者開放，首頁不能變成它們的後門。
  it("不顯示 hidden 的分頁類型", () => {
    renderGrid();
    expect(screen.queryByText("信箱")).not.toBeInTheDocument();
    // api_docs_tab 的 zh-TW 值直接就是 "API Docs"（未翻譯）。
    expect(screen.queryByText("API Docs")).not.toBeInTheDocument();
  });

  it("橋接執行中時，Claude Code 可以點，並帶上 claudeBridge 選項", async () => {
    const onOpenTab = vi.fn();
    renderGrid(onOpenTab, true);
    const button = await screen.findByText("Claude Code");
    await waitFor(() => expect(button.closest("button")).toBeEnabled());
    fireEvent.click(button);
    expect(onOpenTab).toHaveBeenCalledWith("terminal", { claudeBridge: true });
  });

  // 建立一個注入了死埠位址的分頁，比不給點更難除錯——這是選單既有的理由，
  // 首頁沿用同一個規則。
  it("橋接沒在跑時，Claude Code 停用並顯示原因", async () => {
    renderGrid(vi.fn(), false);
    const card = (await screen.findByText("Claude Code")).closest("button")!;
    await waitFor(() => expect(card).toBeDisabled());
    expect(screen.getByText(/橋接 server 尚未啟動/)).toBeInTheDocument();
  });

  it("一般分頁不受橋接狀態影響", () => {
    const onOpenTab = vi.fn();
    renderGrid(onOpenTab, false);
    fireEvent.click(screen.getByText("資料庫"));
    expect(onOpenTab).toHaveBeenCalledWith("database", undefined);
  });
});
