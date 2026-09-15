import { describe, expect, it } from "vitest";
import { languageDirective, localeSources } from "./i18n";

describe("languageDirective", () => {
  it("returns English for en locale", () => {
    expect(languageDirective("en")).toBe("English");
  });

  it("returns Traditional Chinese for zh-TW locale", () => {
    expect(languageDirective("zh-TW")).toBe("Traditional Chinese (繁體中文)");
  });
});

describe("兩個語系的字典要對齊", () => {
  // 這條測試比對的是**原始**字典（localeSources），不是 translations。
  //
  // translations.en 是 `{...zhTW, ...enRaw}`，key 集合永遠等於 zhTW，所以拿它
  // 比對的話必然通過——i18n.remoteTerminal.test.ts 以前就是那樣寫的，那條測試
  // 從落地那天起就不可能失敗，而英文介面實際上已經累積了 93 個顯示中文的字串。
  //
  // 漏掉的後果是靜默的：不會報錯、不會空白，就是英文介面上出現繁體中文。
  it("英文字典不能缺 key，否則英文介面會靜默顯示中文", () => {
    const zh = Object.keys(localeSources["zh-TW"]);
    const en = new Set(Object.keys(localeSources.en));
    const missing = zh.filter((k) => !en.has(k));
    expect(missing, `英文缺少 ${missing.length} 個字串`).toEqual([]);
  });

  it("英文字典不能有 zh-TW 沒有的 key", () => {
    // 這個方向代表 zh-TW 那邊被刪掉或打錯字了——TranslationKey 是從 zhTW
    // 推導的，所以多出來的 en key 永遠不會被任何地方讀到。
    const en = Object.keys(localeSources.en);
    const zh = new Set(Object.keys(localeSources["zh-TW"]));
    expect(en.filter((k) => !zh.has(k))).toEqual([]);
  });
});
