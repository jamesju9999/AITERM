import { describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";

/**
 * inject.js 是給瀏覽器用的平鋪 script，直接 import 會找不到 window。這裡把它
 * 讀進來、在一個假的 window 上求值，再取出掛上去的純函式來測。
 *
 * win 可傳入自訂內容——用來讓 pickKey(window) 的抽樣結果變成決定性的（見
 * "pickKey(window) 過濾" 測試）。不傳時預設空物件，行為與原本相同。
 */
function loadInject(win: Record<string, unknown> = {}): Record<string, unknown> {
  const src = readFileSync("src-tauri/src/chatgpt_web/inject.js", "utf8");
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

describe("inject.js PoW", () => {
  const { __aitermTest } = loadInject() as {
    __aitermTest: {
      sha3_512Hex(s: string): string;
      buildConfig(): unknown[];
      solvePow(seed: string, target: string, prefix: string, maxIter: number, deadlineMs?: number):
        { token: string; iters: number; exhausted?: boolean };
    };
  };

  it("解出的 token 前綴正確且雜湊真的落在目標之下", () => {
    const r = __aitermTest.solvePow("seed", "0fffff", "gAAAAAC", 100000);
    expect(r.exhausted).toBeFalsy();
    expect(r.token.startsWith("gAAAAAC")).toBe(true);
    const encoded = r.token.slice("gAAAAAC".length);
    expect(__aitermTest.sha3_512Hex("seed" + encoded).slice(0, 6) <= "0fffff").toBe(true);
  });

  it("超過上限時回 exhausted 而不是無限跑", () => {
    // 目標 "000000" 幾乎不可能命中，用極小的 maxIter 逼出這條路徑。
    const r = __aitermTest.solvePow("seed", "000000", "gAAAAAB", 5);
    expect(r.exhausted).toBe(true);
    expect(r.iters).toBe(5);
    expect(r.token.startsWith("gAAAAAB")).toBe(true);
  });

  it("config 是 18 元素，第 4 格由 solver 改寫", () => {
    const c = __aitermTest.buildConfig();
    expect(c).toHaveLength(18);
    expect(c[3]).toBe(0);
  });

  it("pickKey(window) 過濾掉自動化標記，只剩真實 key 才會被抽中", () => {
    // config[12] 是 pickKey(window) 的結果，會 base64 進 PoW payload、POST 到
    // OpenAI 的 sentinel 端點。fakeWin 裡混入我們自己會掛的 __aiterm* /
    // __TAURI* key（刻意排在 Object.keys 順序最前面）——若過濾邏輯被拿掉，
    // 這兩個都有機會被抽中送出去，等於主動標記自己是自動化。
    //
    // Math.random 鎖定回傳 0：pickKey 內部是 `keys[Math.floor(Math.random() *
    // keys.length)]`，回傳 0 時永遠選 keys[0]。過濾後 keys 只剩 ["realKey"]，
    // 選到的必是它；若過濾被拿掉，keys[0] 會是 "__aitermFake"，斷言必定失敗。
    // 這讓紅/綠都是決定性的，不依賴抽樣運氣。
    const fakeWin: Record<string, unknown> = {
      __aitermFake: 1,
      __TAURI_x: 1,
      realKey: 1,
    };
    const randomSpy = vi.spyOn(Math, "random").mockReturnValue(0);
    try {
      const { __aitermTest: fakeAitermTest } = loadInject(fakeWin) as {
        __aitermTest: { buildConfig(): unknown[] };
      };
      const c = fakeAitermTest.buildConfig();
      expect(c[12]).toBe("realKey");
    } finally {
      randomSpy.mockRestore();
    }
  });

  it("PoW 牆鐘上限會在合理時間內提早停止，不會跑滿 maxIter（防止 webview 主執行緒被凍住數分鐘）", () => {
    const start = Date.now();
    // 目標 "000000" 幾乎不可能命中，maxIter 給極大值，逼牆鐘（而非迭代上限）
    // 成為真正生效的煞車。
    const r = __aitermTest.solvePow("seed", "000000", "gAAAAAB", 10_000_000, 50);
    const elapsed = Date.now() - start;
    expect(r.exhausted).toBe(true);
    expect(r.iters).toBeLessThan(10_000_000);
    // 每 256 次才看一次時鐘，所以至少要跑滿一輪（i=0..255）才可能第一次停下。
    // 這條斷言釘住「每 256 次檢查一次」這個取捨本身。
    expect(r.iters).toBeGreaterThanOrEqual(256);
    expect(elapsed).toBeLessThan(2000);
  });
});
