import { describe, expect, it } from "vitest";
// 刻意不用 `node:fs`：`tsconfig.app.json` 的 `types` 只有 `["vite/client"]`
// 且 `include` 整個 `src`（測試檔也在型別檢查範圍內），import node:fs 會讓
// `npx tsc -b` 失敗。改用 Vite 原生的 `?raw` 匯入，型別由 vite/client 提供。
// 同一個坑 `src/lib/chatgptWebInject.test.ts` 與
// `src/components/quotaBadgeCoverage.test.ts` 都踩過、也都繞過。
import tabBarSrc from "./TabBar/index.tsx?raw";
import terminalAppSrc from "./TerminalApp.tsx?raw";

/**
 * 新增分頁型別時，九個檔案會對 `TabType` 做分支。漏處理某一處**不會編譯
 * 失敗**（TypeScript 的 union 在 `if` 鏈裡不強制窮舉），只會變成執行期的
 * 怪現象：標題空白、圖示不見、關閉時沒有確認。
 *
 * 這個 repo 記過一次同類的坑：側邊欄的「Agent: xxx」分頁實際型別是
 * `terminal` 而非 `code-assistant`，型別判斷搞錯害整套分頁邏輯做錯。
 *
 * 所以這個測試直接掃原始碼，確認新型別在每個該出現的地方都出現了。
 * 掃字串比逐一寫 render 測試脆，但它抓的是「有沒有漏掉某個檔案」，
 * 那正是逐一寫測試最容易漏的東西。
 */
describe("remote-terminal tab type coverage", () => {
  it("is a member of the TabType union", () => {
    expect(tabBarSrc).toMatch(/export type TabType =[^;]*"remote-terminal"/s);
  });

  it("has an icon in the tab bar", () => {
    // 沒有圖示的分頁在側邊欄會是一個空格，看起來像壞掉。
    const iconFn = tabBarSrc.slice(tabBarSrc.indexOf("function getTabIcon"));
    expect(iconFn).toContain("remote-terminal");
  });

  it("gets a title when opened", () => {
    // TerminalApp 有一串 `if (type === "...") title = ...`。漏掉的話標題會是
    // 預設值或空字串。
    expect(terminalAppSrc).toMatch(/type === "remote-terminal"\s*\)\s*title\s*=/);
  });

  it("has a render branch", () => {
    expect(terminalAppSrc).toContain('tab.type === "remote-terminal"');
  });

  it("renders RemoteTerminalView", () => {
    expect(terminalAppSrc).toContain("RemoteTerminalView");
  });
});
