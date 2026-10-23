import { describe, expect, it } from "vitest";
import {
  ANSWER_SYSTEM, SELECT_SYSTEM, archSummary, buildAnswerMessage, buildSelectMessage, extractJsonObject, parseAnswer, parseSelect,
} from "./qa";
import type { Architecture } from "./model";
import type { ScanResult } from "../../ipc/architecture";

const scan: ScanResult = {
  root: "/p",
  files: [
    { path: "web/App.tsx", lang: "typescript", lines: 50 },
    { path: "web/api.ts", lang: "typescript", lines: 20 },
    { path: "web/a.ts", lang: "typescript", lines: 5 },
    { path: "web/b.ts", lang: "typescript", lines: 5 },
    { path: "web/c.ts", lang: "typescript", lines: 5 },
    { path: "server/routes.py", lang: "python", lines: 80 },
    { path: "server/db.py", lang: "python", lines: 30 },
  ],
  edges: [], deps: [], truncated: false, git_head: null,
};

const arch: Architecture = {
  layers: [{ id: "frontend", name: "前端" }, { id: "backend", name: "後端" }],
  modules: [
    { id: "web", layer: "frontend", name: "網頁", files: ["web/App.tsx", "web/api.ts", "web/a.ts", "web/b.ts", "web/c.ts"], tech: ["React", "Vite"], summary: "UI 外殼", dataflow: "" },
    { id: "server", layer: "backend", name: "服務", files: ["server/routes.py", "server/db.py"], tech: ["Flask"], summary: "API 服務", dataflow: "" },
  ],
  edges: [
    { from: "web", to: "server", kind: "http", source: "ai", confidence: 0.9, note: "登入請求", evidence: { file: "web/api.ts", line: 5 } },
    { from: "server", to: "web", kind: "import", source: "scan", confidence: 1 },
  ],
  meta: { scannedAt: "", gitHead: null, analyzed: true, dropped: 0 },
};

describe("system prompts", () => {
  it("SELECT 要求 JSON、files 上限 8", () => {
    expect(SELECT_SYSTEM).toContain("JSON");
    expect(SELECT_SYSTEM).toContain("modules");
    expect(SELECT_SYSTEM).toContain("files");
    expect(SELECT_SYSTEM).toContain("8");
  });
  it("ANSWER 說明欄位、3–10 步、mermaid 不加圍欄", () => {
    for (const k of ["answer", "walkthrough", "steps", "module", "caption", "evidence", "diagram", "sequenceDiagram", "flowchart LR", "Mermaid"]) {
      expect(ANSWER_SYSTEM).toContain(k);
    }
    expect(ANSWER_SYSTEM).toMatch(/3.{0,3}10/);
  });
});

describe("archSummary", () => {
  it("模組行與連線行格式", () => {
    const s = archSummary(arch);
    expect(s).toContain("web | 網頁 | frontend | UI 外殼 | React, Vite");
    expect(s).toContain("web -http-> server (ai, 登入請求, web/api.ts:5)");
    expect(s).toContain("server -import-> web (scan)");
  });
  it("超過 12000 字先截連線並附省略說明，模組仍完整", () => {
    const big: Architecture = {
      ...arch,
      edges: Array.from({ length: 400 }, (_, i) => ({
        from: "web", to: "server", kind: "ipc" as const, source: "ai" as const, confidence: 1, note: `連線說明 ${i} ${"x".repeat(40)}`,
      })),
    };
    const s = archSummary(big);
    expect(s.length).toBeLessThanOrEqual(12000);
    expect(s).toContain("web | 網頁 | frontend");
    expect(s).toContain("server | 服務 | backend");
    const m = /…省略 (\d+) 條連線/.exec(s);
    expect(m).not.toBeNull();
    const shown = (s.match(/-ipc->/g) ?? []).length;
    expect(shown + Number(m![1])).toBe(400);
    expect(shown).toBeGreaterThan(0);
  });
  it("未超過時不出現省略說明", () => {
    expect(archSummary(arch)).not.toContain("省略");
  });
});

