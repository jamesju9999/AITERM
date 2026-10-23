import { describe, it, expect } from "vitest";
import { createImeEnterGuard, isImeComposing } from "./imeComposing";

describe("isImeComposing", () => {
  it("React 合成事件：組字中 (nativeEvent.isComposing) 回傳 true", () => {
    expect(isImeComposing({ nativeEvent: { isComposing: true } })).toBe(true);
  });

  it("React 合成事件：沒在組字回傳 false", () => {
    expect(isImeComposing({ nativeEvent: { isComposing: false } })).toBe(false);
  });

  it("原生 KeyboardEvent：組字中 (isComposing) 回傳 true", () => {
    expect(isImeComposing({ isComposing: true })).toBe(true);
  });

  it("舊版 WebKit：keyCode === 229 回傳 true", () => {
    expect(isImeComposing({ keyCode: 229 })).toBe(true);
  });

  it("一般 Enter 鍵 (keyCode 13、未組字) 回傳 false", () => {
    expect(isImeComposing({ keyCode: 13, isComposing: false, nativeEvent: { isComposing: false } })).toBe(false);
  });

  it("空物件回傳 false", () => {
    expect(isImeComposing({})).toBe(false);
  });
});

describe("createImeEnterGuard", () => {
  const mk = () => {
    let t = 1000;
    const g = createImeEnterGuard(() => t);
    return { g, advance: (ms: number) => { t += ms; } };
  };
  const enter = { keyCode: 13, isComposing: false, nativeEvent: { isComposing: false } };

  it("(a) isComposing 為 true 時忽略", () => {
    expect(mk().g.shouldIgnoreEnter({ nativeEvent: { isComposing: true } })).toBe(true);
  });
  it("(b) keyCode 229 時忽略", () => {
    expect(mk().g.shouldIgnoreEnter({ keyCode: 229 })).toBe(true);
  });
  it("(c) compositionstart 後尚未結束：忽略", () => {
    const { g, advance } = mk();
    g.onCompositionStart();
    advance(5000);
    expect(g.shouldIgnoreEnter(enter)).toBe(true);
  });
  it("(d) compositionend 後 10ms 的 Enter（WKWebView 實況：isComposing=false, keyCode=13）：忽略", () => {
    const { g, advance } = mk();
    g.onCompositionStart();
    g.onCompositionEnd();
    advance(10);
    expect(g.shouldIgnoreEnter(enter)).toBe(true);
  });
  it("(e) compositionend 後 150ms 的 Enter：放行", () => {
    const { g, advance } = mk();
    g.onCompositionStart();
    g.onCompositionEnd();
    advance(150);
    expect(g.shouldIgnoreEnter(enter)).toBe(false);
  });
  it("(f) 沒有組字過的純 Enter：放行", () => {
    expect(mk().g.shouldIgnoreEnter(enter)).toBe(false);
  });
  it("(g) 第二輪組字同樣有效", () => {
    const { g, advance } = mk();
    g.onCompositionStart();
    g.onCompositionEnd();
    advance(500);
    expect(g.shouldIgnoreEnter(enter)).toBe(false);
    g.onCompositionStart();
    expect(g.shouldIgnoreEnter(enter)).toBe(true);
    g.onCompositionEnd();
    advance(20);
    expect(g.shouldIgnoreEnter(enter)).toBe(true);
    advance(200);
    expect(g.shouldIgnoreEnter(enter)).toBe(false);
  });
});
