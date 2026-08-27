import { describe, expect, it } from "vitest";
import { translations } from "./i18n";

/**
 * 後端 `EndReason` 的八個值。改動這個陣列前先看
 * `src-tauri/src/share/protocol.rs`——那邊新增變體時，這裡跟 i18n 都要跟著加。
 */
const END_REASONS = [
  "denied",
  "host_stopped_sharing",
  "session_closed",
  "kicked_by_host",
  "invalid_code",
  "version_mismatch",
  "sas_commit_mismatch",
  "sas_handshake_failed",
] as const;

describe("remote terminal i18n", () => {
  for (const locale of ["zh-TW", "en"] as const) {
    it(`has a human sentence for every end reason in ${locale}`, () => {
      // spec 要求「不能有『未知錯誤』」——每個結束原因都要有一句人話。
      // `Translations` 裡混了函式型的值（例如 home_resume_count），跟
      // `Record<string, string>` 不完全重疊，所以先轉 `unknown` 再轉型別，
      // 範圍限縮在這個測試檔裡。
      const t = translations[locale] as unknown as Record<string, string>;
      for (const reason of END_REASONS) {
        const key = `remote_terminal_ended_${reason}`;
        expect(t[key], `missing ${key} in ${locale}`).toBeTruthy();
      }
    });
  }

  it("keeps the two locales in sync for sharing strings", () => {
    // 語系漂移是這個 repo 記過的坑：只加一邊，另一邊會靜默 fallback 或空白。
    const prefixes = ["remote_terminal_", "share_", "consent_", "connect_"];
    const pick = (loc: "zh-TW" | "en") =>
      Object.keys(translations[loc])
        .filter((k) => prefixes.some((p) => k.startsWith(p)))
        .sort();
    expect(pick("zh-TW")).toEqual(pick("en"));
  });
});
