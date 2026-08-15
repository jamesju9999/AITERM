import { describe, it, expect } from "vitest";

/**
 * 結構性測試：**每個會顯示 provider 給使用者挑的地方，都要有配額徽章。**
 *
 * 這條規則在實作過程中被違反了三次，每次都是實機才發現，而且型別檢查、
 * 元件自己的測試、lint 全都是綠的：
 *
 *   1. 終端機標題列 —— app 的主畫面，卻不是用 ModelPickerButton
 *   2. Ask AI 面板 —— 同上
 *   3. 設計與規格 —— 同上
 *
 * 共同原因：規格假設「改 ModelPickerButton 一處就到處都有」，但實際上有
 * 三個視圖有自己的 provider 按鈕。單元測試抓不到「某個畫面少了一個東西」，
 * 只有把這個不變式寫成靜態檢查才擋得住下一次。
 *
 * 判準：一個檔案若含有 provider 切換入口，就必須也引用 `QuotaBadge`。
 * 入口有兩種形態，**兩種都要算**：
 *   - 自己渲染 `<ProviderPalette`（TerminalView、DesignView）
 *   - 只有一顆按鈕、把面板交給上層開（AiPanel 的 `onOpenProviderPalette`）
 * 一開始只認第一種，漏掉了 AiPanel —— 是下面那個「防呆」測試抓出來的。
 * 走 `ModelPickerButton` 的視圖不受此限，徽章已經包在那個元件裡了。
 *
 * 刻意不用 `node:fs`：`tsconfig.app.json` 的 `types` 只有 `["vite/client"]`，
 * 直接 import node 型別會讓 `npx tsc -b` 失敗（`src/lib/chatgptWebInject.test.ts`
 * 也踩過同一個坑）。改用 Vite 的 `?raw` glob，在打包階段就把原始碼讀進來。
 */

const sources = import.meta.glob("../**/*.tsx", {
  query: "?raw",
  eager: true,
  import: "default",
}) as Record<string, string>;

/** 這個檔案裡有沒有讓使用者切換 provider 的入口。 */
function hasProviderSwitcher(src: string): boolean {
  return src.includes("<ProviderPalette") || src.includes("onOpenProviderPalette");
}

describe("配額徽章的覆蓋範圍", () => {
  it("每個有 provider 切換入口的視圖都要顯示配額徽章", () => {
    const offenders = Object.entries(sources)
      .filter(([path]) => !/\.test\.tsx$/.test(path))
      .filter(([path]) => !path.endsWith("ProviderPalette.tsx"))
      .filter(([, src]) => hasProviderSwitcher(src) && !src.includes("QuotaBadge"))
      .map(([path]) => path);

    expect(
      offenders,
      "這些視圖讓使用者挑 provider，卻沒有顯示配額徽章。\n" +
        "請用 useProviderQuota(providerId) 取得代表窗並渲染 <QuotaBadge>，\n" +
        "或說明為什麼這個畫面不需要：\n  " +
        offenders.join("\n  "),
    ).toEqual([]);
  });

  it("這個檢查本身有掃到東西（避免 glob 失效變成永遠通過）", () => {
    // glob 路徑寫錯時 sources 會是空物件，上面那個測試就永遠綠 ——
    // 那比沒有測試更糟，因為它看起來還在保護。
    const withSwitcher = Object.values(sources).filter(hasProviderSwitcher);
    expect(withSwitcher.length).toBeGreaterThanOrEqual(3);
  });
});
