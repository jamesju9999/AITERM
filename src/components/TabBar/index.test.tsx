import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";

const navigateMock = vi.fn();
vi.mock("react-router-dom", async () => {
  const actual = await vi.importActual<typeof import("react-router-dom")>("react-router-dom");
  return { ...actual, useNavigate: () => navigateMock };
});

import { LocaleProvider } from "../../contexts/LocaleContext";
import { TabBar, type Tab, type TabBarProps } from "./index";
import { reorderTabs } from "./reorderTabs";

const baseTabs: Tab[] = [{ id: "t1", title: "Tab 1", type: "terminal" }];

function renderTabBar(overrides: Partial<TabBarProps> = {}) {
  return render(
    <LocaleProvider>
      <TabBar
        tabs={baseTabs}
        activeId="t1"
        onSelect={() => {}}
        onClose={() => {}}
        onAdd={() => {}}
        isSidebarOpen={false}
        onToggle={() => {}}
        width={260}
        {...overrides}
      />
    </LocaleProvider>,
  );
}

describe("TabBar settings navigation — collapsed gear icon", () => {
  beforeEach(() => {
    navigateMock.mockReset();
  });

  it("navigates to the about tab when hasUpdate is true", () => {
    renderTabBar({ isSidebarOpen: false, hasUpdate: true });
    fireEvent.click(screen.getByTitle(/Ctrl\+,/));
    expect(navigateMock).toHaveBeenCalledWith("/settings", { state: { tab: "about" } });
  });

  it("navigates to settings without a tab override when hasUpdate is false", () => {
    renderTabBar({ isSidebarOpen: false, hasUpdate: false });
    fireEvent.click(screen.getByTitle(/Ctrl\+,/));
    expect(navigateMock).toHaveBeenCalledWith("/settings", undefined);
  });
});

describe("TabBar settings navigation — expanded footer item", () => {
  beforeEach(() => {
    navigateMock.mockReset();
  });

  it("navigates to the about tab when hasUpdate is true", () => {
    renderTabBar({ isSidebarOpen: true, hasUpdate: true });
    fireEvent.click(screen.getByTitle(/Ctrl\+,/));
    expect(navigateMock).toHaveBeenCalledWith("/settings", { state: { tab: "about" } });
  });

  it("navigates to settings without a tab override when hasUpdate is false", () => {
    renderTabBar({ isSidebarOpen: true, hasUpdate: false });
    fireEvent.click(screen.getByTitle(/Ctrl\+,/));
    expect(navigateMock).toHaveBeenCalledWith("/settings", undefined);
  });
});

// LocaleProvider defaults to zh-TW, so these assert the zh-TW accessible names.
describe("TabBar mail connection-failure indicator", () => {
  const mailTabs: Tab[] = [{ id: "m1", title: "Mail", type: "mail" }];

  it("marks the mail icon when an account has stopped connecting", () => {
    renderTabBar({ tabs: mailTabs, activeId: "m1", mailFailedAccountCount: 1 });
    expect(screen.getByRole("img", { name: "1 個信箱帳號連線失敗" })).toBeTruthy();
  });

  it("shows nothing while every account is healthy", () => {
    renderTabBar({ tabs: mailTabs, activeId: "m1", mailFailedAccountCount: 0 });
    expect(screen.queryByRole("img", { name: /連線失敗/ })).toBeNull();
  });

  it("does not mark a non-mail tab", () => {
    renderTabBar({ tabs: baseTabs, activeId: "t1", mailFailedAccountCount: 1 });
    expect(screen.queryByRole("img", { name: /連線失敗/ })).toBeNull();
  });

  // The two are different kinds of signal, so they must stay tellable apart —
  // both by a screen reader (distinct accessible names) and visually (distinct
  // classes, which anchor them to opposite corners of the icon).
  it("stays distinguishable from the unread badge when both are showing", () => {
    renderTabBar({ tabs: mailTabs, activeId: "m1", mailUnreadCount: 3, mailFailedAccountCount: 2 });

    // Both queries throw if their element is missing, so reaching the last
    // assertion already proves the two coexist under distinct accessible names.
    const failure = screen.getByRole("img", { name: "2 個信箱帳號連線失敗" });
    const unread = screen.getByRole("img", { name: "3 封未讀郵件" });
    expect(failure.className).not.toBe(unread.className);
  });
});

