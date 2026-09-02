import { describe, it, expect } from "vitest";
import { isImeComposing } from "./imeComposing";

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
