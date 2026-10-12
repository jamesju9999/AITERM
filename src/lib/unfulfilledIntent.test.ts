import { describe, expect, it } from "vitest";
import { looksLikeUnfulfilledIntent } from "./unfulfilledIntent";

describe("looksLikeUnfulfilledIntent", () => {
  it.each([
    "LINE 應用程式已安裝，讓我先取得 LINE 的 UI 快照，以找到景詳的對話。",
    "讓我先取得 UI 快照以找到景詳的對話。",
    "我來點擊搜尋欄。",
    "接下來我會輸入訊息並送出。",
    "Let me read the UI snapshot first.",
    "I'll click the search field now:",
  ])("宣告了要做、卻沒呼叫工具：%s", (text) => {
    expect(looksLikeUnfulfilledIntent(text)).toBe(true);
  });

  it.each([
    "已完成：在 TextEdit 中按 cmd+a 再按 delete。",
    "目前 TextEdit 的內容是空的。",
    "我找不到名為景詳的聯絡人，請確認名稱。",
    "你好！有什麼我可以幫忙的？",
    "Done. The field is now empty.",
    "",
  ])("一般回答或最終結果不算：%s", (text) => {
    expect(looksLikeUnfulfilledIntent(text)).toBe(false);
  });

  it("很長的回覆（多半是真的在解釋）不算，即使以「讓我」開頭", () => {
    expect(looksLikeUnfulfilledIntent("讓我解釋一下。" + "這是一段很長的說明。".repeat(40))).toBe(false);
  });

  it("以問句結尾是在向使用者確認，不算", () => {
    expect(looksLikeUnfulfilledIntent("讓我先確認一下，你要傳給哪一位景詳？")).toBe(false);
    expect(looksLikeUnfulfilledIntent("Let me know which contact you mean?")).toBe(false);
  });
});
