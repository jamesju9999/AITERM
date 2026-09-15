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

  // 這裡原本有一條 "keeps the two locales in sync for sharing strings"，比對
  // `Object.keys(translations["zh-TW"])` 與 `Object.keys(translations.en)`。
  //
  // **那條測試從落地那天起就不可能失敗。** `translations.en` 是
  // `{...zhTW, ...enRaw}`，所以它的 key 集合永遠等於 `zhTW`，兩邊必然相等。
  // 它的註解寫著「語系漂移是這個 repo 記過的坑」——為了那件事而寫，卻完全
  // 防不到它：發現的當下英文字典實際上已經缺了 93 個 key。
  //
  // 取代它的是 `i18n.test.ts` 的「兩個語系的字典要對齊」，那條比對的是
  // **原始**字典（`localeSources`）而不是合併後的結果，而且涵蓋全部 key
  // 而不只這四個前綴。
});