describe("buildSelectMessage / buildAnswerMessage", () => {
  it("挑檔訊息含摘要、問題與可用檔案路徑", () => {
    const m = buildSelectMessage(arch, "登入怎麼跑？");
    expect(m).toContain("web | 網頁");
    expect(m).toContain("登入怎麼跑？");
    expect(m).toContain("web/App.tsx");
    expect(m).toContain("server/db.py");
  });
  it("回答訊息含摘要、問題與檔案內容", () => {
    const m = buildAnswerMessage(arch, "登入怎麼跑？", [["web/api.ts", "fetch('/login')"]]);
    expect(m).toContain("web | 網頁");
    expect(m).toContain("登入怎麼跑？");
    expect(m).toContain("### web/api.ts");
    expect(m).toContain("fetch('/login')");
  });
});

describe("parseSelect", () => {
  it("接受合法 JSON 與圍欄", () => {
    const r = parseSelect("```json\n" + JSON.stringify({ modules: ["web"], files: ["web/api.ts"] }) + "\n```", arch, scan);
    expect(r.modules).toEqual(["web"]);
    expect(r.files).toEqual(["web/api.ts"]);
  });
  it("非 JSON 或非物件回空陣列且不丟例外", () => {
    expect(parseSelect("抱歉我不知道", arch, scan)).toEqual({ modules: [], files: [] });
    expect(parseSelect("[1,2]", arch, scan)).toEqual({ modules: [], files: [] });
    expect(parseSelect("null", arch, scan)).toEqual({ modules: [], files: [] });
    expect(parseSelect(JSON.stringify({ modules: "web", files: 3 }), arch, scan)).toEqual({ modules: [], files: [] });
  });
  it("丟掉未知模組與不在掃描內的檔案（需完全相符）", () => {
    const r = parseSelect(JSON.stringify({ modules: ["ghost", "server"], files: ["nope.ts", "server/db.py", "./server/routes.py", 7] }), arch, scan);
    expect(r.modules).toEqual(["server"]);
    expect(r.files).toEqual(["server/db.py"]);
  });
  it("已選模組沒有任何有效檔案時補它的前 3 個檔案", () => {
    const r = parseSelect(JSON.stringify({ modules: ["web"], files: ["nope.ts"] }), arch, scan);
    expect(r.files).toEqual(["web/App.tsx", "web/api.ts", "web/a.ts"]);
  });
  it("模組已有有效檔案就不補", () => {
    const r = parseSelect(JSON.stringify({ modules: ["web", "server"], files: ["web/b.ts"] }), arch, scan);
    expect(r.files).toEqual(["web/b.ts", "server/routes.py", "server/db.py"]);
  });
  it("去重並保留出現順序，上限 8", () => {
    const files = Array.from({ length: 10 }, (_, i) => `f${i}.ts`);
    const big: ScanResult = { ...scan, files: files.map((p) => ({ path: p, lang: "typescript", lines: 1 })) };
    const bigArch: Architecture = { ...arch, modules: [{ ...arch.modules[0], files }] };
    const r = parseSelect(JSON.stringify({ modules: ["web", "web"], files: ["f3.ts", "f3.ts", ...files] }), bigArch, big);
    expect(r.modules).toEqual(["web"]);
    expect(r.files).toHaveLength(8);
    expect(r.files.slice(0, 3)).toEqual(["f3.ts", "f0.ts", "f1.ts"]);
  });
});

const goodAnswer = {
  answer: "## 登入\n先打 API。",
  walkthrough: {
    title: "登入流程",
    steps: [
      { module: "web", caption: "送出", evidence: { file: "web/api.ts", line: 5 } },
      { module: "server", caption: "驗證" },
    ],
  },
  diagram: "sequenceDiagram\n  Web->>Server: login",
};

