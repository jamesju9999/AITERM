import { describe, expect, it } from "vitest";
import { languageDirective } from "./i18n";

describe("languageDirective", () => {
  it("returns English for en locale", () => {
    expect(languageDirective("en")).toBe("English");
  });

  it("returns Traditional Chinese for zh-TW locale", () => {
    expect(languageDirective("zh-TW")).toBe("Traditional Chinese (繁體中文)");
  });
});
