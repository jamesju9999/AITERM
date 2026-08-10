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
  const win = loadInject();
  const { __aitermTest } = win as { __aitermTest: { sha3_512Hex(s: string): string } };

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

  it("跨多個吸收區塊（rate=72 bytes）也要正確——單一區塊向量測不到分塊/續轉邏輯", () => {
    // 200 bytes → ceil(201/72) = 3 個區塊。值以 Node `crypto.createHash("sha3-512")`
    // 獨立驗證過，見 commit 說明。
    expect(__aitermTest.sha3_512Hex("a".repeat(200))).toBe(
      "eae6c85c6904f11075de9f9d5e1064371d000510fa3d2d79d40cf9be34892fb0" +
      "1859d0a0234e138bcb0ad5c84f6c0dca226a414b0c9a2897cb695f5185fe36ec",
    );
  });

  it("剛好落在區塊邊界（71/72/73 bytes）——padding 最容易寫錯的位置", () => {
    // 同樣以 Node `crypto.createHash("sha3-512")` 獨立產生比對。
    expect(__aitermTest.sha3_512Hex("a".repeat(71))).toBe(
      "070faf98d2a8fddf8ed886408744dc06456096c2e045f26f3c7b010530e6bbb3" +
      "db535a54d636856f4e0e1e982461cb9a7e8e57ff8895cff1619af9f0e486e28c",
    );
    expect(__aitermTest.sha3_512Hex("a".repeat(72))).toBe(
      "a8ae722a78e10cbbc413886c02eb5b369a03f6560084aff566bd597bb7ad8c1c" +
      "cd86e81296852359bf2faddb5153c0a7445722987875e74287adac21adebe952",
    );
    expect(__aitermTest.sha3_512Hex("a".repeat(73))).toBe(
      "23e6a8815f8201dbbf6a5463be8dcadb1acea9df5f8998954e59ac9565cf6d29" +
      "b17aa27a5e8b0fc06343db6122d6e544d27583ddc78504d08203217e7e65b6bd",
    );
  });

  it("__aitermTest 掛載點不可列舉——Task 7 的 buildConfig() 會從 Object.keys(window) 隨機抽一個 key 送給 OpenAI 的 sentinel 端點，可列舉的話這個明顯的自動化標記遲早會被抽中洩漏出去", () => {
    expect(Object.keys(win)).not.toContain("__aitermTest");
    expect(__aitermTest.sha3_512Hex("abc")).toBe(
      "b751850b1a57168a5693cd924b6b096e08f621827444f70d884f5d0240d2712e" +
      "10e116e9192af3c91a7ec57647e3934057340b4cf408d5a56592f8274eec53f0",
    );
  });
});