// LocaleProvider 預設 zh-TW，所以這裡斷言 zh-TW 的無障礙名稱。
describe("TabBar terminal attention indicator", () => {
  // 提示點只出現在非 active 分頁上，所以這裡刻意讓 activeId 指向另一個分頁。
  const twoTabs: Tab[] = [
    { id: "t1", title: "Tab 1", type: "terminal" },
    { id: "t2", title: "Tab 2", type: "terminal" },
  ];

  it("等待輸入時標示該分頁", () => {
    renderTabBar({
      tabs: [twoTabs[0], { ...twoTabs[1], attention: "waiting" }],
      activeId: "t1",
    });
    expect(screen.getByRole("img", { name: "終端機正在等待你的回應" })).toBeTruthy();
  });

  it("指令完成時標示該分頁", () => {
    renderTabBar({
      tabs: [twoTabs[0], { ...twoTabs[1], attention: "done" }],
      activeId: "t1",
    });
    expect(screen.getByRole("img", { name: "終端機指令已完成" })).toBeTruthy();
  });

  it("指令失敗時標示該分頁", () => {
    renderTabBar({
      tabs: [twoTabs[0], { ...twoTabs[1], attention: "failed" }],
      activeId: "t1",
    });
    expect(screen.getByRole("img", { name: "終端機指令失敗" })).toBeTruthy();
  });

  it("三種狀態用不同的 class，才能是三種不同顏色", () => {
    for (const attention of ["waiting", "done", "failed"] as const) {
      const { unmount } = renderTabBar({
        tabs: [twoTabs[0], { ...twoTabs[1], attention }],
        activeId: "t1",
      });
      expect(screen.getByRole("img", { name: /終端機/ }).className)
        .toContain(`terminal-attention-badge--${attention}`);
      unmount();
    }
  });

  it("沒有 attention 時什麼都不顯示", () => {
    renderTabBar({ tabs: twoTabs, activeId: "t1" });
    expect(screen.queryByRole("img", { name: /終端機/ })).toBeNull();
  });

  it("非終端機分頁不顯示提示點", () => {
    renderTabBar({
      tabs: [twoTabs[0], { id: "m1", title: "Mail", type: "mail", attention: "waiting" }],
      activeId: "t1",
    });
    expect(screen.queryByRole("img", { name: /終端機/ })).toBeNull();
  });

  // 與既有 mail badge 的關係：兩者語意完全不同，class 必須可區分。
  // 比照本檔既有的「stays distinguishable from the unread badge」測試。
  it("與 mail 的 badge class 不相同", () => {
    const { unmount } = renderTabBar({
      tabs: [twoTabs[0], { ...twoTabs[1], attention: "failed" }],
      activeId: "t1",
    });
    const attention = screen.getByRole("img", { name: "終端機指令失敗" }).className;
    unmount();

    renderTabBar({
      tabs: [{ id: "m1", title: "Mail", type: "mail" }],
      activeId: "t1",
      mailFailedAccountCount: 1,
    });
    const mailFailure = screen.getByRole("img", { name: "1 個信箱帳號連線失敗" }).className;

    expect(attention).not.toBe(mailFailure);
  });
});

