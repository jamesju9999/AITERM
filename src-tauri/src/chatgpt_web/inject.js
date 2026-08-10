// inject.js — 注入到隱藏 chatgpt.com WebviewWindow 的腳本（ChatGPT Web 供應商，
// 方案 A：把該頁面當傳輸層）。所有對 backend-api 的請求都由這支腳本從頁面內部
// 發出，天然帶有真實瀏覽器的 TLS 指紋與 cookie。
//
// ChatGPT 的 sentinel 反濫用機制要求一個工作量證明（PoW），演算法是
// SHA3-512——但 WebCrypto 沒有提供 SHA3（只有 SHA-1/256/384/512，都是
// Keccak 之前的 NIST 標準），所以這裡自帶一份精簡實作。
//
// 這個檔案有兩個消費者，因此寫成平鋪的 script（不是 ES module）：
//   1. Tauri：以 `.initialization_script(include_str!("inject.js"))` 注入到
//      chatgpt.com 頁面，那裡有真實的全域 `window`。
//   2. vitest：用 `new Function("window", src)(win)` 求值，把整份檔案當成
//      以 window 為參數的函式主體，win 是假的空物件。
//
// IIFE 只用來把中間狀態（RC 表、keccakf 等）關在閉包裡，不對外洩漏。對外只有
// 兩個掛載點：window.__aitermTest（測試用純函式）與 window.__aiterm（Rust 端
// 透過 eval 呼叫的介面，Task 8 才加）。兩者都必須用 Object.defineProperty
// 掛成不可列舉（不寫 enumerable，預設就是 false），理由：
//   - Task 7 的 buildConfig() 會從 Object.keys(window) 隨機挑一個 key 塞進
//     PoW payload，送去 OpenAI 的 sentinel 端點。用一般賦值掛的 __aitermTest
//     是可列舉屬性，累積幾十次請求後幾乎必然被抽中送出去，等於主動告訴
//     OpenAI「這是自動化在操縱這個頁面」。
//   - window.__aiterm 若可列舉又可寫，等於給頁面腳本一個能被覆寫的掛載點；
//     Rust 端會直接 eval("window.__aiterm.pull(id)")，被換掉就是任意程式碼
//     執行點。defineProperty 預設的 writable: false, configurable: false
//     同時擋掉這個風險。
//
// 不變式：這個檔案在載入期不可以有任何副作用（不可啟動 timer、不可發請求），
// 只能定義函式並掛載——測試是在載入時求值的，若之後（如 Task 10 的登入輪詢
// 器）寫成「載入即啟動」，會漏一個 timer 出來讓 vitest 掛住不結束。
(() => {
  // 精簡 SHA3-512（Keccak-f[1600]，rate 72 bytes）。以 32 位元 lo/hi 對表示 64 位元字，
  // 避免 BigInt 的效能與相容性問題。
  function sha3_512Hex(input) {
    const RC = [
      [0x00000001,0x00000000],[0x00008082,0x00000000],[0x0000808a,0x80000000],[0x80008000,0x80000000],
      [0x0000808b,0x00000000],[0x80000001,0x00000000],[0x80008081,0x80000000],[0x00008009,0x80000000],
      [0x0000008a,0x00000000],[0x00000088,0x00000000],[0x80008009,0x00000000],[0x8000000a,0x00000000],
      [0x8000808b,0x00000000],[0x0000008b,0x80000000],[0x00008089,0x80000000],[0x00008003,0x80000000],
      [0x00008002,0x80000000],[0x00000080,0x80000000],[0x0000800a,0x00000000],[0x8000000a,0x80000000],
      [0x80008081,0x80000000],[0x00008080,0x80000000],[0x80000001,0x00000000],[0x80008008,0x80000000],
    ];
    // ρ 的旋轉偏移，以 lane index = x + 5y 排列，共 25 個。
    const ROT = [0,1,62,28,27, 36,44,6,55,20, 3,10,43,25,39, 41,45,15,21,8, 18,2,61,56,14];
    const s = new Array(50).fill(0); // 25 lanes × (lo,hi)

    const rotl = (lo, hi, n) => {
      if (n === 0) return [lo, hi];
      if (n < 32) return [((lo << n) | (hi >>> (32 - n))) >>> 0, ((hi << n) | (lo >>> (32 - n))) >>> 0];
      n -= 32;
      if (n === 0) return [hi, lo];
      return [((hi << n) | (lo >>> (32 - n))) >>> 0, ((lo << n) | (hi >>> (32 - n))) >>> 0];
    };

    function keccakf() {
      const b = new Array(50), c = new Array(10), d = new Array(10);
      for (let round = 0; round < 24; round++) {
        for (let x = 0; x < 5; x++) {
          c[2*x] = s[2*x] ^ s[2*(x+5)] ^ s[2*(x+10)] ^ s[2*(x+15)] ^ s[2*(x+20)];
          c[2*x+1] = s[2*x+1] ^ s[2*(x+5)+1] ^ s[2*(x+10)+1] ^ s[2*(x+15)+1] ^ s[2*(x+20)+1];
        }
        for (let x = 0; x < 5; x++) {
          const [rl, rh] = rotl(c[2*((x+1)%5)], c[2*((x+1)%5)+1], 1);
          d[2*x] = (c[2*((x+4)%5)] ^ rl) >>> 0;
          d[2*x+1] = (c[2*((x+4)%5)+1] ^ rh) >>> 0;
        }
        for (let x = 0; x < 5; x++) for (let y = 0; y < 5; y++) {
          s[2*(x+5*y)] = (s[2*(x+5*y)] ^ d[2*x]) >>> 0;
          s[2*(x+5*y)+1] = (s[2*(x+5*y)+1] ^ d[2*x+1]) >>> 0;
        }
        for (let x = 0; x < 5; x++) for (let y = 0; y < 5; y++) {
          const i = x + 5*y, j = y + 5*((2*x + 3*y) % 5);
          const [rl, rh] = rotl(s[2*i], s[2*i+1], ROT[i]);
          b[2*j] = rl; b[2*j+1] = rh;
        }
        for (let x = 0; x < 5; x++) for (let y = 0; y < 5; y++) {
          const i = x + 5*y;
          s[2*i] = (b[2*i] ^ ((~b[2*(((x+1)%5) + 5*y)]) & b[2*(((x+2)%5) + 5*y)])) >>> 0;
          s[2*i+1] = (b[2*i+1] ^ ((~b[2*(((x+1)%5) + 5*y)+1]) & b[2*(((x+2)%5) + 5*y)+1])) >>> 0;
        }
        s[0] = (s[0] ^ RC[round][0]) >>> 0;
        s[1] = (s[1] ^ RC[round][1]) >>> 0;
      }
    }

    const rate = 72;
    const bytes = typeof input === "string" ? new TextEncoder().encode(input) : input;
    const padded = new Uint8Array(Math.ceil((bytes.length + 1) / rate) * rate);
    padded.set(bytes);
    padded[bytes.length] = 0x06;
    padded[padded.length - 1] |= 0x80;

    for (let off = 0; off < padded.length; off += rate) {
      for (let i = 0; i < rate; i += 8) {
        const idx = i / 8;
        let lo = 0, hi = 0;
        for (let k = 0; k < 4; k++) lo |= padded[off + i + k] << (8 * k);
        for (let k = 0; k < 4; k++) hi |= padded[off + i + 4 + k] << (8 * k);
        s[2*idx] = (s[2*idx] ^ lo) >>> 0;
        s[2*idx+1] = (s[2*idx+1] ^ hi) >>> 0;
      }
      keccakf();
    }

    let out = "";
    for (let i = 0; i < 8; i++) {
      const lo = s[2*i], hi = s[2*i+1];
      for (let k = 0; k < 4; k++) out += ((lo >>> (8*k)) & 0xff).toString(16).padStart(2, "0");
      for (let k = 0; k < 4; k++) out += ((hi >>> (8*k)) & 0xff).toString(16).padStart(2, "0");
    }
    return out;
  }

  // 從 Object.keys 隨機取一個 key，空物件回空字串。
  //
  // 過濾 `__aiterm` / `__TAURI` 開頭：抽到的名字會 base64 進 config、POST 到
  // OpenAI 的 sentinel 端點。把自己的掛載點名稱遞交過去等於主動標記自己是
  // 自動化。掛載點本身已經是不可列舉的（見檔頭註解），這是第二道防線——
  // Tauri 的 `__TAURI_INTERNALS__` 也是不可列舉的，但別把正確性建立在第三方
  // 的實作細節上。
  const pickKey = (obj) => {
    const keys = Object.keys(obj).filter(
      (k) => !k.startsWith("__aiterm") && !k.startsWith("__TAURI")
    );
    return keys.length ? keys[Math.floor(Math.random() * keys.length)] : "";
  };

  const uuid = () => crypto.randomUUID();

  // config 需要的瀏覽器特徵在頁面內全是真值。OmniRoute 跑在伺服器上必須捏造
  // （假螢幕尺寸、假核心數、從硬編清單隨機挑 key），我們送出的指紋則與 OpenAI
  // 看到的其他一切一致——這是把 webview 當傳輸層的額外好處。
  const buildConfig = () => {
    const dplAttr = document.documentElement.getAttribute("data-build");
    const script = document.querySelector('script[src*=".js"]');
    const perfNow = performance.now();
    const nav = navigator;
    return [
      screen.width + screen.height,
      new Date().toString(),
      4294705152,
      0, // solver 改寫這一格
      nav.userAgent,
      script ? script.src : "",
      dplAttr ? "dpl=" + dplAttr : "",
      nav.language,
      (nav.languages || []).join(","),
      0,
      pickKey(nav),
      pickKey(document),
      pickKey(window),
      perfNow,
      uuid(),
      "",
      nav.hardwareConcurrency,
      Date.now() - perfNow,
    ];
  };

  const b64 = (obj) => btoa(unescape(encodeURIComponent(JSON.stringify(obj))));

  // 同步 PoW 迴圈跑在 webview 主執行緒上，所以要有牆鐘上限而不只是迭代上限。
  //
  // 實測單次雜湊約 0.1–0.17ms（Node，900 bytes 輸入），WebKit 的位元運算迴圈
  // 通常再慢 2–4 倍。純用迭代數當上限，等於把「視窗要凍多久」交給上游決定：
  // difficulty 每多一個十六進位位數，期望迭代數就乘以 16。
  const POW_DEADLINE_MS = 15000;

  // 把 config[3] 換成遞增計數器，算 SHA3-512(seed + base64(JSON(config)))，
  // 取十六進位前綴與 difficulty 做字串比較。實測 difficulty 是 "06b931" 這種
  // 6 位值，命中機率約 2.6%，平均數十次即可。
  const solvePow = (seed, target, prefix, maxIter) => {
    const cfg = buildConfig();
    const deadline = Date.now() + POW_DEADLINE_MS;
    for (let i = 0; i < maxIter; i++) {
      // 每 256 次才看一次時鐘：Date.now() 本身不便宜，而 256 次的誤差
      // （最壞約 100ms）遠小於 15 秒的預算。
      if ((i & 255) === 255 && Date.now() > deadline) {
        return { token: prefix + b64(cfg), iters: i + 1, exhausted: true };
      }
      cfg[3] = i;
      const enc = b64(cfg);
      if (sha3_512Hex(seed + enc).slice(0, target.length) <= target) {
        return { token: prefix + enc, iters: i + 1 };
      }
    }
    return { token: prefix + b64(cfg), iters: maxIter, exhausted: true };
  };

  // 不可列舉：見檔頭說明，避免被 Task 7 的 buildConfig() 隨機抽中送給 OpenAI。
  Object.defineProperty(window, "__aitermTest", {
    value: { sha3_512Hex, buildConfig, solvePow },
  });
})();
