import { describe, expect, it } from "vitest";
import { parseStoredQa, qaToMarkdown, serializeQa, type MdLabels } from "./qaStore";
import type { QaMessage } from "./QaPanel";

const L: MdLabels = { title: "架構問答", exportedAt: "匯出時間", question: "問題", tour: "流程導覽", diagram: "流程圖", truncated: "（回答被截斷）" };

const ai = (over: Partial<Extract<QaMessage, { role: "ai" }>["qa"]> = {}): QaMessage => ({
  id: "a1", role: "ai",
  qa: { answer: "登入走 `/api/login`。", tour: null, diagram: "", dropped: 0, truncated: false, ...over },
});

describe("serializeQa / parseStoredQa", () => {
  it("只存使用者與 AI 的訊息，錯誤泡泡不存", () => {
    const msgs: QaMessage[] = [
      { id: "u1", role: "user", text: "怎麼登入？" },
      { id: "e1", role: "error", text: "網路錯誤", raw: "x" },
      ai(),
    ];
    const back = parseStoredQa(serializeQa(msgs));
    expect(back.map((m) => m.role)).toEqual(["user", "ai"]);
    expect(back[1]).toMatchObject({ role: "ai", qa: { answer: "登入走 `/api/login`。" } });
  });
  it("壞掉的 JSON、非陣列、缺欄位的項目一律丟掉而不是拋例外", () => {
    expect(parseStoredQa("not json")).toEqual([]);
    expect(parseStoredQa('{"a":1}')).toEqual([]);
    const mixed = JSON.stringify({ v: 1, messages: [{ id: "x", role: "user" }, { id: "y", role: "ai", qa: { answer: 5 } }, { id: "z", role: "user", text: "ok" }] });
    expect(parseStoredQa(mixed)).toEqual([{ id: "z", role: "user", text: "ok" }]);
  });
  it("只留最後 100 則，避免記錄檔無限長", () => {
    const msgs: QaMessage[] = Array.from({ length: 150 }, (_, i) => ({ id: `u${i}`, role: "user" as const, text: `q${i}` }));
    const back = parseStoredQa(serializeQa(msgs));
    expect(back).toHaveLength(100);
    expect(back[0]).toMatchObject({ text: "q50" });
  });
  it("導覽與圖表也能還原，導覽步驟的證據保留", () => {
    const tour = { title: "登入", steps: [{ module: "m1", caption: "送出", evidence: { file: "a.ts", line: 3 } }, { module: "m2", caption: "驗證" }] };
    const back = parseStoredQa(serializeQa([ai({ tour, diagram: "graph LR; A-->B" })]));
    expect(back[0]).toMatchObject({ qa: { tour, diagram: "graph LR; A-->B" } });
  });
  it("證據那行的程式碼文字（text）也會存、讀回", () => {
    const tour = { title: "t", steps: [{ module: "m", caption: "c", evidence: { file: "a.ts", line: 3, text: "return x;" } }] };
    expect(parseStoredQa(serializeQa([ai({ tour })]))[0]).toMatchObject({ qa: { tour } });
  });
  it("提問時間（at）會存、讀回；舊記錄沒有或型別不對就沒有", () => {
    const back = parseStoredQa(serializeQa([{ id: "u", role: "user", text: "q", at: 1700000000000 }]));
    expect(back).toEqual([{ id: "u", role: "user", text: "q", at: 1700000000000 }]);
    const old = parseStoredQa(JSON.stringify({ v: 1, messages: [{ id: "a", role: "user", text: "x" }, { id: "b", role: "user", text: "y", at: "昨天" }] }));
    expect(old).toEqual([{ id: "a", role: "user", text: "x" }, { id: "b", role: "user", text: "y" }]);
  });

  it("導覽步驟型別不對時只丟該步，保留其餘", () => {
    const raw = JSON.stringify({ v: 1, messages: [{ id: "a", role: "ai", qa: { answer: "x", diagram: "", dropped: 0, truncated: false, tour: { title: "t", steps: [{ module: 1, caption: "bad" }, { module: "m", caption: "good" }] } } }] });
    const back = parseStoredQa(raw);
    expect(back[0]).toMatchObject({ qa: { tour: { steps: [{ module: "m", caption: "good" }] } } });
  });
});

describe("qaToMarkdown", () => {
  const when = new Date("2026-10-23T08:00:00Z");
  it("標題含專案名稱與匯出時間，每題一個二級標題，答案原樣放在下面", () => {
    const md = qaToMarkdown([{ id: "u1", role: "user", text: "怎麼登入？" }, ai()], "HTBTW_Web", when, L);
    expect(md).toMatch(/^# 架構問答 — HTBTW_Web\n/);
    expect(md).toContain("匯出時間：2026-10-23");
    expect(md).toContain("## 問題 1：怎麼登入？");
    expect(md).toContain("登入走 `/api/login`。");
  });
  it("圖表放成 mermaid 區塊；導覽列成編號清單並附證據", () => {
    const tour = { title: "登入流程", steps: [{ module: "m1", caption: "送出", evidence: { file: "a.ts", line: 3 } }, { module: "m2", caption: "驗證" }] };
    const md = qaToMarkdown([{ id: "u", role: "user", text: "q" }, ai({ tour, diagram: "graph LR\nA-->B" })], "P", when, L, (id) => ({ m1: "前端", m2: "後端" })[id] ?? id);
    expect(md).toContain("```mermaid\ngraph LR\nA-->B\n```");
    expect(md).toContain("### 流程導覽：登入流程");
    expect(md).toContain("1. **前端**：送出（`a.ts:3`）");
    expect(md).toContain("2. **後端**：驗證");
  });
  it("答案本身含 ``` 時 mermaid 外框仍然不會被提早關閉（用更長的圍欄）", () => {
    const md = qaToMarkdown([ai({ diagram: "a\n```\nb" })], "P", when, L);
    expect(md).toContain("````mermaid\na\n```\nb\n````");
  });
  it("被截斷的回答有註記；沒有訊息時只有標題", () => {
    expect(qaToMarkdown([ai({ truncated: true })], "P", when, L)).toContain("（回答被截斷）");
    expect(qaToMarkdown([], "P", when, L).trim().split("\n").length).toBeLessThanOrEqual(3);
  });
});
