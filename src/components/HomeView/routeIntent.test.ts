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

  // JSON 語法壞掉（模型輸出被截斷是常見情況）
  it("JSON 語法壞掉時降級，不丟例外", () => {
    expect(() => parseRouteReply('{"type":"database"', userText)).not.toThrow();
    expect(parseRouteReply('{"type":"database"', userText)).toEqual(fallbackRoute(userText));
  });
});
