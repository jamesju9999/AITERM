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

  // mail 的後端是完整的，只是還沒對使用者開放；用 hidden 旗標記錄這件事，
  // 比在兩個地方各註解掉一行可靠。
  it("mail 標成 hidden，其餘不是", () => {
    const catalog = getTabCatalog(t);
    expect(catalog.find((e) => e.type === "mail")!.hidden).toBe(true);
    expect(catalog.filter((e) => e.hidden).map((e) => e.type)).toEqual(["mail"]);
  });

  it("visibleTabCatalog 濾掉 hidden 的項目", () => {
    expect(visibleTabCatalog(t).some((e) => e.type === "mail")).toBe(false);
  });
});
