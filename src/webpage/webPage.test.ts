/* eslint-disable @typescript-eslint/no-explicit-any -- 模擬瀏覽器環境，型別只會礙事 */
// 網頁提問頁是 Rust 後端內嵌的一份 HTML（src-tauri/src/intake/web_page.html）。
// 這裡用 jsdom 真的載入、執行它，模擬後端回應，驗證整段操作流程。
import raw from "../../src-tauri/src/intake/web_page.html?raw";
import css from "../../src-tauri/src/intake/web_base.css?raw";
import { JSDOM } from "jsdom";
import { describe, it, expect } from "vitest";

const HTML = raw.replace("__BASE_CSS__", css).replaceAll("__TITLE__", "TEST").replaceAll("__BASE__", "/p/t1");

type Handler = (body: any) => { status?: number; json: any };
type Call = { path: string; body: any };

function load(opts: { lang?: string; handlers: Record<string, Handler>; session?: object; draft?: object }) {
  const calls: Call[] = [];
  const uploads: { url: string; name: string }[] = [];
  const dom = new JSDOM(HTML, {
    runScripts: "dangerously",
    pretendToBeVisual: true,
    url: "http://127.0.0.1:8320/p/t1",
    beforeParse(w: any) {
      Object.defineProperty(w.navigator, "language", { value: opts.lang ?? "zh-TW" });
      if (opts.session) w.sessionStorage.setItem("aiterm.web./p/t1s", JSON.stringify(opts.session));
      if (opts.draft) w.sessionStorage.setItem("aiterm.web./p/t1d", JSON.stringify(opts.draft));
      w.URL.createObjectURL = () => "blob:test"; // jsdom 沒有，瀏覽器都有
      w.URL.revokeObjectURL = () => {};
      w.fetch = async (url: string, init: any) => {
        const path = String(url).replace("/p/t1", "");
        const body = JSON.parse(init.body);
        calls.push({ path, body });
        const h = opts.handlers[path];
        const r = h ? h(body) : { status: 404, json: {} };
        const status = r.status ?? 200;
        return { ok: status < 300, status, json: async () => r.json };
      };
      w.XMLHttpRequest = class {
        status = 0;
        responseText = "";
        upload: any = {};
        onload: any;
        onerror: any;
        _url = "";
        open(_m: string, url: string) { this._url = url; }
        setRequestHeader() {}
        abort() {}
        send(file: File) {
          const name = decodeURIComponent(/name=([^&]*)/.exec(this._url)![1]);
          uploads.push({ url: this._url, name });
          void file;
          this.status = 200;
          this.responseText = JSON.stringify({ id: `up-${uploads.length}`, size: 1 });
          setTimeout(() => this.onload?.(), 0);
        }
      };
    },
  });
  const w = dom.window as any;
  const $ = (id: string) => w.document.getElementById(id) as HTMLElement & Record<string, any>;
  const flush = () => new Promise((r) => setTimeout(r, 5));
  const typeCode = async (code: string) => {
    const cells = [...w.document.querySelectorAll(".cell")] as HTMLInputElement[];
    for (let i = 0; i < code.length; i++) {
      cells[i].value = code[i];
      cells[i].dispatchEvent(new w.Event("input", { bubbles: true }));
    }
    await flush();
  };
  return { w, $, calls, uploads, flush, typeCode };
}

const ok = (json: any = {}): Handler => () => ({ json });
const LOGIN: Record<string, Handler> = {
  "/api/code": ok({ ok: true }),
  "/api/login": () => ({ json: { token: "tok", email: "bob@corp.com", ttl: 1800 } }),
  "/api/status": ok({ items: [] }),
  "/api/logout": ok(),
  "/api/unattach": ok(),
  "/api/submit": () => ({ json: { ok: true, ttl: 1800 } }),
};

async function signIn(p: ReturnType<typeof load>) {
  p.$("email").value = "bob@corp.com";
  p.$("f1").dispatchEvent(new p.w.Event("submit", { cancelable: true }));
  await p.flush();
  await p.typeCode("123456");
}

