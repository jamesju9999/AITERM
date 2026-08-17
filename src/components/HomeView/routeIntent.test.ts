import { describe, expect, it } from "vitest";
import { parseRouteReply, fallbackRoute } from "./routeIntent";

const userText = "查一下訂單表有幾筆";

describe("fallbackRoute", () => {
  // 降級路徑：AI 掛掉時輸入框仍然要有反應。
  it("降級為終端機分頁並把整句話當成 agent 任務", () => {
    expect(fallbackRoute("幫我看為什麼 build 失敗")).toEqual({
      type: "terminal",
      mission: "幫我看為什麼 build 失敗",
      userText: "幫我看為什麼 build 失敗",
      fallback: true,
    });
  });
});

describe("parseRouteReply", () => {
  it("讀得懂乾淨的 JSON", () => {
    expect(parseRouteReply('{"type":"database"}', userText)).toEqual({
      type: "database",
      userText,
      fallback: false,
    });
  });

  // 模型很愛把 JSON 包在 markdown 圍籬裡。
  it("讀得懂包在 ```json 圍籬裡的回應", () => {
    const raw = "好的\n```json\n{\"type\":\"knowledge-base\"}\n```\n";
    expect(parseRouteReply(raw, userText).type).toBe("knowledge-base");
  });

  it("terminal 會帶上使用者原句當任務目標", () => {
    expect(parseRouteReply('{"type":"terminal"}', userText)).toEqual({
      type: "terminal",
      mission: userText,
      userText,
      fallback: false,
    });
  });

  // 最重要的防呆：AI 回了清單外的東西，不可以照單全收去開一個不存在的分頁。
  it("清單外的分頁類型一律降級", () => {
    expect(parseRouteReply('{"type":"spreadsheet"}', userText)).toEqual(fallbackRoute(userText));
  });

  // hidden 的類型（api-docs / mail）後端完整但入口刻意收起來，
  // AI 路由不能變成它們的後門。
  it("hidden 的分頁類型也要降級", () => {
    expect(parseRouteReply('{"type":"mail"}', userText)).toEqual(fallbackRoute(userText));
    expect(parseRouteReply('{"type":"api-docs"}', userText)).toEqual(fallbackRoute(userText));
  });

  it("空回應降級", () => {
    expect(parseRouteReply("", userText)).toEqual(fallbackRoute(userText));
  });

  it("完全不是 JSON 的回應降級", () => {
    expect(parseRouteReply("我不確定你想做什麼", userText)).toEqual(fallbackRoute(userText));
  });

  it("是 JSON 但沒有 type 欄位，降級", () => {
    expect(parseRouteReply('{"reason":"不確定"}', userText)).toEqual(fallbackRoute(userText));
  });

  it("type 不是字串，降級", () => {
    expect(parseRouteReply('{"type":123}', userText)).toEqual(fallbackRoute(userText));
  });

  // 模型輸出在收尾大括號之前就被截斷。這條走的是「找不到結尾 }」的邊界檢查，
  // 到不了 JSON.parse——所以它不能用來證明 try/catch 有效，見下一條。
  it("輸出被截斷（少了結尾大括號）時降級，不丟例外", () => {
    expect(() => parseRouteReply('{"type":"database"', userText)).not.toThrow();
    expect(parseRouteReply('{"type":"database"', userText)).toEqual(fallbackRoute(userText));
  });

  // 括號成對但內容語法壞掉（截斷點剛好落在物件內部、或模型吐出尾隨逗號）。
  // 這是唯一會真的走到 JSON.parse 並丟例外的輸入——上面那條被邊界檢查提前
  // 攔截，所以拿掉 try/catch 時它照樣通過。沒有這條，try/catch 等於無人看守。
  it("括號成對但語法壞掉時降級，不丟例外", () => {
    const raw = '{"type": "database",}';
    expect(() => parseRouteReply(raw, userText)).not.toThrow();
    expect(parseRouteReply(raw, userText)).toEqual(fallbackRoute(userText));
  });
});
