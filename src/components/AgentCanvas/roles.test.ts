import { describe, expect, it } from "vitest";

import { translations } from "../../lib/i18n";
import { roleIcon } from "./roles";

describe("roleIcon", () => {
  it("內建工程師角色對到自己的圖示", () => {
    expect(roleIcon("coder", translations["zh-TW"].ls_preset_architect)).toBe("landmark");
    expect(roleIcon("coder", translations["zh-TW"].ac_role_frontend)).toBe("palette");
  });

  it("切換語系後，用另一個語系存的角色名照樣對得到", () => {
    expect(roleIcon("coder", translations.en.ls_preset_architect)).toBe("landmark");
  });

  it("名稱前後空白不影響", () => {
    expect(roleIcon("coder", `  ${translations["zh-TW"].ls_preset_tester} `)).toBe("test");
  });

  it("自訂角色、空白、非工程師都沒有（沿用種類圖示）", () => {
    expect(roleIcon("coder", "我的自訂角色")).toBeNull();
    expect(roleIcon("coder", "")).toBeNull();
    expect(roleIcon("reviewer", translations["zh-TW"].ls_preset_architect)).toBeNull();
  });
});