describe("reorderTabs", () => {
  it("把元素從 from 搬到 to", () => {
    expect(reorderTabs(["a", "b", "c"], 2, 0)).toEqual(["c", "a", "b"]);
    expect(reorderTabs(["a", "b", "c"], 0, 2)).toEqual(["b", "c", "a"]);
    expect(reorderTabs(["a", "b", "c", "d"], 1, 2)).toEqual(["a", "c", "b", "d"]);
  });

  it("不動原陣列", () => {
    const arr = ["a", "b", "c"];
    reorderTabs(arr, 2, 0);
    expect(arr).toEqual(["a", "b", "c"]);
  });

  // 回傳同一個參考，呼叫端才能靠 identity 判斷「沒有變化」而不觸發重繪。
  it("原地不動時回傳原本那個陣列", () => {
    const arr = ["a", "b", "c"];
    expect(reorderTabs(arr, 1, 1)).toBe(arr);
  });

  it("索引超出範圍時回傳原本那個陣列", () => {
    const arr = ["a", "b", "c"];
    expect(reorderTabs(arr, 3, 0)).toBe(arr);
    expect(reorderTabs(arr, 0, -1)).toBe(arr);
  });
});

describe("TabBar 首頁按鈕", () => {
  it("點首頁按鈕會呼叫 onHome", () => {
    const onHome = vi.fn();
    renderTabBar({ onHome, homeActive: false });
    fireEvent.click(screen.getByTitle(/Ctrl\+0/));
    expect(onHome).toHaveBeenCalledTimes(1);
  });

  it("首頁是目前畫面時，按鈕標成 active", () => {
    renderTabBar({ onHome: () => {}, homeActive: true });
    expect(screen.getByTitle(/Ctrl\+0/).className).toContain("active");
  });

  // 首頁不是分頁：它不能被拖曳排序、也不能佔用 Ctrl+1~9 的編號。
  it("首頁按鈕不在分頁清單裡", () => {
    const { container } = renderTabBar({ onHome: () => {}, homeActive: true });
    const inList = container.querySelectorAll(".aiterm-tabbar-tabs .aiterm-tab");
    expect(inList.length).toBe(1); // 只有 baseTabs 的那一個分頁
  });

  // 側邊欄收合成 48px 只剩圖示時，首頁按鈕必須還在。
  it("側邊欄收合時首頁按鈕仍在", () => {
    renderTabBar({ onHome: () => {}, homeActive: false, isSidebarOpen: false });
    expect(screen.getByTitle(/Ctrl\+0/)).toBeTruthy();
  });

  it("沒有給 onHome 就不顯示首頁按鈕", () => {
    renderTabBar({});
    expect(screen.queryByTitle(/Ctrl\+0/)).toBeNull();
  });

  // 停在首頁時，內容區顯示的是首頁而不是任何分頁；此時分頁若還亮著 active，
  // 側邊欄會有兩個「選取中」訊號互相矛盾。
  it("首頁顯示中時，分頁不顯示 active", () => {
    const { container } = renderTabBar({ onHome: () => {}, homeActive: true });
    const tab = container.querySelector(".aiterm-tabbar-tabs .aiterm-tab")!;
    expect(tab.className).not.toContain("active");
  });

  it("離開首頁後，active 分頁恢復標示", () => {
    const { container } = renderTabBar({ onHome: () => {}, homeActive: false });
    const tab = container.querySelector(".aiterm-tabbar-tabs .aiterm-tab")!;
    expect(tab.className).toContain("active");
  });
});

