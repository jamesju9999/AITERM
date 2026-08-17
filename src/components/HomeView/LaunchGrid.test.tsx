import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { MemoryRouter, useNavigate } from "react-router-dom";

vi.mock("../../ipc/bridge", () => ({ bridgeStatus: vi.fn() }));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { LaunchGrid } from "./LaunchGrid";
import { bridgeStatus } from "../../ipc/bridge";

beforeEach(() => {
  vi.mocked(bridgeStatus).mockReset();
});

// bridgeRunning 預設 true：大多數既有測試不在乎橋接選項，只有專門測停用
// 狀態的案例才傳 false。useBridgeRunning 靠 useLocation() 判斷是否在首頁，
// 所以要包一層 MemoryRouter，預設路徑就是 "/"。
function renderGrid(onOpenTab = vi.fn(), bridgeRunning = true) {
  vi.mocked(bridgeStatus).mockResolvedValue({ running: bridgeRunning, port: bridgeRunning ? 8317 : null, token: bridgeRunning ? "tok" : null, error: null });
  render(
    <MemoryRouter initialEntries={["/"]}>
      <LocaleProvider>
        <LaunchGrid onOpenTab={onOpenTab} />
      </LocaleProvider>
    </MemoryRouter>,
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
  // 首頁沿用同一個規則。bridgeRunning 的初始值本來就是 false，所以光測
  // 「一開始就是停用」測不出 disabled 是不是真的跟著 hook 的回傳值走——就算
  // 把 disabled={disabled} 整個刪掉，初始 render 那一瞬間（查詢還沒回來）
  // 也剛好是 disabled 的外觀，這條測試照樣會綠。
  //
  // 所以這裡不測「一開始就停用」，改測「原本可點、橋接被關掉、離開首頁再
  // 回來後變成停用」——這正是本次要修的 stale bug 的使用者路徑（見
  // useBridgeRunning.test.tsx 對這條重查邏輯的單元測試）。
  it("橋接被關掉、離開首頁再回來後，Claude Code 變成停用", async () => {
    const onOpenTab = vi.fn();
    vi.mocked(bridgeStatus).mockResolvedValue({ running: true, port: 8317, token: "tok", error: null });

    // LaunchGrid render 在這裡是無條件的，不透過 <Route> 切換——這才對應
    // App.tsx 的真實行為：TerminalApp（因此連帶 LaunchGrid）永遠掛著不會
    // unmount，只有 pathname 在變。用 <Route> 讓它跟著路徑掛載/卸載，會
    // 掩蓋掉這次要修的 stale bug（unmount 重新掛載本來就會查到新值）。
    function Screen() {
      const navigate = useNavigate();
      return (
        <>
          <button onClick={() => navigate("/settings")}>go-settings</button>
          <button onClick={() => navigate("/")}>go-home</button>
          <LaunchGrid onOpenTab={onOpenTab} />
        </>
      );
    }

    render(
      <MemoryRouter initialEntries={["/"]}>
        <LocaleProvider>
          <Screen />
        </LocaleProvider>
      </MemoryRouter>,
    );

    const button = (await screen.findByText("Claude Code")).closest("button")!;
    await waitFor(() => expect(button).toBeEnabled());

    vi.mocked(bridgeStatus).mockResolvedValue({ running: false, port: null, token: null, error: null });
    fireEvent.click(screen.getByText("go-settings"));
    fireEvent.click(screen.getByText("go-home"));

    await waitFor(() => expect(button).toBeDisabled());
    expect(screen.getByText(/橋接 server 尚未啟動/)).toBeInTheDocument();

    fireEvent.click(button);
    expect(onOpenTab).not.toHaveBeenCalled();
  });

  it("一般分頁不受橋接狀態影響", () => {
    const onOpenTab = vi.fn();
    renderGrid(onOpenTab, false);
    fireEvent.click(screen.getByText("資料庫"));
    expect(onOpenTab).toHaveBeenCalledWith("database", undefined);
  });

  // 卡片的分頁類型專屬色是透過 CSS 自訂屬性 --card-color 傳給 CSS，不是
  // 寫死的 class，所以要查 style 屬性才能確認顏色真的有從 catalog 傳下來。
  it("每張卡片都帶有對應分頁類型的 --card-color", () => {
    renderGrid();
    const terminalCard = screen.getByText("終端機").closest("button")!;
    const databaseCard = screen.getByText("資料庫").closest("button")!;
    expect(terminalCard.style.getPropertyValue("--card-color")).toBe("#4ade80");
    expect(databaseCard.style.getPropertyValue("--card-color")).toBe("#60a5fa");
  });
});
