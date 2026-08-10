//! ⚠️ 臨時可行性探勘（ChatGPT Web 供應商，方案 A：把 webview 當傳輸層）。
//!
//! 查完整個檔案刪除，並移除 lib.rs 的 mod／setup 呼叫／command 註冊，以及
//! capabilities/probe-chatgpt.json。設 `AITERM_PROBE_CHATGPT=1` 才啟用。
//!
//! 三個問題（前兩個已實測通過）：
//!   1. initialization_script 能不能注入到 chatgpt.com —— 可以，CSP 不擋腳本
//!   2. 從該頁面內 fetch backend-api 會不會被 Cloudflare 挑戰 —— 不會
//!      （STATUS 200 / cf-mitigated null；同一支請求從外部 client 打是 403 challenge）
//!   3. chunk 能不能穩定流出到 Rust、速率夠不夠 agent loop 用 —— 本輪要測
//!
//! 回報通道走 Tauri IPC 而不是 localhost HTTP：實測 chatgpt.com 的 CSP
//! `connect-src` 會擋掉往 127.0.0.1 的 fetch，而 IPC 底層是
//! webkit.messageHandlers / chrome.webview.postMessage，不受 connect-src 管轄。
//! 這也是真實架構會用的路徑。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

use tauri::{AppHandle, WebviewUrl, WebviewWindowBuilder};

static CHUNKS: AtomicU64 = AtomicU64::new(0);
static FIRST_CHUNK_MS: AtomicU64 = AtomicU64::new(0);
static STREAM_START_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    static T0: OnceLock<Instant> = OnceLock::new();
    T0.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// 注入到 chatgpt.com 的腳本。只掛工具、不自動發請求——登入流程要讓使用者自己走完。
fn injected_script() -> String {
    r#"
(() => {
  const send = (payload) => {
    try {
      window.__TAURI_INTERNALS__.invoke("probe_report", { payload: JSON.stringify(payload) });
    } catch (e) {
      console.error("[aiterm-probe] IPC 失敗", e);
    }
  };
  const ipcReady = typeof window.__TAURI_INTERNALS__?.invoke === "function";

  send({ kind: "injected", url: location.href });

  // ── Sentinel（OpenAI 自己的反濫用層，與 Cloudflare 無關）─────────────────
  // 實測直接打 /conversation 會拿到 403 "Unusual activity has been detected
  // from your device"。真實前端要先走兩段 chat-requirements 拿 token，再附上
  // 一個 SHA3-512 工作量證明。演算法出處見 openai-sentinel / chat2api。
  //
  // OmniRoute 因為跑在伺服器上，config 裡的瀏覽器特徵全是捏造的（假螢幕尺寸、
  // 假核心數、從硬編清單隨機挑 key）。在頁面內這些都拿得到真的——這正是方案 A
  // 的額外優勢，送出去的指紋跟 OpenAI 看到的其他一切一致。
  const buildConfig = () => {
    const dplAttr = document.documentElement.getAttribute("data-build");
    const script = document.querySelector('script[src*=".js"]');
    const perfNow = performance.now();
    return [
      screen.width + screen.height,
      new Date().toString(),
      4294705152,
      0,                                   // solver 會改這一格
      navigator.userAgent,
      script ? script.src : "",
      dplAttr ? "dpl=" + dplAttr : "",
      navigator.language,
      navigator.languages.join(","),
      0,
      Object.keys(navigator)[Math.floor(Math.random() * Object.keys(navigator).length)],
      Object.keys(document)[Math.floor(Math.random() * Object.keys(document).length)],
      Object.keys(window)[Math.floor(Math.random() * Object.keys(window).length)],
      perfNow,
      crypto.randomUUID(),
      "",
      navigator.hardwareConcurrency,
      Date.now() - perfNow,
    ];
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

  // PoW：把 config[3] 換成遞增計數器，算 SHA3-512(seed + base64(JSON(config)))，
  // 取十六進位前綴與 difficulty 比大小。難度 06b931 約 2.6% 命中率，平均 38 次。
  const b64 = (obj) => btoa(unescape(encodeURIComponent(JSON.stringify(obj))));
  const solvePow = (seed, target, prefix, maxIter) => {
    const cfg = buildConfig();
    const t0 = performance.now();
    for (let i = 0; i < maxIter; i++) {
      cfg[3] = i;
      const enc = b64(cfg);
      if (sha3_512Hex(seed + enc).slice(0, target.length) <= target) {
        return { token: prefix + enc, iters: i + 1, ms: Math.round(performance.now() - t0) };
      }
    }
    return { token: prefix + b64(cfg), iters: maxIter, ms: Math.round(performance.now() - t0),
             exhausted: true };
  };

  // 先送未解的 token——OmniRoute 註解說低摩擦帳號吃得下。過得了就不必實作
  // SHA3-512；過不了再補 solver。
  const unsolved = (prefix) => prefix + btoa(unescape(encodeURIComponent(JSON.stringify(buildConfig()))));

  // backend-api 認的是 Bearer access token，不是 cookie。真實前端先用 cookie
  // 打 /api/auth/session 換出 accessToken 再帶上——少了它，sentinel 會回
  // persona: "chatgpt-noauth"，對話請求則是 403 "Unusual activity"（實測）。
  window.probeAuth = async () => {
    const r = await fetch("/api/auth/session", { headers: { accept: "application/json" } });
    const j = await r.json().catch(() => ({}));
    window.__probeToken = j.accessToken || null;
    send({ kind: "auth", status: r.status, hasToken: !!j.accessToken,
           user: j.user ? j.user.email : null, plan: j.accountPlan || null });
    return !!j.accessToken;
  };

  const authHeaders = () => {
    const h = { "content-type": "application/json" };
    if (window.__probeToken) h["authorization"] = "Bearer " + window.__probeToken;
    return h;
  };

  window.probeSentinel = async () => {
    if (!window.__probeToken) await window.probeAuth();
    const post = (url, body) =>
      fetch(url, { method: "POST", headers: authHeaders(), body: JSON.stringify(body) });

    const prep = await post("/backend-api/sentinel/chat-requirements/prepare",
                            { p: solvePow("", "0fffff", "gAAAAAC", 100000).token });
    const prepText = await prep.text();
    send({ kind: "sentinelPrepare", status: prep.status, bodyHead: prepText.slice(0, 300) });
    if (!prep.ok) return prep.status;

    const prepJson = JSON.parse(prepText || "{}");
    const cr = await post("/backend-api/sentinel/chat-requirements",
                          { p: solvePow("", "0fffff", "gAAAAAC", 100000).token, prepare_token: prepJson.prepare_token });
    const crText = await cr.text();
    send({ kind: "sentinelRequirements", status: cr.status, bodyHead: crText.slice(0, 400) });
    if (!cr.ok) return cr.status;

    const crJson = JSON.parse(crText || "{}");
    window.__probeReqs = { ...crJson, prepare_token: prepJson.prepare_token };
    send({ kind: "sentinelPow", persona: crJson.persona,
           pow: crJson.proofofwork ? { required: crJson.proofofwork.required,
                                       difficulty: crJson.proofofwork.difficulty,
                                       seedLen: (crJson.proofofwork.seed || "").length } : null });
    return cr.status;
  };

  /** 問題 3：帶著 sentinel token 發對話請求，逐 chunk 回報以量測速率。 */
  window.probeStream = async (prompt = "count from 1 to 40, one number per line") => {
    const reqs = window.__probeReqs;
    if (!reqs) { console.error("先執行 await probeSentinel()"); return; }
    const headers = { ...authHeaders(), accept: "text/event-stream" };
    if (reqs.token) headers["openai-sentinel-chat-requirements-token"] = reqs.token;
    if (reqs.prepare_token) headers["openai-sentinel-chat-requirements-prepare-token"] = reqs.prepare_token;
    const pow = reqs.proofofwork || {};
    const solved = solvePow(pow.seed || "", (pow.difficulty || "").toLowerCase(), "gAAAAAB", 500000);
    send({ kind: "powSolved", iters: solved.iters, ms: solved.ms, exhausted: !!solved.exhausted });
    headers["openai-sentinel-proof-token"] = solved.token;

    send({ kind: "streamStart" });
    const r = await fetch("/backend-api/conversation", {
      method: "POST",
      headers,
      body: JSON.stringify({
        action: "next",
        messages: [{ id: crypto.randomUUID(), author: { role: "user" },
                     content: { content_type: "text", parts: [prompt] } }],
        model: "auto",
        parent_message_id: crypto.randomUUID(),
        websocket_request_id: crypto.randomUUID(),
        conversation_mode: { kind: "primary_assistant" },
      }),
    });
    send({ kind: "streamHead", status: r.status, cf: r.headers.get("cf-mitigated") });
    if (!r.ok || !r.body) {
      send({ kind: "streamFail", bodyHead: (await r.text()).slice(0, 300) });
      return r.status;
    }
    const reader = r.body.getReader();
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      send({ kind: "chunk", bytes: value.byteLength });
    }
    send({ kind: "streamEnd" });
    return r.status;
  };

  console.log("[aiterm-probe] ready — IPC 可用 =", ipcReady, "｜依序 probeAuth() → probeSentinel() → probeStream()");
})();
"#
    .to_string()
}