describe("parseAnswer", () => {
  it("接受合法回應", () => {
    const r = parseAnswer(JSON.stringify(goodAnswer), arch, scan)!;
    expect(r.answer).toBe("## 登入\n先打 API。");
    expect(r.tour!.title).toBe("登入流程");
    expect(r.tour!.steps).toEqual(goodAnswer.walkthrough.steps);
    expect(r.diagram).toBe(goodAnswer.diagram);
    expect(r.dropped).toBe(0);
  });
  it("圍欄包住整個回覆也能解", () => {
    expect(parseAnswer("```json\n" + JSON.stringify(goodAnswer) + "\n```", arch, scan)).not.toBeNull();
  });
  it("非 JSON 物件或 answer 不是非空字串回 null", () => {
    expect(parseAnswer("[]", arch, scan)).toBeNull();
    expect(parseAnswer("null", arch, scan)).toBeNull();
    expect(parseAnswer(JSON.stringify({ answer: "" }), arch, scan)).toBeNull();
    expect(parseAnswer(JSON.stringify({ answer: "  " }), arch, scan)).toBeNull();
    expect(parseAnswer(JSON.stringify({ answer: 5 }), arch, scan)).toBeNull();
    expect(parseAnswer(JSON.stringify({}), arch, scan)).toBeNull();
  });
  it("整步丟掉：未知模組、caption 非字串或空，並計入 dropped", () => {
    const r = parseAnswer(JSON.stringify({
      answer: "x",
      walkthrough: { title: "t", steps: [
        { module: "web", caption: "a" },
        { module: "ghost", caption: "b" },
        { module: "server", caption: "" },
        { module: "server", caption: 3 },
        { module: "server", caption: "c" },
      ] },
    }), arch, scan)!;
    expect(r.tour!.steps.map((s) => s.caption)).toEqual(["a", "c"]);
    expect(r.dropped).toBe(3);
  });
  it("evidence 不在掃描內或行號越界只丟 evidence、保留該步、不計 dropped", () => {
    const mk = (evidence: unknown) => parseAnswer(JSON.stringify({
      answer: "x",
      walkthrough: { title: "t", steps: [{ module: "web", caption: "a", evidence }, { module: "server", caption: "b" }] },
    }), arch, scan)!;
    for (const ev of [
      { file: "nope.ts", line: 1 }, { file: "web/api.ts", line: 21 }, { file: "web/api.ts", line: 0 },
      { file: "web/api.ts", line: 1.5 }, { file: "web/api.ts", line: "3" }, { file: 3, line: 3 }, "web/api.ts:3",
    ]) {
      const r = mk(ev);
      expect(r.tour!.steps).toHaveLength(2);
      expect(r.tour!.steps[0].evidence).toBeUndefined();
      expect("evidence" in r.tour!.steps[0]).toBe(false);
      expect(r.dropped).toBe(0);
    }
    expect(mk({ file: "web/api.ts", line: 20 }).tour!.steps[0].evidence).toEqual({ file: "web/api.ts", line: 20 });
    expect(mk({ file: "web/api.ts", line: 1 }).tour!.steps[0].evidence).toEqual({ file: "web/api.ts", line: 1 });
  });
  it("有效步驟少於 2 時 tour 為 null", () => {
    const one = parseAnswer(JSON.stringify({ answer: "x", walkthrough: { title: "t", steps: [{ module: "web", caption: "a" }, { module: "ghost", caption: "b" }] } }), arch, scan)!;
    expect(one.tour).toBeNull();
    expect(one.dropped).toBe(1);
    expect(parseAnswer(JSON.stringify({ answer: "x", walkthrough: { title: "t", steps: [] } }), arch, scan)!.tour).toBeNull();
    expect(parseAnswer(JSON.stringify({ answer: "x" }), arch, scan)!.tour).toBeNull();
    expect(parseAnswer(JSON.stringify({ answer: "x", walkthrough: "no" }), arch, scan)!.tour).toBeNull();
  });
  it("title 缺漏時給預設空字串以外的可用標題", () => {
    const r = parseAnswer(JSON.stringify({ answer: "x", walkthrough: { steps: [{ module: "web", caption: "a" }, { module: "server", caption: "b" }] } }), arch, scan)!;
    expect(r.tour!.title.length).toBeGreaterThan(0);
  });
  it("diagram：去圍欄、trim、非字串為空", () => {
    const d = (diagram: unknown) => parseAnswer(JSON.stringify({ answer: "x", diagram }), arch, scan)!.diagram;
    expect(d("```mermaid\nflowchart LR\n A-->B\n```")).toBe("flowchart LR\n A-->B");
    expect(d("  flowchart LR\n A-->B \n")).toBe("flowchart LR\n A-->B");
    expect(d(42)).toBe("");
    expect(d(null)).toBe("");
    expect(parseAnswer(JSON.stringify({ answer: "x" }), arch, scan)!.diagram).toBe("");
  });
});

