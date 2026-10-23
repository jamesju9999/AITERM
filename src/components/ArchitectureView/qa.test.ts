import { describe, expect, it } from "vitest";
import {
  ANSWER_SYSTEM, SELECT_SYSTEM, archSummary, buildAnswerMessage, buildSelectMessage, parseAnswer, parseSelect,
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
    expect(parseAnswer("hello", arch, scan)).toBeNull();
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
