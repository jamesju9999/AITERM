import { describe, expect, it } from "vitest";
import { colorForTab, getTabCatalog, visibleTabCatalog } from "./tabCatalog";
import { translations } from "../../lib/i18n";
import type { TabType } from "../TabBar";

const t = translations["zh-TW"];

// 新增一種 TabType 卻忘了在 catalog 補一筆時，首頁的大圖入口和 AI 路由都會默默地
// 少一個選項，而且不會有任何錯誤。窮盡性（有沒有漏列 TabType）是靠下面的型別檢查
// 擋住的，不是靠這個檔案裡的任何一個 it()——少列一種 TabType，`_exhaustive` 那行
// 會是型別錯誤，`npx tsc -b` 會擋下來。
const ALL_TYPES = [
  "terminal", "database", "design", "cross-db", "vcs", "doc-converter",
  "api-docs", "loop-studio", "code-assistant", "knowledge-base", "mail",
  "remote-terminal",
] as const satisfies readonly TabType[];

// 少列一種 TabType 時，這行會是型別錯誤（tsc -b 會擋）
const _exhaustive: [Exclude<TabType, (typeof ALL_TYPES)[number]>] extends [never] ? true : never = true;
void _exhaustive;

describe("getTabCatalog", () => {
  it("涵蓋每一種 TabType", () => {
    const types = getTabCatalog(t).map((e) => e.type);
    for (const type of ALL_TYPES) {
      expect(types).toContain(type);
    }
  });

  it("每一筆都有非空的標題與說明", () => {
    for (const entry of getTabCatalog(t)) {
      expect(entry.label.length).toBeGreaterThan(0);
      expect(entry.desc.length).toBeGreaterThan(0);
    }
  });

  // mail、api-docs 的後端／畫面都是完整的，只是還沒對使用者開放；用 hidden
  // 旗標記錄這件事，比在兩個地方各註解掉一行可靠。api-docs 的隱藏是 commit
  // 3547799 刻意做的決定，不是遺漏。remote-terminal 曾經也是 hidden（2B-2a
  // 只做畫面，還沒有連線入口），2B-2b 把「連線到同事的終端機」的入口
  // （分享按鈕、同意視窗、連線對話框）補上之後就拿掉了 hidden。
  it("mail、api-docs 標成 hidden，remote-terminal 跟其餘不是", () => {
    const catalog = getTabCatalog(t);
    expect(catalog.find((e) => e.type === "mail")!.hidden).toBe(true);
    expect(catalog.find((e) => e.type === "api-docs")!.hidden).toBe(true);
    expect(catalog.find((e) => e.type === "remote-terminal")!.hidden).toBeFalsy();
    expect(catalog.filter((e) => e.hidden).map((e) => e.type).sort()).toEqual([
      "api-docs", "mail",
    ]);
  });

  it("visibleTabCatalog 濾掉 hidden 的項目", () => {
    expect(visibleTabCatalog(t).some((e) => e.type === "mail")).toBe(false);
  });

  // 這個回歸真的發生過：把清單抽成單一來源時，api-docs 曾經漏掉 hidden 標記，
  // 導致它從「刻意收起來的入口」變成「重新出現在選單裡」。
  it("visibleTabCatalog 不含 api-docs（迴歸測試，見 commit 3547799）", () => {
    expect(visibleTabCatalog(t).some((e) => e.type === "api-docs")).toBe(false);
  });

  it("包含 Claude Code，且它是帶 claudeBridge 選項的終端機分頁", () => {
    const entry = getTabCatalog(t).find((e) => e.id === "claude-code")!;
    expect(entry).toBeDefined();
    expect(entry.type).toBe("terminal");
    expect(entry.opts).toEqual({ claudeBridge: true });
    expect(entry.requiresBridge).toBe(true);
    // 停用提示字串掛在 entry 上，消費端（NewTabPicker、LaunchGrid）不必
    // 各自認識「這是橋接的提示」這件事，只要讀 entry.disabledHint。
    expect(entry.disabledHint).toBe(t.bridge_new_tab_disabled_hint);
  });

  // 兩筆的 type 都是 "terminal"，所以 id 必須唯一，否則 React key 會撞。
  it("每一筆的 id 都是唯一的", () => {
    const ids = getTabCatalog(t).map((e) => e.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  // 首頁與新增分頁選單看到的項目數必須一致——這個缺口實際發生過：
  // 選單有 10 個入口，首頁只渲染了 catalog 的 9 個。
  it("可見項目包含一般終端機與 Claude Code 兩筆", () => {
    const visible = visibleTabCatalog(t);
    expect(visible.filter((e) => e.type === "terminal")).toHaveLength(2);
  });

  // color 是必填欄位（型別上不是 optional），但型別檢查擋不住寫錯格式的
  // 值（例如少打一個字元的 hex），這裡用 regex 補上執行期斷言。
  it("每一筆的 color 都是合法的 6 位 hex", () => {
    for (const entry of getTabCatalog(t)) {
      expect(entry.color).toMatch(/^#[0-9a-f]{6}$/i);
    }
  });

  // C 方案的重點是「每類一色」——兩筆撞色就失去分辨分頁類型的作用。
  it("每一筆的 color 互不重複", () => {
    const colors = getTabCatalog(t).map((e) => e.color.toLowerCase());
    expect(new Set(colors).size).toBe(colors.length);
  });
});

describe("colorForTab", () => {
  // 這是這個函式唯一有邏輯的分支：type 是 "terminal" 的分頁裡，帶
  // claudeBridge 的那些要拿到 claude-code 的陶土色，不能被 type 一起
  // find 到一般終端機的綠色。"explicit"／"default" 都算 Claude Code，
  // 只要 claudeBridge 是 truthy。
  it("terminal 型別但帶 claudeBridge 時拿到 Claude Code 的陶土色", () => {
    expect(colorForTab("terminal", "explicit")).toBe("#d97757");
    expect(colorForTab("terminal", "default")).toBe("#d97757");
  });

  it("一般終端機（沒有 claudeBridge）拿到終端機的綠色", () => {
    expect(colorForTab("terminal")).toBe("#4ade80");
  });

  it("資料庫分頁拿到藍色", () => {
    expect(colorForTab("database")).toBe("#60a5fa");
  });
});
