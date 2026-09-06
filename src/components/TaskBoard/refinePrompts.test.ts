import { describe, expect, it } from "vitest";

import { buildRefinePrompt, parseRefined } from "./refinePrompts";

describe("buildRefinePrompt", () => {
  it("把使用者的草稿原文帶進去", () => {
    const p = buildRefinePrompt("把打卡 API 整理一下", "/r", false);
    expect(p).toContain("把打卡 API 整理一下");
  });

  it("帶上工作目錄，讓模型知道要在哪裡動手", () => {
    expect(buildRefinePrompt("x", "/Users/me/HCP", false)).toContain("/Users/me/HCP");
  });

  // 目錄沒填不該讓提示詞出現空白的一行「工作目錄：」，那會讓模型以為
  // 路徑是空字串。
  it("目錄沒填時明說未指定", () => {
    expect(buildRefinePrompt("x", "", false)).toContain("（未指定）");
  });

  it("需要標題時才要求標題行", () => {
    expect(buildRefinePrompt("x", "/r", true)).toContain("標題：");
    expect(buildRefinePrompt("x", "/r", false)).not.toContain("標題：");
  });
});

describe("parseRefined", () => {
  it("拆出標題與內容", () => {
    const got = parseRefined("標題：整理打卡 API\n---\n目標：產出 OpenAPI 規格檔。\n驗收：yaml 可用。");
    expect(got).toEqual({
      title: "整理打卡 API",
      body: "目標：產出 OpenAPI 規格檔。\n驗收：yaml 可用。",
    });
  });

  it("沒有分隔線時整份都是內容", () => {
    expect(parseRefined("目標：產出規格檔。")).toEqual({
      title: null,
      body: "目標：產出規格檔。",
    });
  });

  // 沒要求標題時模型就不會寫分隔線，內容裡的 --- 也可能是 Markdown 分隔線；
  // 這種情況下前面那段仍然是內容的一部分，不能被吃掉當標題。
  it("分隔線前有多行時不把它們當標題", () => {
    const got = parseRefined("目標：整理 API。\n範圍：只動 login。\n---\n驗收：yaml 可用。");
    expect(got?.title).toBeNull();
  });

  it("容忍模型用半形冒號或英文 Title", () => {
    expect(parseRefined("Title: Refactor login\n---\nDo the thing.")?.title).toBe("Refactor login");
    expect(parseRefined("標題: 重構登入\n---\n做這件事。")?.title).toBe("重構登入");
  });

  // 「標題：」前綴沒寫、但分隔線前只有一行有字的話，那一行顯然就是標題。
  it("沒有前綴但只有一行時當作標題", () => {
    expect(parseRefined("整理打卡 API\n---\n目標：產出規格檔。")?.title).toBe("整理打卡 API");
  });

  it("剝掉模型多包的程式碼區塊", () => {
    expect(parseRefined("```\n標題：甲\n---\n乙\n```")).toEqual({ title: "甲", body: "乙" });
  });

  // 格式壞掉時，保住內容比保住標題重要——使用者按了潤飾卻拿到空白輸入框
  // 是最糟的結果。
  it("分隔線後面是空的就整份當內容", () => {
    const got = parseRefined("標題：甲\n---\n   ");
    expect(got?.body).toContain("標題：甲");
  });

  it("整份空白回傳 null", () => {
    expect(parseRefined("   \n  ")).toBeNull();
  });
});
