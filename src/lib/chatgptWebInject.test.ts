import { describe, expect, it, vi } from "vitest";
// `?raw` 是 Vite 原生的原文匯入，型別由 vite/client 提供。
//
// 刻意不用 `node:fs`：`tsconfig.app.json` 的 `types` 只有 `["vite/client"]`，
// 而它 `include` 整個 `src`——測試檔也在型別檢查範圍內。為了一個測試把 `node`
// 加進去，會讓所有前端程式碼都能通過「誤用 Node API」的型別檢查，而那類錯誤
// 要到執行時才會炸。`?raw` 同時解掉「路徑相對於 cwd」的隱性依賴。
import injectSource from "../../src-tauri/src/chatgpt_web/inject.js?raw";

/**
 * inject.js 是給瀏覽器用的平鋪 script，直接 import 會找不到 window。這裡把它
 * 在一個假的 window 上求值，再取出掛上去的純函式來測。
 *
 * win 可傳入自訂內容——用來讓 pickKey(window) 的抽樣結果變成決定性的（見
 * "pickKey(window) 過濾" 測試）。不傳時預設空物件，行為與原本相同。
 */
function loadInject(win: Record<string, unknown> = {}): Record<string, unknown> {
  new Function("window", injectSource)(win);
  return win;
}

describe("inject.js 載入期無副作用", () => {
  it("求值時不啟動任何 timer", () => {
    // 檔頭不變式：載入期只能定義函式並掛載，不可啟動 timer、不可發請求。這條
    // 是專門守這件事的——傳進去的假 window 是 {}，若腳本裡寫了
    // setTimeout(...)，那會解析到 jsdom 的「全域」setTimeout 而不是這個假
    // window（因為裸的 setTimeout 不會走 window 參數），不會拋錯，只會安靜地
    // 漏一個 timer 出來，讓 vitest 掛住不結束（Task 10 的登入輪詢器如果寫成
    // 「載入即啟動」就會踩到這個）。
    vi.useFakeTimers();
    try {
      loadInject();
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });
});

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

  it("__aitermTest 掛載點不可列舉——buildConfig() 會隨機抽一個 window 屬性名送給 OpenAI 的 sentinel 端點，可列舉的話這個明顯的自動化標記遲早會被抽中洩漏出去", () => {
    expect(Object.keys(win)).not.toContain("__aitermTest");
    // pickKey 實際用的是 for...in，所以要鎖住的是**那個**列舉方式看不到它。
    // 只斷言 Object.keys 的話，測試守的機制跟程式碼用的機制就對不上了。
    const viaForIn: string[] = [];
    for (const k in win) viaForIn.push(k);
    expect(viaForIn, "for...in 也不可以看到掛載點").not.toContain("__aitermTest");
    expect(__aitermTest.sha3_512Hex("abc")).toBe(
      "b751850b1a57168a5693cd924b6b096e08f621827444f70d884f5d0240d2712e" +
      "10e116e9192af3c91a7ec57647e3934057340b4cf408d5a56592f8274eec53f0",
    );
  });
});

