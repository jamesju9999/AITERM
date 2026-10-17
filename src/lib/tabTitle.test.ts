import { describe, expect, it } from "vitest";
import { translations } from "./i18n";
import { displayTabTitle } from "./tabTitle";

const zh = translations["zh-TW"];
const en = translations.en;

describe("displayTabTitle", () => {
  it("英文時建立的預設標題，中文介面顯示中文", () => {
    expect(displayTabTitle({ type: "terminal", title: "Terminal" }, zh)).toBe("終端機");
    expect(displayTabTitle({ type: "agent-canvas", title: "Agent Canvas" }, zh)).toBe("Agent 協作");
  });

  it("中文時建立的預設標題，英文介面顯示英文", () => {
    expect(displayTabTitle({ type: "database", title: "資料庫" }, en)).toBe("Database");
  });

  it("使用者改過的標題照原樣", () => {
    expect(displayTabTitle({ type: "terminal", title: "PingPon" }, zh)).toBe("PingPon");
  });

  it("別種分頁的預設標題不會被套用", () => {
    expect(displayTabTitle({ type: "terminal", title: "Database" }, zh)).toBe("Database");
  });
});