describe("TabBar 分頁拖曳排序", () => {
  const threeTabs: Tab[] = [
    { id: "t1", title: "Tab 1", type: "terminal" },
    { id: "t2", title: "Tab 2", type: "terminal" },
    { id: "t3", title: "Tab 3", type: "terminal" },
  ];

  /** .aiterm-tab 的高度（index.css）。 */
  const ROW = 56;

  /**
   * jsdom 沒有版面引擎，getBoundingClientRect 一律回傳全 0，落點判定會整個失效。
   * 這裡替每個分頁鋪上等距矩形，讓「拖到哪一格」算得出來。
   *
   * 也就是說：這些測試驗證的是排序邏輯，不是像素級的命中判定——手感仍須在真的
   * app 裡拖過才算數。
   */
  function layoutTabs(container: HTMLElement): HTMLElement[] {
    // 一定要限定在分頁清單裡：footer 的設定項也掛 .aiterm-tab。
    const els = Array.from(
      container.querySelectorAll<HTMLElement>(".aiterm-tabbar-tabs .aiterm-tab"),
    );
    els.forEach((el, i) => {
      el.getBoundingClientRect = () =>
        ({
          top: i * ROW, bottom: (i + 1) * ROW, height: ROW,
          left: 0, right: 48, width: 48, x: 0, y: i * ROW,
          toJSON: () => ({}),
        }) as DOMRect;
    });
    return els;
  }

  /** 第 i 格的垂直中心。 */
  const centerOf = (i: number) => i * ROW + ROW / 2;

  it("把第三個分頁往上拖到第一格，回報 (2, 0)", () => {
    const onReorder = vi.fn();
    const { container } = renderTabBar({ tabs: threeTabs, activeId: "t1", onReorder });
    const els = layoutTabs(container);

    fireEvent.mouseDown(els[2], { button: 0, clientY: centerOf(2) });
    fireEvent.mouseMove(window, { clientY: centerOf(0) });
    fireEvent.mouseUp(window, { clientY: centerOf(0) });

    expect(onReorder).toHaveBeenCalledTimes(1);
    expect(onReorder).toHaveBeenCalledWith(2, 0);
  });

  it("把第一個分頁往下拖到最後一格，回報 (0, 2)", () => {
    const onReorder = vi.fn();
    const { container } = renderTabBar({ tabs: threeTabs, activeId: "t1", onReorder });
    const els = layoutTabs(container);

    fireEvent.mouseDown(els[0], { button: 0, clientY: centerOf(0) });
    fireEvent.mouseMove(window, { clientY: centerOf(2) });
    fireEvent.mouseUp(window, { clientY: centerOf(2) });

    expect(onReorder).toHaveBeenCalledWith(0, 2);
  });

  // 沒有這道門檻的話，單純點分頁切換會被誤判成拖曳。
  it("移動不到門檻就不算拖曳：照常切換分頁、不重排", () => {
    const onReorder = vi.fn();
    const onSelect = vi.fn();
    const { container } = renderTabBar({ tabs: threeTabs, activeId: "t1", onReorder, onSelect });
    const els = layoutTabs(container);

    fireEvent.mouseDown(els[2], { button: 0, clientY: centerOf(2) });
    fireEvent.mouseMove(window, { clientY: centerOf(2) + 2 });
    fireEvent.mouseUp(window, { clientY: centerOf(2) + 2 });
    fireEvent.click(els[2]);

    expect(onReorder).not.toHaveBeenCalled();
    expect(onSelect).toHaveBeenCalledWith("t3");
  });

  it("拖曳後落回原本那一格，不回報重排", () => {
    const onReorder = vi.fn();
    const { container } = renderTabBar({ tabs: threeTabs, activeId: "t1", onReorder });
    const els = layoutTabs(container);

    // 往下移 20px：超過門檻算拖曳，但還沒越過下一格的中心。
    fireEvent.mouseDown(els[0], { button: 0, clientY: centerOf(0) });
    fireEvent.mouseMove(window, { clientY: centerOf(0) + 20 });
    fireEvent.mouseUp(window, { clientY: centerOf(0) + 20 });

    expect(onReorder).not.toHaveBeenCalled();
  });

  // 放開滑鼠後瀏覽器還會補一個 click，不擋掉的話拖完會順便切走分頁。
  it("真的拖曳過之後，收尾的那次點擊不切換分頁", () => {
    const onSelect = vi.fn();
    const { container } = renderTabBar({
      tabs: threeTabs, activeId: "t1", onSelect, onReorder: () => {},
    });
    const els = layoutTabs(container);

    fireEvent.mouseDown(els[2], { button: 0, clientY: centerOf(2) });
    fireEvent.mouseMove(window, { clientY: centerOf(0) });
    fireEvent.mouseUp(window, { clientY: centerOf(0) });
    fireEvent.click(els[2]);

    expect(onSelect).not.toHaveBeenCalled();
  });
});