const GOOD_PRETTY = JSON.stringify(goodAnswer, null, 2);

describe("extractJsonObject", () => {
  it("乾淨 JSON 與整份圍欄", () => {
    expect(extractJsonObject('{"a":1}')).toEqual({ a: 1 });
    expect(extractJsonObject('```json\n{"a":1}\n```')).toEqual({ a: 1 });
  });
  it("前後有說明文字（Claude 常見的「以下是…」）", () => {
    expect(extractJsonObject("以下是我的分析：\n" + GOOD_PRETTY + "\n希望有幫助！")).toEqual(goodAnswer);
  });
  it("圍欄不是整個回覆：圍欄後面還有解釋", () => {
    expect(extractJsonObject("好的：\n```json\n" + GOOD_PRETTY + "\n```\n以上是答案，{可再追問}。")).toEqual(goodAnswer);
  });
  it("字串值內有真正的換行與 tab（長 Markdown answer）", () => {
    const raw = '{"answer":"## 標題\n- 一\n\t- 二","diagram":""}';
    expect(raw).toContain("\n"); // 確認夾具真的含字面換行
    expect(() => JSON.parse(raw)).toThrow();
    expect(extractJsonObject(raw)).toEqual({ answer: "## 標題\n- 一\n\t- 二", diagram: "" });
  });
  it("字串內的跳脫引號與已跳脫換行不被破壞", () => {
    const raw = '{"answer":"他說 \\"hi\\"\\n換行 {花括號}",\n"x":1}';
    expect(extractJsonObject("前言 " + raw + " 後記")).toEqual({ answer: '他說 "hi"\n換行 {花括號}', x: 1 });
  });
  it("物件與陣列尾逗號", () => {
    expect(extractJsonObject('{"a":[1,2,],"b":{"c":1,},}')).toEqual({ a: [1, 2], b: { c: 1 } });
    expect(extractJsonObject('{"a":"x,}"  ,}')).toEqual({ a: "x,}" }); // 字串裡的 ,} 不動
  });
  it("截斷在字串中間：補上引號與括號", () => {
    const cut = '{"answer":"登入流程是這樣的：先送出請求，然後';
    expect(extractJsonObject(cut)).toEqual({ answer: "登入流程是這樣的：先送出請求，然後" });
  });
  it("截斷在一步完整之後", () => {
    const cut = '{"answer":"x","walkthrough":{"title":"t","steps":[{"module":"web","caption":"送出"},';
    expect(extractJsonObject(cut)).toEqual({ answer: "x", walkthrough: { title: "t", steps: [{ module: "web", caption: "送出" }] } });
  });
  it("截斷在半個鍵／冒號後：退回到上一個完整成員", () => {
    expect(extractJsonObject('{"answer":"x","diagram":')).toEqual({ answer: "x" });
    expect(extractJsonObject('{"answer":"x","walk')).toEqual({ answer: "x" });
    expect(extractJsonObject('{"answer":"x\\')).toEqual({ answer: "x" }); // 結尾懸著一個反斜線
  });
  it("截斷發生在步驟物件中間", () => {
    const r = extractJsonObject('{"answer":"x","walkthrough":{"steps":[{"module":"web","caption":"a"},{"module":"server","cap');
    expect((r!.walkthrough as { steps: unknown[] }).steps[0]).toEqual({ module: "web", caption: "a" });
  });
  it("純文字、陣列、空字串、純空白回 null", () => {
    expect(extractJsonObject("這只是一段說明，沒有 JSON")).toBeNull();
    expect(extractJsonObject("[1,2,3]")).toBeNull();
    expect(extractJsonObject('[{"a":1}]')).toBeNull();
    expect(extractJsonObject("null")).toBeNull();
    expect(extractJsonObject("")).toBeNull();
    expect(extractJsonObject("   \n ")).toBeNull();
  });
});