describe("inject.js 對 Rust 的介面", () => {
  const win: Record<string, unknown> = {};
  loadInject(win);

  /**
   * Rust 端是 `w.eval("window.__aiterm.pull(id)")`。這個掛載點名稱或形狀一改，
   * 那行 eval 就會靜默失敗——沒有編譯器、沒有型別檢查會發現，只會表現成
   * 「請求送出去但永遠沒有回應」。這條測試是唯一會叫的地方。
   */
  it("__aiterm.pull 與 __aiterm.watchLogin 存在且是函式", () => {
    const aiterm = win.__aiterm as { pull?: unknown; watchLogin?: unknown } | undefined;
    expect(aiterm, "Rust 端 eval 的掛載點不存在").toBeDefined();
    expect(typeof aiterm?.pull, "pull 不是函式，eval 會靜默失敗").toBe("function");
    // ensure_window 的 visible 分支與 on_page_load 都 eval 這個名字。
    expect(typeof aiterm?.watchLogin, "watchLogin 不是函式，登入後不會自動收起視窗").toBe(
      "function",
    );
  });

  /**
   * `Session::request_raw("models")` eval 的是 `window.__aiterm.models(id)`。
   * 這個名字改掉，設定頁的模型下拉會空著並在 30 秒後逾時——而逾時訊息不會
   * 指向真正的原因。
   */
  it("__aiterm.models 存在且是函式", () => {
    const aiterm = win.__aiterm as { models?: unknown } | undefined;
    expect(typeof aiterm?.models, "models 不是函式，模型清單會逾時").toBe("function");
  });

  /**
   * 理由同 __aitermTest：可列舉的掛載點會被 pickKey(window) 抽中送給 OpenAI。
   * 另外 writable:false 擋掉頁面腳本覆寫——Rust 直接 eval 這個名字，被換掉
   * 就是一個任意程式碼執行點。
   */
  it("__aiterm 不可列舉且不可覆寫", () => {
    expect(Object.keys(win)).not.toContain("__aiterm");
    const viaForIn: string[] = [];
    for (const k in win) viaForIn.push(k);
    expect(viaForIn, "for...in 也不可以看到掛載點").not.toContain("__aiterm");
    const d = Object.getOwnPropertyDescriptor(win, "__aiterm");
    expect(d?.enumerable).toBe(false);
    expect(d?.writable).toBe(false);
    expect(d?.configurable).toBe(false);
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
    const actualPrefix = __aitermTest.sha3_512Hex("seed" + encoded).slice(0, 6);
    expect(
      actualPrefix <= "0fffff",
      `hash prefix ${actualPrefix} 應該 <= target 0fffff`,
    ).toBe(true);
  });

  it("超過上限時回 exhausted 而不是無限跑", () => {
    // 目標 "000000" 幾乎不可能命中，用極小的 maxIter 逼出這條路徑。
    const r = __aitermTest.solvePow("seed", "000000", "gAAAAAB", 5);
    expect(r.exhausted).toBe(true);
    expect(r.iters).toBe(5);
    expect(r.token.startsWith("gAAAAAB")).toBe(true);
  });

  it("config 各格對應真實瀏覽器特徵，不能悄悄換掉來源——這是送給 OpenAI 的指紋，某一格換了來源不會讓 PoW 失敗（雜湊照樣算得出來），只會讓指紋跟同一個 session 的其他訊號兜不起來，只有反濫用系統看得到", () => {
    const c = __aitermTest.buildConfig();
    expect(c).toHaveLength(18);
    // 可在 jsdom 下決定性比對的格子。
    expect(c[0]).toBe(screen.width + screen.height);
    expect(c[2]).toBe(4294705152);
    expect(c[3]).toBe(0);
    expect(c[4]).toBe(navigator.userAgent);
    expect(c[7]).toBe(navigator.language);
    expect(c[8]).toBe(navigator.languages.join(","));
    expect(c[9]).toBe(0);
    expect(c[16]).toBe(navigator.hardwareConcurrency);
    // 其餘幾格（Date().toString()、script src、dpl、pickKey 三格、perfNow、
    // uuid、固定空字串、Date.now()-perfNow）沒有可決定性比對的基準值，退而
    // 求其次驗證型別——至少擋住「整格被拿掉或悄悄換成 undefined」。
    expect(typeof c[1]).toBe("string");
    expect(typeof c[5]).toBe("string");
    expect(typeof c[6]).toBe("string");
    expect(typeof c[10]).toBe("string");
    expect(typeof c[11]).toBe("string");
    expect(typeof c[12]).toBe("string");
    expect(typeof c[13]).toBe("number");
    expect(typeof c[14]).toBe("string");
    expect(typeof c[15]).toBe("string");
    expect(typeof c[17]).toBe("number");
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

  it("省略 deadlineMs 時，預設的 15 秒牆鐘上限真的有生效——前幾個測試都明確傳了 deadlineMs=50，繞過了預設值，這條補上「不傳參數」這條路徑", () => {
    const t0 = 1_000_000;
    let calls = 0;
    // solvePow 的牆鐘用 performance.now()（單調時鐘，不是可被系統時鐘調整
    // 跳動的 Date.now()——見 inject.js 的說明）。buildConfig() 內部也會呼叫
    // 一次 performance.now()（算 perfNow，跟牆鐘無關），所以前兩次呼叫
    // （buildConfig 的 perfNow、solvePow 算 deadline）回 t0，之後（迴圈內
    // i=255 才第一次檢查）回 t0+20000——超過預設的 15000ms 但小於 30000ms，
    // 藉此把「15 秒」這個數量級也釘住：若有人把 POW_DEADLINE_MS 改成
    // 30000，20000 就不會超時，這條測試會紅。
    //
    // 只 mock performance.now，不動 Date.now：buildConfig() 最後一格仍會呼叫
    // Date.now()（算 timeOrigin），但那跟牆鐘計算是兩支互不相干的時鐘，不會
    // 再互相耦合——之前用 Date.now 時得靠「迴圈前剛好被呼叫兩次」分段，
    // buildConfig 若多呼叫一次 Date.now() 就會讓那個假設靜默失效。
    vi.spyOn(performance, "now").mockImplementation(() => {
      calls++;
      return calls <= 2 ? t0 : t0 + 20000;
    });
    try {
      const bigMaxIter = 10_000_000;
      // 用未被 mock 的 Date.now() 量測真實耗時：若實作偷偷退回用 Date.now()
      // 當牆鐘，這裡 mock 的 performance.now() 就不會生效，迴圈會在真實時間
      // 裡老實地跑到真的超過 15 秒才停（exhausted/iters 斷言意外還是會過，
      // 只是變慢）——這條 elapsed 斷言才是真正把「有沒有吃到 mock」釘住的。
      const startReal = Date.now();
      // 不傳 deadlineMs，逼出預設值那條路徑。
      const r = __aitermTest.solvePow("seed", "000000", "gAAAAAB", bigMaxIter);
      const elapsedReal = Date.now() - startReal;
      expect(r.exhausted).toBe(true);
      expect(r.iters).toBeLessThan(bigMaxIter);
      expect(r.iters).toBeGreaterThanOrEqual(256);
      expect(elapsedReal).toBeLessThan(2000);
    } finally {
      vi.restoreAllMocks();
    }
  });

  it("deadlineMs 傳 0 代表沒有時間預算、立刻放棄——0 是合法值，用 ?? 才會保留它，用 || 會把它當成「沒傳」而靜默退回 15 秒預設", () => {
    const r = __aitermTest.solvePow("seed", "000000", "gAAAAAB", 10_000_000, 0);
    expect(r.exhausted).toBe(true);
    // deadline = Date.now()+0，第一個檢查點（i=255）當下真實時鐘必定已經過了
    // deadline，所以在 256 次迭代內就會停。
    expect(r.iters).toBe(256);
  });

  it("pickKey 用字首（startsWith）過濾，不是子字串（includes）——內含 __aiterm 但不是開頭的 key 應該被保留", () => {
    // "x__aitermY" 不是我們的掛載點（開頭是 x），是一個普通的真實瀏覽器 key，
    // 理論上應該被保留、有機會被抽中送給 OpenAI 當指紋的一部分。若過濾邏輯
    // 誤用 includes，這種 key 會被誤判成自動化標記而濾掉，導致合法的瀏覽器
    // 特徵被排除、pickKey 落回空字串。fixture 只放這一個 key，過濾後候選數量
    // （1 或 0）本身就決定了結果，不需要另外鎖 Math.random。
    const fakeWin: Record<string, unknown> = { x__aitermY: 1 };
    const { __aitermTest: fakeAitermTest } = loadInject(fakeWin) as {
      __aitermTest: { buildConfig(): unknown[] };
    };
    const c = fakeAitermTest.buildConfig();
    expect(c[12]).toBe("x__aitermY");
  });
});