// 橋接分頁的外觀分兩種來源：explicit 換整顆圖示，default 只加徽章。圖示是內聯
// SVG，沒有可查詢的文字或 role，所以認 svg 元素本身——同一個 .aiterm-tab-icon 裡
// 有沒有換成另一顆，用 innerHTML 比對最直接，也不用替每顆圖示加 test id。
function iconMarkupOf(tab: Tab): string {
  const { container } = renderTabBar({ tabs: [tab], activeId: tab.id });
  // 一定要限定在分頁清單裡：側邊欄收合鈕自己也掛 .aiterm-tab-icon，而且排在
  // 更前面，抓第一個會兩次都拿到同一顆收合圖示，測試就永遠通過。
  return container.querySelector(".aiterm-tabbar-tabs .aiterm-tab-icon")!.innerHTML;
}

/** 只取圖示本身（去掉徽章那些 span），才能單獨比對「圖示有沒有換」。 */
function svgOnly(markup: string): string {
  return markup.slice(0, markup.indexOf("</svg>") + 6);
}

describe("TabBar Claude Code bridge 外觀", () => {
  it("explicit 來源換成另一顆圖示", () => {
    const plain = svgOnly(iconMarkupOf({ id: "t1", title: "Tab 1", type: "terminal" }));
    cleanup();
    const bridged = svgOnly(
      iconMarkupOf({ id: "t1", title: "Tab 1", type: "terminal", claudeBridge: "explicit" }),
    );
    expect(bridged).not.toBe(plain);
  });

  // 使用者點的是「終端機」，把圖示整顆換掉會讓人以為自己點錯了。
  it("default 來源維持終端機圖示，只多一顆徽章", () => {
    const plain = svgOnly(iconMarkupOf({ id: "t1", title: "Tab 1", type: "terminal" }));
    cleanup();
    renderTabBar({
      tabs: [{ id: "t1", title: "Tab 1", type: "terminal", claudeBridge: "default" }],
      activeId: "t1",
    });
    const icon = document.querySelector(".aiterm-tabbar-tabs .aiterm-tab-icon")!;
    expect(svgOnly(icon.innerHTML)).toBe(plain);
    expect(screen.getByText("CC")).toBeInTheDocument();
  });

  it("explicit 來源不再疊徽章——已經換了整顆圖示", () => {
    renderTabBar({
      tabs: [{ id: "t1", title: "Tab 1", type: "terminal", claudeBridge: "explicit" }],
      activeId: "t1",
    });
    expect(screen.queryByText("CC")).not.toBeInTheDocument();
  });

  it("沒有橋接的終端機分頁兩者都沒有", () => {
    renderTabBar({ tabs: baseTabs, activeId: "t1" });
    expect(screen.queryByText("CC")).not.toBeInTheDocument();
  });

  it("非終端機分頁即使帶了旗標也維持原本的圖示", () => {
    const plain = iconMarkupOf({ id: "m1", title: "Mail", type: "mail" });
    cleanup();
    const bridged = iconMarkupOf({ id: "m1", title: "Mail", type: "mail", claudeBridge: "explicit" });
    expect(bridged).toBe(plain);
  });

  // 徽章曾經跟 attention 點搶同一塊版位，兩者要能同時出現。
  it("default 來源的橋接分頁仍然顯示 attention 提示點", () => {
    renderTabBar({
      tabs: [{ id: "t1", title: "Tab 1", type: "terminal", claudeBridge: "default", attention: "waiting" }],
      activeId: "t2",
    });
    expect(screen.getByText("CC")).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "終端機正在等待你的回應" })).toBeInTheDocument();
  });
});
