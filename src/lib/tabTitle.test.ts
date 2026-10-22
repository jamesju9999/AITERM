import { describe, expect, it } from "vitest";
import { translations } from "./i18n";
import { displayTabTitle } from "./tabTitle";

const zh = translations["zh-TW"];
const en = translations.en;

describe("displayTabTitle", () => {
  it("英文時建立的預設標題，中文介面顯示中文", () => {
    expect(displayTabTitle({ type: "terminal", title: "Terminal" }, zh)).toBe("終端機");
    expect(displayTabTitle({ type: "agent-canvas", title: "Agent Canvas" }, zh)).toBe("Agent 協作");
    expect(displayTabTitle({ type: "architecture", title: "Architecture" }, zh)).toBe("架構導覽");
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

  it("Agent 協作分頁帶上專案名稱：側邊欄只顯示專案名，其他地方是「Agent 協作 · 專案名」", () => {
    const tab = { type: "agent-canvas" as const, title: "Agent Canvas", agentCanvasProjectName: "Ask Test" };
    expect(displayTabTitle(tab, zh, "short")).toBe("Ask Test");
    expect(displayTabTitle(tab, zh)).toBe("Agent 協作 · Ask Test");
  });

  it("改過名的 Agent 協作分頁不加專案名", () => {
    const tab = { type: "agent-canvas" as const, title: "我的團隊", agentCanvasProjectName: "Ask Test" };
    expect(displayTabTitle(tab, zh, "short")).toBe("我的團隊");
  });
});