/// 探勘用的回報端點。真實架構不會長這樣（會是串流通道），這裡只為量測。
#[tauri::command]
pub fn probe_report(payload: String) {
    let t = now_ms();
    if payload.contains("\"kind\":\"chunk\"") {
        let n = CHUNKS.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = FIRST_CHUNK_MS.compare_exchange(0, t, Ordering::Relaxed, Ordering::Relaxed);
        if n <= 3 || n % 25 == 0 {
            log::info!("[probe] chunk #{n} @ {t}ms  {payload}");
        }
        return;
    }
    if payload.contains("\"kind\":\"streamStart\"") {
        CHUNKS.store(0, Ordering::Relaxed);
        FIRST_CHUNK_MS.store(0, Ordering::Relaxed);
        STREAM_START_MS.store(t, Ordering::Relaxed);
    }
    if payload.contains("\"kind\":\"streamEnd\"") {
        let start = STREAM_START_MS.load(Ordering::Relaxed);
        let first = FIRST_CHUNK_MS.load(Ordering::Relaxed);
        let n = CHUNKS.load(Ordering::Relaxed);
        let dur = t.saturating_sub(start).max(1);
        log::info!(
            "[probe] ── 串流結束：chunk={n}｜首個 chunk={}ms｜總時長={dur}ms｜平均 {:.1} chunk/s",
            first.saturating_sub(start),
            n as f64 * 1000.0 / dur as f64,
        );
        return;
    }
    log::info!("[probe] {payload}");
}

/// 開一個登入視窗並注入探勘腳本。
pub fn start(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let url = "https://chatgpt.com/".parse().expect("static url");
        match WebviewWindowBuilder::new(&handle, "chatgpt-probe", WebviewUrl::External(url))
            .title("AITerm 探勘：ChatGPT Web")
            .inner_size(1100.0, 850.0)
            .initialization_script(injected_script())
            .build()
        {
            Ok(_) => log::info!("[probe] 登入視窗已開啟（回報走 Tauri IPC）"),
            Err(e) => log::error!("[probe] 開視窗失敗：{e}"),
        }
    });
}
