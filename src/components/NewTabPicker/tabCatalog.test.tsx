import { describe, expect, it } from "vitest";
import { getTabCatalog, visibleTabCatalog } from "./tabCatalog";
import { translations } from "../../lib/i18n";
import type { TabType } from "../TabBar";

const t = translations["zh-TW"];

// 這個測試存在的理由：新增一種 TabType 卻忘了在 catalog 補一筆時，首頁的大圖
// 入口和 AI 路由都會默默地少一個選項，而且不會有任何錯誤。
const ALL_TYPES: TabType[] = [
  "terminal", "database", "design", "cross-db", "vcs", "doc-converter",
  "api-docs", "loop-studio", "code-assistant", "knowledge-base", "mail",
];

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

  // mail 與 api-docs 的後端都是完整的，只是還沒對使用者開放；用 hidden 旗標記錄
  // 這件事，比在兩個地方各註解掉一行可靠。api-docs 的隱藏是 commit 3547799 刻意
  // 做的決定，不是遺漏。
  it("mail 與 api-docs 標成 hidden，其餘不是", () => {
    const catalog = getTabCatalog(t);
    expect(catalog.find((e) => e.type === "mail")!.hidden).toBe(true);
    expect(catalog.find((e) => e.type === "api-docs")!.hidden).toBe(true);
    expect(catalog.filter((e) => e.hidden).map((e) => e.type).sort()).toEqual(["api-docs", "mail"]);
  });

  it("visibleTabCatalog 濾掉 hidden 的項目", () => {
    expect(visibleTabCatalog(t).some((e) => e.type === "mail")).toBe(false);
  });

  // 這個回歸真的發生過：把清單抽成單一來源時，api-docs 曾經漏掉 hidden 標記，
  // 導致它從「刻意收起來的入口」變成「重新出現在選單裡」。
  it("visibleTabCatalog 不含 api-docs（迴歸測試，見 commit 3547799）", () => {
    expect(visibleTabCatalog(t).some((e) => e.type === "api-docs")).toBe(false);
  });
});
