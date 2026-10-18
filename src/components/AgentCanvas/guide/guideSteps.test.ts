import { beforeEach, describe, expect, it } from "vitest";

import {
  DEFAULT_GUIDE,
  STEP_IDS,
  contextHint,
  loadGuide,
  observedSteps,
  saveGuide,
  withObserved,
  type GuideAgent,
  type GuideCard,
} from "./guideSteps";

const ag = (id: string, kind: GuideAgent["kind"]): GuideAgent => ({ id, kind });
const cd = (over: Partial<GuideCard>): GuideCard => ({ card_kind: "work", agent_id: null, status: "planning", outcome: null, ...over });

beforeEach(() => localStorage.clear());

describe("observedSteps", () => {
  it("每一步都從桌布狀態判斷", () => {
    expect(observedSteps([], [])).toEqual(new Set());
    expect(observedSteps([ag("c", "coder")], [])).toEqual(new Set(["coder"]));
    expect(observedSteps([], [cd({})])).toEqual(new Set(["module"]));
    expect(observedSteps([], [cd({ agent_id: "c" })])).toEqual(new Set(["module", "assign"]));
    expect(observedSteps([ag("r", "reviewer"), ag("o", "orchestrator"), ag("v", "verifier")], [])).toEqual(
      new Set(["reviewer", "orchestrator", "verifier"]),
    );
    expect(observedSteps([], [cd({ card_kind: "merge", status: "done", outcome: "success" })])).toEqual(new Set(["merged"]));
    expect(observedSteps([], [cd({ card_kind: "verify", status: "done", outcome: "failed" })])).toEqual(new Set(["verified"]));
  });

  it("不是模組的卡、沒成功的合併、還在跑的驗收都不算", () => {
    expect(observedSteps([], [cd({ card_kind: "review", agent_id: "r" })])).toEqual(new Set());
    expect(observedSteps([], [cd({ card_kind: "merge", status: "done", outcome: "failed" })])).toEqual(new Set());
    expect(observedSteps([], [cd({ card_kind: "merge", status: "running" })])).toEqual(new Set());
    expect(observedSteps([], [cd({ card_kind: "verify", status: "running" })])).toEqual(new Set());
  });
});

describe("withObserved", () => {
  it("只增不減、不重複；沒有新的就回傳同一個物件", () => {
    const s = { ...DEFAULT_GUIDE, done: ["coder" as const] };
    const next = withObserved(s, new Set(["module"]));
    expect(next.done).toEqual(["coder", "module"]);
    expect(withObserved(next, new Set(["coder"]))).toBe(next);
    expect(withObserved(next, new Set())).toBe(next);
  });
});

describe("loadGuide / saveGuide", () => {
  it("沒資料、壞 JSON、型別不對都回預設值", () => {
    expect(loadGuide()).toEqual(DEFAULT_GUIDE);
    localStorage.setItem("aiterm.agentCanvas.guide", "{oops");
    expect(loadGuide()).toEqual(DEFAULT_GUIDE);
    localStorage.setItem("aiterm.agentCanvas.guide", JSON.stringify({ tourSeen: "yes", done: "coder" }));
    expect(loadGuide()).toEqual(DEFAULT_GUIDE);
  });

  it("存了再讀回來一樣；未知的步驟與提示過濾掉", () => {
    const s = { tourSeen: true, checklistHidden: false, hintsOff: true, done: ["coder" as const, "merged" as const], dismissedHints: ["add_module" as const] };
    saveGuide(s);
    expect(loadGuide()).toEqual(s);
    localStorage.setItem("aiterm.agentCanvas.guide", JSON.stringify({ ...s, done: ["coder", "bogus"], dismissedHints: ["nope"] }));
    expect(loadGuide()).toEqual({ ...s, done: ["coder"], dismissedHints: [] });
  });

  it("八步的順序固定", () => {
    expect(STEP_IDS).toEqual(["coder", "module", "assign", "reviewer", "orchestrator", "verifier", "merged", "verified"]);
  });
});

describe("contextHint", () => {
  it("沒有工程師不給提示（空桌布有自己的引導）", () => {
    expect(contextHint([], [])).toBeNull();
    expect(contextHint([ag("r", "reviewer")], [])).toBeNull();
  });

  it("有工程師沒模組：指向模組庫", () => {
    expect(contextHint([ag("c", "coder")], [])).toEqual({ id: "add_module", anchor: { guide: "library" } });
  });

  it("模組庫有模組、有工程師手上沒模組：指向那位工程師", () => {
    const agents = [ag("c1", "coder"), ag("c2", "coder")];
    const tasks = [cd({ agent_id: "c1" }), cd({})];
    expect(contextHint(agents, tasks)).toEqual({ id: "assign_module", anchor: { agent: "c2" } });
  });

  it("模組都在工程師手上時，依序提示審核者、協調者、驗收者", () => {
    const tasks = [cd({ agent_id: "c" })];
    expect(contextHint([ag("c", "coder")], tasks)).toEqual({ id: "add_reviewer", anchor: { guide: "pool-reviewer" } });
    expect(contextHint([ag("c", "coder"), ag("r", "reviewer")], tasks)).toEqual({ id: "add_orchestrator", anchor: { guide: "pool-orchestrator" } });
    expect(contextHint([ag("c", "coder"), ag("r", "reviewer"), ag("o", "orchestrator")], tasks)).toEqual({
      id: "add_verifier",
      anchor: { guide: "pool-verifier" },
    });
    expect(contextHint([ag("c", "coder"), ag("r", "reviewer"), ag("o", "orchestrator"), ag("v", "verifier")], tasks)).toBeNull();
  });

  it("前面的條件優先：還有沒拿到模組的工程師時，先指派再談審核者", () => {
    const agents = [ag("c1", "coder"), ag("c2", "coder")];
    const tasks = [cd({ agent_id: "c1" }), cd({})];
    expect(contextHint(agents, tasks)?.id).toBe("assign_module");
    // 模組庫是空的就沒得指派，c2 閒著也不提示指派。
    expect(contextHint(agents, [cd({ agent_id: "c1" })])?.id).toBe("add_reviewer");
  });

  it("模組庫只算計畫中、沒指派的 work 卡", () => {
    const agents = [ag("c1", "coder"), ag("c2", "coder")];
    const tasks = [cd({ agent_id: "c1" }), cd({ card_kind: "review" }), cd({ status: "done" })];
    expect(contextHint(agents, tasks)?.id).toBe("add_reviewer");
  });
});