describe("parseSelect 容錯", () => {
  it("說明文字包住的 JSON 也能挑檔", () => {
    const r = parseSelect('好的，我選：\n```json\n{"modules":["web"],"files":["web/api.ts"],}\n```\n以上。', arch, scan);
    expect(r.modules).toEqual(["web"]);
    expect(r.files).toContain("web/api.ts");
  });
});

describe("parseAnswer 容錯", () => {
  it("說明文字包住", () => {
    const r = parseAnswer("以下是答案：\n" + GOOD_PRETTY + "\n如需更多請告訴我", arch, scan)!;
    expect(r.answer).toBe(goodAnswer.answer);
    expect(r.tour!.steps).toHaveLength(2);
  });
  it("圍欄後接解釋", () => {
    expect(parseAnswer("```json\n" + GOOD_PRETTY + "\n```\n補充說明", arch, scan)!.tour).not.toBeNull();
  });
  it("answer 內含字面換行", () => {
    const raw = '{"answer":"第一行\n第二行","walkthrough":{"steps":[]},"diagram":""}';
    expect(parseAnswer(raw, arch, scan)!.answer).toBe("第一行\n第二行");
  });
  it("尾逗號", () => {
    expect(parseAnswer('{"answer":"ok","diagram":"",}', arch, scan)!.answer).toBe("ok");
  });
  it("截斷：answer 救回來，沒有導覽", () => {
    const r = parseAnswer('{"answer":"## 流程\\n先呼叫 API，然後', arch, scan)!;
    expect(r.answer).toBe("## 流程\n先呼叫 API，然後");
    expect(r.tour).toBeNull();
  });
  it("截斷在導覽中間：保留完整的步驟", () => {
    const cut = '{"answer":"x","walkthrough":{"title":"t","steps":[{"module":"web","caption":"a"},{"module":"server","caption":"b"},{"module":"web","cap';
    const r = parseAnswer(cut, arch, scan)!;
    expect(r.tour!.steps.map((s) => s.caption)).toEqual(["a", "b"]);
  });
  it("最後手段：結構壞到修不了，仍用寬鬆掃描撈出 answer", () => {
    // 巢狀括號錯亂，標準解析與修補都失敗。
    const bad = '{"answer":"救得回來的\\n答案","walkthrough":{"steps":[}}]]';
    expect(extractJsonObject(bad)).toBeNull();
    const r = parseAnswer(bad, arch, scan)!;
    expect(r.answer).toBe("救得回來的\n答案");
    expect(r.tour).toBeNull();
    expect(r.diagram).toBe("");
  });
  it("純文字（沒有 JSON）：整段當答案，去圍欄與前後空白", () => {
    expect(parseAnswer("  登入流程：先 A 再 B。\n\n- 一\n- 二  ", arch, scan)).toEqual({ answer: "登入流程：先 A 再 B。\n\n- 一\n- 二", tour: null, diagram: "", dropped: 0 });
    expect(parseAnswer("```markdown\n## 標題\n內容\n```", arch, scan)!.answer).toBe("## 標題\n內容");
  });
  it("看起來像失敗的 JSON 嘗試且修不了：回 null", () => {
    expect(parseAnswer('{"answer": ', arch, scan)).toBeNull();
    expect(parseAnswer('{"walkthrough":{"steps":[}}]]', arch, scan)).toBeNull();
    expect(parseAnswer('```json\n{"oops": [}\n```', arch, scan)).toBeNull();
    expect(parseAnswer("```JSON\n", arch, scan)).toBeNull();
  });
  it("空字串、空白、[]、null、數字回 null", () => {
    for (const x of ["", "  \n", "[]", "null", "42", '"str"']) expect(parseAnswer(x, arch, scan)).toBeNull();
  });
  it("解得出物件但 answer 不是非空字串仍回 null（不降級成純文字）", () => {
    expect(parseAnswer('{"walkthrough":{"steps":[]}}', arch, scan)).toBeNull();
    expect(parseAnswer('說明 {"answer":"  "} 結束', arch, scan)).toBeNull();
  });
});