describe("web question page", () => {
  it("asks for the email, then six code boxes; a full code logs in by itself and opens the form", async () => {
    const p = load({ handlers: LOGIN });
    expect(p.$("compose").hidden).toBe(true);
    await signIn(p);
    expect(p.calls.map((c) => c.path)).toContain("/api/code");
    expect(p.calls.find((c) => c.path === "/api/login")!.body).toEqual({ email: "bob@corp.com", code: "123456" });
    expect(p.$("compose").hidden).toBe(false);
    expect(p.$("login").hidden).toBe(true);
    expect(p.$("whoami").textContent).toBe("bob@corp.com");
  });

  it("pasting the whole code into the first box fills all six", async () => {
    const p = load({ handlers: LOGIN });
    p.$("email").value = "bob@corp.com";
    p.$("f1").dispatchEvent(new p.w.Event("submit", { cancelable: true }));
    await p.flush();
    const first = p.w.document.querySelector(".cell");
    const ev = new p.w.Event("paste", { bubbles: true, cancelable: true }) as any;
    ev.clipboardData = { getData: () => "654 321" };
    first.dispatchEvent(ev);
    await p.flush();
    expect(p.calls.find((c) => c.path === "/api/login")!.body.code).toBe("654321");
  });

  it("a wrong code shows the error under the boxes, clears them and keeps the form closed", async () => {
    const p = load({ handlers: { ...LOGIN, "/api/login": () => ({ status: 403, json: { error: "x", code: "bad_code" } }) } });
    await signIn(p);
    expect(p.$("e-code").textContent).toBe("驗證碼錯誤或已過期");
    expect(p.$("compose").hidden).toBe(true);
    expect([...p.w.document.querySelectorAll(".cell")].every((c: any) => c.value === "")).toBe(true);
  });

  it("an invalid email is stopped in the page with a message beside the field", async () => {
    const p = load({ handlers: LOGIN });
    p.$("email").value = "not-an-email";
    p.$("f1").dispatchEvent(new p.w.Event("submit", { cancelable: true }));
    await p.flush();
    expect(p.$("e-email").textContent).toContain("電子郵件");
    expect(p.calls.length).toBe(0);
  });

  it("a pasted screenshot is uploaded on its own, and the question is sent with the upload id; the form then clears for the next one", async () => {
    const p = load({ handlers: LOGIN });
    await signIn(p);
    p.$("body").value = "畫面壞了";
    p.$("body").dispatchEvent(new p.w.Event("input"));
    const png = new p.w.File([new Uint8Array([1, 2, 3])], "image.png", { type: "image/png" });
    const ev = new p.w.Event("paste", { bubbles: true }) as any;
    ev.clipboardData = { files: [png] };
    p.w.document.dispatchEvent(ev);
    await p.flush();
    expect(p.uploads).toHaveLength(1);
    expect(p.uploads[0].url).toContain("token=tok");
    expect(p.uploads[0].name).toMatch(/^貼上的截圖-\d+\.png$/);
    p.$("f3").dispatchEvent(new p.w.Event("submit", { cancelable: true }));
    await p.flush();
    const sub = p.calls.find((c) => c.path === "/api/submit")!.body;
    expect(sub).toMatchObject({ token: "tok", body: "畫面壞了", uploads: ["up-1"] });
    expect(p.$("body").value).toBe("");
    expect(p.$("files").children.length).toBe(0);
    expect(p.$("msg").textContent).toContain("已收到");
    expect(p.calls.filter((c) => c.path === "/api/status").length).toBeGreaterThan(1);
  });

  it("empty submission is refused in the page with a message under the box", async () => {
    const p = load({ handlers: LOGIN });
    await signIn(p);
    p.$("f3").dispatchEvent(new p.w.Event("submit", { cancelable: true }));
    await p.flush();
    expect(p.$("e-body").textContent).toContain("問題內容");
    expect(p.calls.some((c) => c.path === "/api/submit")).toBe(false);
  });

  it("when the session expires on submit the form returns to verification and the draft stays", async () => {
    const p = load({ handlers: { ...LOGIN, "/api/submit": () => ({ status: 401, json: { code: "session" } }) } });
    await signIn(p);
    p.$("body").value = "還沒送出的長篇大論";
    p.$("body").dispatchEvent(new p.w.Event("input"));
    p.$("f3").dispatchEvent(new p.w.Event("submit", { cancelable: true }));
    await p.flush();
    expect(p.$("login").hidden).toBe(false);
    expect(p.$("compose").hidden).toBe(true);
    expect(p.$("body").value).toBe("還沒送出的長篇大論");
    expect(p.$("msg").textContent).toContain("你寫的內容還在");
  });

  it("the draft survives a reload and a still-valid session is restored without a new code", async () => {
    const p = load({
      handlers: LOGIN,
      draft: { subject: "主旨", body: "寫到一半" },
      session: { token: "tok", email: "bob@corp.com", expiry: Date.now() + 600_000 },
    });
    await p.flush();
    expect(p.$("body").value).toBe("寫到一半");
    expect(p.$("compose").hidden).toBe(false);
    expect(p.calls.some((c) => c.path === "/api/code")).toBe(false);
  });

  it("a stored session the server no longer knows is dropped quietly", async () => {
    const p = load({
      handlers: { ...LOGIN, "/api/status": () => ({ status: 401, json: { code: "session" } }) },
      session: { token: "stale", email: "bob@corp.com", expiry: Date.now() + 600_000 },
    });
    await p.flush();
    expect(p.$("compose").hidden).toBe(true);
    expect(p.$("login").hidden).toBe(false);
  });

  it("lists the asker's questions with a coarse stage, never internal state", async () => {
    const p = load({
      handlers: { ...LOGIN, "/api/status": ok({ items: [{ id: "1", subject: "壞掉了", status: "working", at: 0 }, { id: "2", subject: "好了", status: "replied", at: 0 }] }) },
    });
    await signIn(p);
    const rows = [...p.$("sent").children].map((li: any) => li.textContent);
    expect(rows[0]).toContain("壞掉了");
    expect(rows[0]).toContain("處理中");
    expect(rows[1]).toContain("已回覆");
  });

  it("speaks English to a browser set to English", async () => {
    const p = load({ lang: "en-US", handlers: LOGIN });
    expect(p.$("send").textContent).toBe("Send code");
    expect(p.w.document.documentElement.lang).toBe("en");
  });
});
