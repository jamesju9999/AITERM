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
//     OpenAI「這是自動化在操縱這個頁面」。這道防線只擋得住 Object.keys 這類
//     走可列舉屬性的列舉方式——指紋腳本常見的
//     Object.getOwnPropertyNames(window) 一樣會列出不可列舉屬性，所以
//     __aitermTest 在正式版仍是可被那種手法發現的表面；不可列舉不是「隱藏」
//     的保證。
//   - window.__aiterm 若可列舉又可寫，等於給頁面腳本一個能被覆寫的掛載點；
//     Rust 端會直接 eval("window.__aiterm.pull(id)")，被換掉就是任意程式碼
//     執行點。defineProperty 預設的 writable: false, configurable: false
//     同時擋掉這個風險（這點與是否可列舉無關）。
//
// 不變式：這個檔案在載入期不可以有任何副作用（不可啟動 timer、不可發請求），
// 只能定義函式並掛載——測試是在載入時求值的，若之後（如 Task 10 的登入輪詢
// 器）寫成「載入即啟動」，會漏一個 timer 出來讓 vitest 掛住不結束。
(() => {
  // Tauri IPC 的入口。**不是全域**：tauri.conf.json 沒有設 withGlobalTauri
  // （預設 false），所以頁面上沒有 window.__TAURI__，真正存在的是
  // window.__TAURI_INTERNALS__.invoke。
  //
  // 必須寫成惰性的箭頭函式，不可寫成 `const invoke = window.__TAURI_INTERNALS__.invoke;`
  // ——vitest 裡 window 是 `{}`，那行會在載入期 TypeError，把所有純函式測試
  // 一起炸掉，而且錯誤會指向掛載失敗、不是指向這裡。
  const invoke = (cmd, args) => window.__TAURI_INTERNALS__.invoke(cmd, args);

  // Tauri IPC 的 rejection 值常常是序列化後的物件，String(e) 會得到
  // "[object Object]"，把真正的原因整個吃掉。
  const errText = (e) =>
    e instanceof Error
      ? e.message
      : typeof e === "string"
        ? e
        : (() => {
            try {
              return JSON.stringify(e);
            } catch {
              return String(e);
            }
          })();

  // 這是唯一把失敗告知 Rust 的管道。它自己靜默拋出就等於請求永久掛住
  // （Rust 端既收不到 chunk 也收不到 error，只會一直等），所以要吞掉自身例外。
  const reportError = (id, e) => {
    try {
      invoke("chatgpt_web_chunk", { id, data: JSON.stringify({ error: errText(e) }) });
    } catch {
      /* IPC 都不通了，沒有別的管道可用。 */
    }
  };

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
  // 自動化。掛載點本身對這裡用的 Object.keys 是不可列舉的（見檔頭註解），這
  // 是第二道防線——但只防得住 Object.keys 這種列舉方式，不是通用的隱藏保證；
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
      nav.languages.join(","),
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
  const solvePow = (seed, target, prefix, maxIter, deadlineMs) => {
    const cfg = buildConfig();
    // 用 performance.now()（單調時鐘）而不是 Date.now()（牆鐘，可被系統時鐘
    // 調整跳動）。筆電睡醒、VM 還原快照時 NTP 可能把系統時鐘往回校正幾秒到
    // 幾分鐘，若剛好落在這個迴圈中，用 Date.now() 算出的 deadline 會變成
    // 「未來」，15 秒煞車完全失效，同步迴圈就會照 maxIter 跑到底——這正是這
    // 個上限想防的事。buildConfig() 裡的 Date.now() - perfNow 是在算
    // timeOrigin，那本來就要用牆鐘，不受影響。
    const deadline = performance.now() + (deadlineMs ?? POW_DEADLINE_MS);
    for (let i = 0; i < maxIter; i++) {
      // 每 256 次才看一次時鐘：performance.now() 本身不便宜，而 256 次的誤差
      // （最壞約 100ms）遠小於 15 秒的預算。
      if ((i & 255) === 255 && performance.now() > deadline) {
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

  // backend-api 認的是 Bearer access token，不是 cookie。少了它，sentinel 回應
  // 的 persona 會是 "chatgpt-noauth"，對話請求則是 403 "Unusual activity has
  // been detected from your device"（實測）。
  let accessToken = null;

  // `force` 用在 401 之後重取。**沒有它會永久卡死**：token 有有效期
  // （/api/auth/session 自己就回 expires），而 AITerm 是終端機 App、開一整天
  // 是常態。過期後 `if (accessToken) return` 讓這個函式永遠不重查，所有請求
  // 一路 401 到使用者重啟整個 App 為止——而隱藏的 ChatGPT 頁面其實還登著，
  // 使用者完全沒有線索。
  const ensureAuth = async (force) => {
    if (accessToken && !force) return accessToken;
    const r = await fetch("/api/auth/session", { headers: { accept: "application/json" } });
    const j = await r.json().catch(() => ({}));
    accessToken = j.accessToken || null;
    return accessToken;
  };

  const authHeaders = () => ({
    "content-type": "application/json",
    ...(accessToken ? { authorization: "Bearer " + accessToken } : {}),
  });

  // 帶 Bearer 的 POST，遇 401 就重取一次 token 再試。
  // 只重試一次：第二次仍 401 表示真的沒登入（或帳號被登出），再重試只是拖延
  // 錯誤回報。
  const postAuthed = async (url, body) => {
    const send = () =>
      fetch(url, { method: "POST", headers: authHeaders(), body: JSON.stringify(body) });
    let r = await send();
    if (r.status === 401) {
      await ensureAuth(true);
      r = await send();
    }
    return r;
  };

  // 解 PoW，超時就丟出可辨識的錯，而不是把解不出來的 token 送出去。
  //
  // solvePow 超時時仍會回一個 token，但那個 token **保證不符合 difficulty**。
  // 照送的話伺服器回 403「Unusual activity has been detected from your device」
  // ——使用者看到的是與真正原因（PoW 超時）毫無關聯的字串，而且每次請求都會
  // 重犯；反覆遞交無效的工作量證明本身也是反濫用系統會記的行為。
  const powToken = (seed, target, prefix, maxIter) => {
    const r = solvePow(seed, target, prefix, maxIter);
    if (r.exhausted) {
      throw new Error("pow_timeout: 工作量證明超時（difficulty=" + target + "）");
    }
    return r.token;
  };

  // 兩段 chat-requirements：prepare 拿 prepare_token，再換取對話用的 token 與
  // proofofwork 參數。兩段的 p 都是解過的 PoW token（prepare 階段 seed 為空、
  // target 固定 "0fffff"）。
  const sentinel = async () => {
    const prep = await postAuthed("/backend-api/sentinel/chat-requirements/prepare", {
      p: powToken("", "0fffff", "gAAAAAC", 100000),
    });
    if (!prep.ok) throw new Error("sentinel prepare " + prep.status);
    const prepJson = await prep.json();

    const cr = await postAuthed("/backend-api/sentinel/chat-requirements", {
      p: powToken("", "0fffff", "gAAAAAC", 100000),
      prepare_token: prepJson.prepare_token,
    });
    if (!cr.ok) throw new Error("sentinel chat-requirements " + cr.status);
    const crJson = await cr.json();
    return { ...crJson, prepare_token: prepJson.prepare_token };
  };

  const run = async (id, payload) => {
    if (!(await ensureAuth())) throw new Error("not_logged_in");
    const reqs = await sentinel();
    const pow = reqs.proofofwork || {};
    const headers = { ...authHeaders(), accept: "text/event-stream" };
    if (reqs.token) headers["openai-sentinel-chat-requirements-token"] = reqs.token;
    if (reqs.prepare_token) {
      headers["openai-sentinel-chat-requirements-prepare-token"] = reqs.prepare_token;
    }
    headers["openai-sentinel-proof-token"] = powToken(
      pow.seed || "",
      (pow.difficulty || "").toLowerCase(),
      "gAAAAAB",
      500000
    );

    const r = await fetch("/backend-api/conversation", {
      method: "POST",
      headers,
      body: JSON.stringify({
        action: "next",
        messages: [
          {
            id: uuid(),
            author: { role: "user" },
            content: { content_type: "text", parts: [payload.text] },
          },
        ],
        model: payload.model,
        parent_message_id: uuid(),
        websocket_request_id: uuid(),
        conversation_mode: { kind: "primary_assistant" },
      }),
    });
    if (!r.ok || !r.body) {
      const body = await r.text();
      invoke("chatgpt_web_chunk", { id, data: JSON.stringify({ error: body, status: r.status }) });
      return;
    }
    const reader = r.body.getReader();
    const dec = new TextDecoder();
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      // 送原始 chunk，不在這裡切行——HTTP chunk 不保證切在 \n 上，Rust 端的
      // SseParser 自己有行緩衝。
      invoke("chatgpt_web_chunk", { id, data: dec.decode(value, { stream: true }) });
    }
    // 結尾一定要帶換行：SseParser.feed_str 只處理完整的行，沒有 \n 的殘缺尾巴
    // 會永遠留在緩衝裡，Rust 端就等不到結束訊號。
    invoke("chatgpt_web_chunk", { id, data: "data: [DONE]\n\n" });
  };

  // 顯示視窗期間輪詢登入狀態。拿到 token 就通知 Rust 收起視窗——不做這件事
  // 視窗會一直開著，使用者很自然會去關掉它，之後每次請求都要多付一次載入成本。
  //
  // 這個函式**不可以在載入期自動啟動**：測試是在載入時求值的，載入即啟動的
  // timer 會漏出來讓 vitest 不結束（檔頭的不變式）。只能由 Rust 端 eval 觸發。
  let loginWatcher = null;
  const watchLogin = () => {
    // 去重：ensure_window(true) 每次被呼叫都會 eval 一次，沒有這道防護就會疊出
    // 多個並行輪詢器，每個都在打 /api/auth/session。
    if (loginWatcher) return;
    // 上限：使用者始終不登入時（例如 Google SSO 在內嵌 webview 裡走不完），
    // 輪詢不該永遠跑下去。
    const deadline = Date.now() + 10 * 60 * 1000;
    const tick = async () => {
      loginWatcher = null;
      accessToken = null; // 強制重新查，別用登入前的快取值
      try {
        if (await ensureAuth()) {
          invoke("chatgpt_web_logged_in", {});
          return;
        }
      } catch {
        /* 網路暫時不通就繼續等下一輪。 */
      }
      if (Date.now() < deadline) {
        loginWatcher = setTimeout(tick, 2000);
      }
    };
    loginWatcher = setTimeout(tick, 0);
  };

  // Rust 端只送 id，payload 由這裡反向拉取——Claude Code 的 system prompt 動輒
  // 30K 字元，用 eval 拼進 JS 字串會踩上跳脫與長度限制。
  //
  // Task 10 會往這個物件加 watchLogin，所以宣告成具名物件：defineProperty 預設
  // writable: false，不能重新定義同一個屬性，只能往裡面加 key。
  const aiterm = {
    pull: async (id) => {
      try {
        // chatgpt_web_take 回的是 Option<String>——Rust 端存進 pending map 的是
        // build_payload(...).to_string()，也就是一份 **JSON 字串**。直接拿去存取
        // .text 會得到 undefined，而 JSON.stringify 會把陣列裡的 undefined 轉成
        // null，於是送給上游的是 parts: [null]——請求不會報錯，只會得到莫名其妙
        // 的回覆。
        const raw = await invoke("chatgpt_web_take", { id });
        if (raw == null) throw new Error("payload_missing: " + id);
        await run(id, JSON.parse(raw));
      } catch (e) {
        reportError(id, e);
      }
    },
    watchLogin,
  };
  Object.defineProperty(window, "__aiterm", { value: aiterm });

  // 不可列舉：見檔頭說明，避免被 Task 7 的 buildConfig() 隨機抽中送給 OpenAI。
  Object.defineProperty(window, "__aitermTest", {
    value: { sha3_512Hex, buildConfig, solvePow },
  });
})();
