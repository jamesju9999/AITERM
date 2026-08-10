import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

/**
 * inject.js 是給瀏覽器用的平鋪 script，直接 import 會找不到 window。這裡把它
 * 讀進來、在一個假的 window 上求值，再取出掛上去的純函式來測。
 */
function loadInject(): Record<string, unknown> {
  const src = readFileSync("src-tauri/src/chatgpt_web/inject.js", "utf8");
  const win: Record<string, unknown> = {};
  new Function("window", src)(win);
  return win;
}

describe("inject.js SHA3-512", () => {
  const { __aitermTest } = loadInject() as { __aitermTest: { sha3_512Hex(s: string): string } };

  it("符合標準測試向量", () => {
    expect(__aitermTest.sha3_512Hex("")).toBe(
      "a69f73cca23a9ac5c8b567dc185a756e97c982164fe25859e0d1dcc1475c80a6" +
      "15b2123af1f5f94c11e3e9402c3ac558f500199d95b6d3e301758586281dcd26",
    );
    expect(__aitermTest.sha3_512Hex("abc")).toBe(
      "b751850b1a57168a5693cd924b6b096e08f621827444f70d884f5d0240d2712e" +
      "10e116e9192af3c91a7ec57647e3934057340b4cf408d5a56592f8274eec53f0",
    );
  });
});
