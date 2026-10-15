import { describe, expect, it } from "vitest";

import { translations } from "../../lib/i18n";
import { localizedCardTitle } from "./cardTitle";

const zh = translations["zh-TW"];
const en = translations.en;
const c = (card_kind: "work" | "review" | "fix" | "plan" | "merge", title: string) => ({ card_kind, title });

describe("localizedCardTitle", () => {
  it("中文介面原樣不變", () => {
    for (const card of [
      c("review", "審查：A 登入"), c("review", "審查規格：A（修正後）"), c("fix", "修正：B（第 2 輪）"),
      c("plan", "拆解規格：A"), c("merge", "合併 3 個工程師分支"),
    ]) {
      expect(localizedCardTitle(card, zh)).toBe(card.title);
    }
  });

  it("英文介面重組成英文，模組名稱保留", () => {
    expect(localizedCardTitle(c("review", "審查：A 登入"), en)).toBe("Review: A 登入");
    expect(localizedCardTitle(c("review", "審查規格：A（修正後）"), en)).toBe("Spec review: A (after fix)");
    expect(localizedCardTitle(c("fix", "修正：B（第 2 輪）"), en)).toBe("Fix: B (round 2)");
    expect(localizedCardTitle(c("plan", "拆解規格：A"), en)).toBe("Split spec: A");
    expect(localizedCardTitle(c("merge", "合併 3 個工程師分支"), en)).toBe("Merge 3 engineer branches");
    expect(localizedCardTitle(c("merge", "合併 2 個 Coder 分支"), en)).toBe("Merge 2 engineer branches");
  });

  it("一般卡片與對不上格式的標題原樣顯示", () => {
    expect(localizedCardTitle(c("work", "審查：看起來像但不是"), en)).toBe("審查：看起來像但不是");
    expect(localizedCardTitle(c("review", "使用者改過的標題"), en)).toBe("使用者改過的標題");
  });
});
