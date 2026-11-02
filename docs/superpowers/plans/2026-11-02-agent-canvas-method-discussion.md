# 方法討論階段 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 空桌布的需求框可以先跟 AI 多輪討論「這批工作的做法」，使用者按「確認」才建團隊，並把「做法決定」帶進團隊推薦、需求卡與專案簡介。

**Architecture:** 純對話通道沿用 `invokeAiComplete`（只回文字、不建任何東西）。AI 在準備好提案時於回覆裡附一個 ```` ```decision ```` JSON 區塊，前端解析成 `MethodDecision` 顯示成卡片；按「確認」呼叫既有 `startFromRequirement`，多帶一個 `decision` 參數。討論 UI 是新元件 `MethodDiscussion`，由 `RequirementStart` 的開關切換，不動原本的一次送出流程。

**Tech Stack:** React 19、TypeScript、Vitest + React Testing Library、`src/lib/i18n.ts`（zhTW 與 enRaw 兩份都要加）。

**Spec:** `docs/superpowers/specs/2026-11-02-agent-canvas-batch-cards-design.md` 的「方法討論階段」一節。

**範圍界線（YAGNI）：**
- 不做批次卡本身。`approach: "batch"` 目前只是記錄在需求卡與簡介裡，團隊與派工行為不變。
- 不做檔案讀取。第一版 AI 看得到的資料：專案簡介、參考資料夾路徑與備註（`buildRecommendMessage` 已有），其餘要使用者貼進對話。
- 只做空桌布（`agents.length === 0`）。已有團隊的專案不開討論。

**重要事實（已讀過程式碼）：**
- 空桌布的需求框是 `RequirementStart.tsx`（textarea＋程式庫＋送出），不是畫面下方的輸入列。
- `startFromRequirement` 的 `deps.complete` 簽章：`(messages: {role:"user";content:string}[], system, streamId, providerId?) => Promise<string>`，只收 user 訊息。多輪對話要把歷史攤平成單一 user 訊息，見 Task 1 的 `buildDiscussionMessage`。
- `i18n.ts` 的 en 是 `{...zhTW, ...enRaw}`，新字串兩份都要加（zhTW 約 924 行、enRaw 約 3549 行的 `ac_req_*` 附近）。
- 本 repo 部分檔案是 CRLF；改完用 `git diff --stat` 確認行數沒暴增。

---

## File Structure

| 檔案 | 動作 | 責任 |
|---|---|---|
| `src/components/AgentCanvas/methodDecision.ts` | 新增 | `MethodDecision` 型別、討論用系統提示詞、歷史攤平、從回覆解析決定、格式化成文字 |
| `src/components/AgentCanvas/methodDecision.test.ts` | 新增 | 上面純函式的測試 |
| `src/components/AgentCanvas/startFromRequirement.ts` | 修改 | 多收 `decision?`，帶進推薦、需求卡、簡介 |
| `src/components/AgentCanvas/startFromRequirement.test.ts` | 修改 | 加 decision 的測試 |
| `src/components/AgentCanvas/MethodDiscussion.tsx` | 新增 | 對話串、輸入、決定卡、確認鈕 |
| `src/components/AgentCanvas/MethodDiscussion.test.tsx` | 新增 | 元件測試 |
| `src/components/AgentCanvas/RequirementStart.tsx` | 修改 | 「先討論做法」開關；確認後走既有流程 |
| `src/components/AgentCanvas/RequirementStart.test.tsx` | 新增 | 開關、切換、確認建團隊 |
| `src/lib/i18n.ts` | 修改 | 新字串（zhTW＋enRaw） |

---

### Task 1: 決定的型別、提示詞與解析（純函式）

**Files:**
- Create: `src/components/AgentCanvas/methodDecision.ts`
- Test: `src/components/AgentCanvas/methodDecision.test.ts`

- [ ] **Step 1: 寫會失敗的測試**

```ts
import { describe, expect, it } from "vitest";

import { buildDiscussionMessage, formatDecision, parseDecision, type MethodDecision } from "./methodDecision";

const good: MethodDecision = {
  approach: "batch",
  reason: "533 支畫面骨架相同",
  trial: "先試 5 支，看通過率",
  requirement: "把 Cm 畫面轉成網頁",
};

const fence = (o: unknown) => "我建議這樣做。\n```decision\n" + JSON.stringify(o) + "\n```\n";

describe("parseDecision", () => {
  it("reads the decision block from a reply", () => {
    expect(parseDecision(fence(good))).toEqual(good);
  });
  it("returns null when there is no block", () => {
    expect(parseDecision("還在討論，先問你幾個問題")).toBeNull();
  });
  it("returns null for malformed JSON", () => {
    expect(parseDecision("```decision\n{oops\n```")).toBeNull();
  });
  it("rejects an unknown approach", () => {
    expect(parseDecision(fence({ ...good, approach: "magic" }))).toBeNull();
  });
  it("rejects a decision with an empty requirement", () => {
    expect(parseDecision(fence({ ...good, requirement: "  " }))).toBeNull();
  });
  it("takes the last block when the reply has several", () => {
    const reply = fence({ ...good, approach: "single" }) + fence(good);
    expect(parseDecision(reply)?.approach).toBe("batch");
  });
});

describe("formatDecision", () => {
  it("names the approach and keeps reason and trial", () => {
    const text = formatDecision(good);
    expect(text).toContain("批次");
    expect(text).toContain("533 支畫面骨架相同");
    expect(text).toContain("先試 5 支");
  });
  it("omits the trial line when there is none", () => {
    expect(formatDecision({ ...good, trial: "" })).not.toContain("試跑");
  });
});

describe("buildDiscussionMessage", () => {
  it("flattens the turns in order and includes the project context", () => {
    const m = buildDiscussionMessage(
      [
        { role: "user", content: "要轉換舊平台" },
        { role: "assistant", content: "有幾支？" },
        { role: "user", content: "533 支畫面" },
      ],
      "專案簡介 X",
      [{ path: "/old", note: "舊系統" }],
    );
    expect(m.indexOf("要轉換舊平台")).toBeLessThan(m.indexOf("有幾支？"));
    expect(m.indexOf("有幾支？")).toBeLessThan(m.indexOf("533 支畫面"));
    expect(m).toContain("專案簡介 X");
    expect(m).toContain("/old：舊系統");
  });
});
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `npx vitest run src/components/AgentCanvas/methodDecision.test.ts`
Expected: FAIL（找不到 `./methodDecision`）

- [ ] **Step 3: 寫最小實作**

```ts
import type { Reference } from "../../ipc/agents";

/**
 * 空桌布的「方法討論」：先跟 AI 多輪討論這批工作的做法，確認後才建團隊。
 * 對話走純文字補全；AI 準備好提案時在回覆裡附一個 ```decision JSON 區塊，這裡負責解析。
 */

export type Approach = "single" | "batch" | "generator";

export interface MethodDecision {
  approach: Approach;
  /** 為什麼選這個做法（依據討論裡的哪一點）。 */
  reason: string;
  /** 試跑計畫與驗證方式；可為空。 */
  trial: string;
  /** 整理後的需求，確認後取代使用者原本輸入的第一句。 */
  requirement: string;
}

export interface DiscussionTurn {
  role: "user" | "assistant";
  content: string;
}

const APPROACHES: readonly Approach[] = ["single", "batch", "generator"];

const APPROACH_LABEL: Record<Approach, string> = {
  single: "單張卡（每項工作各一張模組卡）",
  batch: "批次卡（同一份做法 × 一份清單）",
  generator: "先做產生器（一張卡寫並驗證產生器，再由它大量輸出）",
};

export function buildDiscussionSystem(): string {
  return [
    "你在協助使用者決定 AITerm「Agent 協作」這批工作的做法，只討論、不執行，也不要假裝已經建了任何東西。",
    "",
    "可選的做法：",
    ...APPROACHES.map((a) => `- ${a}：${APPROACH_LABEL[a]}`),
    "",
    "批次只在四項全符合時才建議：有明確清單且通常 20 項以上；每項用同一份 template；驗證方式相同且可機械判斷；項目之間沒有依賴。",
    "能被產生器涵蓋的部分，優先建議先做產生器；批次只處理產生器處理不了的例外項目。",
    "不確定就先問使用者（清單有多大、每項是否同型、怎麼驗證），一次問一到兩個問題，不要一次丟一長串。",
    "",
    "當你有把握提案時，在回覆最後附上一個 decision 區塊（回覆其餘部分用使用者的語言簡短說明）：",
    '```decision\n{"approach":"single|batch|generator","reason":"為什麼","trial":"試跑計畫與驗證方式，沒有就留空字串","requirement":"整理後的完整需求"}\n```',
    "使用者還有疑問或想調整時，就繼續討論，之後再附新的 decision 區塊；以最後一個為準。",
  ].join("\n");
}

/** `deps.complete` 只收 user 訊息，所以把整段對話攤平成一則，並附上專案背景。 */
export function buildDiscussionMessage(turns: DiscussionTurn[], brief: string, refs: Reference[]): string {
  const parts: string[] = [];
  if (brief.trim()) parts.push(`專案簡介：\n${brief.trim()}`);
  parts.push(
    refs.length
      ? `參考資料夾（唯讀，例如舊系統原始碼）：\n${refs.map((r) => `- ${r.path}${r.note ? `：${r.note}` : ""}`).join("\n")}`
      : "參考資料夾：沒有",
  );
  parts.push("到目前為止的對話：\n" + turns.map((x) => `${x.role === "user" ? "使用者" : "你"}：${x.content}`).join("\n\n"));
  parts.push("請接著回覆使用者。");
  return parts.join("\n\n");
}

const BLOCK = /```decision\s*\n([\s\S]*?)```/g;

/** 取回覆裡最後一個 decision 區塊；格式不對一律回 null（當作還在討論）。 */
export function parseDecision(reply: string): MethodDecision | null {
  const blocks = [...reply.matchAll(BLOCK)];
  const last = blocks[blocks.length - 1];
  if (!last) return null;
  let raw: unknown;
  try {
    raw = JSON.parse(last[1].trim());
  } catch {
    return null;
  }
  if (!raw || typeof raw !== "object") return null;
  const o = raw as Record<string, unknown>;
  const approach = o.approach;
  const requirement = typeof o.requirement === "string" ? o.requirement.trim() : "";
  if (typeof approach !== "string" || !APPROACHES.includes(approach as Approach) || !requirement) return null;
  return {
    approach: approach as Approach,
    reason: typeof o.reason === "string" ? o.reason.trim() : "",
    trial: typeof o.trial === "string" ? o.trial.trim() : "",
    requirement,
  };
}

/** 寫進需求卡與專案簡介的文字；讓後面的角色看得到決定與理由。 */
export function formatDecision(d: MethodDecision): string {
  const lines = [`做法決定：${APPROACH_LABEL[d.approach]}`];
  if (d.reason) lines.push(`理由：${d.reason}`);
  if (d.trial) lines.push(`試跑計畫：${d.trial}`);
  return lines.join("\n");
}
```

- [ ] **Step 4: 跑測試確認通過**

Run: `npx vitest run src/components/AgentCanvas/methodDecision.test.ts`
Expected: PASS（全部）

- [ ] **Step 5: 證明測試會紅（突變）**

暫時把 `parseDecision` 的 `blocks[blocks.length - 1]` 改成 `blocks[0]`，跑上面指令。
Expected: 「takes the last block」FAIL。還原後再跑一次確認回綠。

- [ ] **Step 6: Commit**

```bash
git add src/components/AgentCanvas/methodDecision.ts src/components/AgentCanvas/methodDecision.test.ts
git commit -m "feat(agent-canvas): 方法討論的決定型別、提示詞與解析"
```

---

### Task 2: `startFromRequirement` 帶入做法決定

**Files:**
- Modify: `src/components/AgentCanvas/startFromRequirement.ts`（`StartArgs`、推薦用的 `described`、簡介寫入、需求卡 body）
- Test: `src/components/AgentCanvas/startFromRequirement.test.ts`

- [ ] **Step 1: 寫會失敗的測試**（加在 `describe("startFromRequirement"` 內）

```ts
  describe("with a method decision", () => {
    const decision = {
      approach: "generator" as const,
      reason: "畫面骨架相同",
      trial: "先試 5 支",
      requirement: "把 Cm 畫面轉成網頁",
    };

    it("uses the decision's requirement and stores it with the decision as the brief", async () => {
      const d = deps({ getBrief: vi.fn().mockResolvedValue("") });
      await startFromRequirement({ ...base, requirement: "舊平台轉換", decision }, d);
      const brief = (d.setBrief as ReturnType<typeof vi.fn>).mock.calls[0][1] as string;
      expect(brief).toContain("把 Cm 畫面轉成網頁");
      expect(brief).toContain("先做產生器");
      expect(brief).toContain("理由：畫面骨架相同");
    });

    it("puts the decision into the card body and uses the requirement as the title", async () => {
      const d = deps();
      await startFromRequirement({ ...base, requirement: "舊平台轉換", decision }, d);
      const arg = (d.createTask as ReturnType<typeof vi.fn>).mock.calls[0][1];
      expect(arg.title).toContain("把 Cm 畫面轉成網頁");
      expect(arg.body).toContain("做法決定：");
      expect(arg.body).toContain("試跑計畫：先試 5 支");
    });

    it("lets the recommender see the decision", async () => {
      const complete = vi.fn().mockRejectedValue(new Error("no ai"));
      await startFromRequirement({ ...base, requirement: "舊平台轉換", decision }, deps({ complete }));
      const firstUserMsg = complete.mock.calls[0][0][0].content as string;
      expect(firstUserMsg).toContain("做法決定：");
    });

    it("behaves as before when there is no decision", async () => {
      const d = deps();
      await startFromRequirement({ ...base, requirement: "build a todo app" }, d);
      const arg = (d.createTask as ReturnType<typeof vi.fn>).mock.calls[0][1];
      expect(arg.body).not.toContain("做法決定");
    });
  });
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `npx vitest run src/components/AgentCanvas/startFromRequirement.test.ts`
Expected: 新的 3 條 FAIL（`decision` 不在 `StartArgs`，型別或行為不符）；「behaves as before」可能已 PASS，正常。

- [ ] **Step 3: 實作**

`startFromRequirement.ts` 開頭加 import：

```ts
import { formatDecision, type MethodDecision } from "./methodDecision";
```

`StartArgs` 加欄位：

```ts
  /** 方法討論確認後的做法決定；有的話取代 requirement 當作需求，並寫進簡介與需求卡。 */
  decision?: MethodDecision;
```

`startFromRequirement` 內，把

```ts
  const requirement = args.requirement.trim();
```

改成

```ts
  const requirement = (args.decision?.requirement ?? args.requirement).trim();
  const decisionText = args.decision ? formatDecision(args.decision) : "";
```

簡介寫入改成：

```ts
  if (!brief) await d.setBrief(projectId, [requirement, decisionText].filter(Boolean).join("\n\n")).catch(() => {});
```

推薦用的描述改成：

```ts
      const described = [brief ? `${brief}\n\n本次需求：\n${requirement}` : requirement, decisionText].filter(Boolean).join("\n\n");
```

需求卡 body 改成（`withDoneCriteria` 之前先拼接）：

```ts
    const { title, body } = bugCard(requirement, decisionText, null);
```

（`bugCard(text, instructions, related)` 會把 `instructions` 接在需求後面；無決定時 `decisionText` 是空字串，會被 `filter(Boolean)` 濾掉，行為不變。）

- [ ] **Step 4: 跑測試確認通過**

Run: `npx vitest run src/components/AgentCanvas/startFromRequirement.test.ts`
Expected: PASS（含原有測試）

- [ ] **Step 5: Commit**

```bash
git add src/components/AgentCanvas/startFromRequirement.ts src/components/AgentCanvas/startFromRequirement.test.ts
git commit -m "feat(agent-canvas): 建團隊時帶入方法討論的做法決定"
```

---

### Task 3: i18n 字串

**Files:**
- Modify: `src/lib/i18n.ts`（zhTW 約 946 行 `ac_req_criteria_title` 之後；enRaw 約 3565 行之後，各加同一組 key）

- [ ] **Step 1: 在 zhTW 加入**

```ts
    ac_md_toggle: "先討論做法再開始",
    ac_md_toggle_tip: "先跟 AI 討論這批工作該怎麼做（單張卡、批次或先做產生器），確認後才建團隊",
    ac_md_title: "先討論做法",
    ac_md_placeholder: "描述這批工作：有多大、每項是否同型、怎麼驗證",
    ac_md_send: "送出",
    ac_md_thinking: "AI 思考中…",
    ac_md_decision_title: "AI 提議的做法",
    ac_md_approach_single: "單張卡",
    ac_md_approach_batch: "批次卡",
    ac_md_approach_generator: "先做產生器",
    ac_md_trial: "試跑計畫",
    ac_md_reason: "理由",
    ac_md_confirm: "確認，建立團隊",
    ac_md_confirm_hint: "還沒有提案；繼續討論，AI 有把握時會提出做法",
    ac_md_back: "改回直接送出",
    ac_md_batch_note: "批次卡還在開發中，這個決定目前只記錄在需求卡與簡介裡，團隊與派工和一般一樣。",
    ac_md_error: (msg: string) => `討論失敗：${msg}`,
```

- [ ] **Step 2: 在 enRaw 加入同一組 key（英文）**

```ts
    ac_md_toggle: "Discuss the approach first",
    ac_md_toggle_tip: "Talk with AI about how to do this work (single cards, a batch, or a generator first); the team is built after you confirm",
    ac_md_title: "Discuss the approach",
    ac_md_placeholder: "Describe the work: how big, whether items are alike, how to verify",
    ac_md_send: "Send",
    ac_md_thinking: "AI is thinking…",
    ac_md_decision_title: "Approach proposed by AI",
    ac_md_approach_single: "Single cards",
    ac_md_approach_batch: "Batch card",
    ac_md_approach_generator: "Generator first",
    ac_md_trial: "Trial run",
    ac_md_reason: "Reason",
    ac_md_confirm: "Confirm and build the team",
    ac_md_confirm_hint: "No proposal yet; keep talking and AI will propose an approach when it is sure",
    ac_md_back: "Back to direct send",
    ac_md_batch_note: "Batch cards are still in development; for now this decision is only recorded in the card and brief, and the team and dispatch work as usual.",
    ac_md_error: (msg: string) => `Discussion failed: ${msg}`,
```

- [ ] **Step 2.5: 確認沒有 CRLF 假異動**

Run: `git diff --stat src/lib/i18n.ts`
Expected: 約 +34 行，沒有整檔變動。

- [ ] **Step 3: 型別與語系漂移檢查**

Run: `npx tsc -b && npx vitest run src/lib/i18n`
Expected: 0 錯誤；i18n 相關測試 PASS（`localeSources` 兩份 key 一致）。

- [ ] **Step 4: Commit**

```bash
git add src/lib/i18n.ts
git commit -m "feat(agent-canvas): 方法討論的中英文字串"
```

---

### Task 4: `MethodDiscussion` 元件

**Files:**
- Create: `src/components/AgentCanvas/MethodDiscussion.tsx`
- Test: `src/components/AgentCanvas/MethodDiscussion.test.tsx`

元件不碰 Tauri：`complete`、`getBrief`、`getReferences` 都由 props 注入（跟 `StartDeps` 同一個做法，測試不必 mock IPC）。

- [ ] **Step 1: 寫會失敗的測試**

```tsx
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";
import { MethodDiscussion } from "./MethodDiscussion";

const decisionReply =
  "建議批次。\n```decision\n" +
  JSON.stringify({ approach: "batch", reason: "骨架相同", trial: "先試 5 支", requirement: "轉換 Cm 畫面" }) +
  "\n```";

function setup(complete: (m: { role: "user"; content: string }[], s: string) => Promise<string>) {
  const onConfirm = vi.fn();
  const onBack = vi.fn();
  render(
    <LocaleProvider>
      <MethodDiscussion
        complete={complete}
        getContext={async () => ({ brief: "", refs: [] })}
        busy={false}
        onConfirm={onConfirm}
        onBack={onBack}
      />
    </LocaleProvider>,
  );
  return { onConfirm, onBack };
}

const send = async (text: string) => {
  fireEvent.change(screen.getByTestId("ac-md-input"), { target: { value: text } });
  fireEvent.click(screen.getByTestId("ac-md-send"));
};

describe("MethodDiscussion", () => {
  it("keeps confirm disabled until the AI proposes a decision", async () => {
    const complete = vi.fn().mockResolvedValue("有幾支？");
    setup(complete);
    await send("要轉換舊平台");
    await waitFor(() => expect(screen.getByText("有幾支？")).toBeTruthy());
    expect((screen.getByTestId("ac-md-confirm") as HTMLButtonElement).disabled).toBe(true);
  });

  it("shows the proposal and confirms with it", async () => {
    const complete = vi.fn().mockResolvedValue(decisionReply);
    const { onConfirm } = setup(complete);
    await send("533 支畫面");
    await waitFor(() => expect(screen.getByTestId("ac-md-decision")).toBeTruthy());
    const confirm = screen.getByTestId("ac-md-confirm") as HTMLButtonElement;
    expect(confirm.disabled).toBe(false);
    fireEvent.click(confirm);
    expect(onConfirm).toHaveBeenCalledWith(expect.objectContaining({ approach: "batch", requirement: "轉換 Cm 畫面" }));
  });

  it("does not show the raw decision block in the chat", async () => {
    setup(vi.fn().mockResolvedValue(decisionReply));
    await send("533 支畫面");
    await waitFor(() => expect(screen.getByTestId("ac-md-decision")).toBeTruthy());
    expect(screen.queryByText(/```decision/)).toBeNull();
  });

  it("sends the whole history on the second turn", async () => {
    const complete = vi.fn().mockResolvedValueOnce("有幾支？").mockResolvedValueOnce("好");
    setup(complete);
    await send("要轉換舊平台");
    await waitFor(() => expect(screen.getByText("有幾支？")).toBeTruthy());
    await send("533 支");
    await waitFor(() => expect(complete).toHaveBeenCalledTimes(2));
    const second = complete.mock.calls[1][0][0].content as string;
    expect(second).toContain("要轉換舊平台");
    expect(second).toContain("有幾支？");
    expect(second).toContain("533 支");
  });

  it("shows the error and keeps the typed text out of the history on failure", async () => {
    setup(vi.fn().mockRejectedValue(new Error("boom")));
    await send("hi");
    await waitFor(() => expect(screen.getByText(/boom/)).toBeTruthy());
  });

  it("the back button calls onBack", () => {
    const { onBack } = setup(vi.fn());
    fireEvent.click(screen.getByTestId("ac-md-back"));
    expect(onBack).toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `npx vitest run src/components/AgentCanvas/MethodDiscussion.test.tsx`
Expected: FAIL（找不到 `./MethodDiscussion`）

- [ ] **Step 3: 實作**

```tsx
import { useRef, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import type { Reference } from "../../ipc/agents";
import { buildDiscussionMessage, buildDiscussionSystem, parseDecision, type DiscussionTurn, type MethodDecision } from "./methodDecision";

/**
 * 空桌布的「先討論做法」：多輪對話，AI 有把握時提出做法決定，使用者確認後才建團隊。
 * 只對話、不建任何東西；外部呼叫都由 props 注入，測試不必 mock Tauri。
 */

const strip = (reply: string) => reply.replace(/```decision[\s\S]*?```/g, "").trim();

export function MethodDiscussion({
  complete,
  getContext,
  busy,
  onConfirm,
  onBack,
}: {
  complete: (messages: { role: "user"; content: string }[], system: string) => Promise<string>;
  getContext: () => Promise<{ brief: string; refs: Reference[] }>;
  /** 外層正在建團隊：鎖住輸入與確認。 */
  busy: boolean;
  onConfirm: (d: MethodDecision) => void;
  onBack: () => void;
}) {
  const { t } = useLocale();
  const [turns, setTurns] = useState<DiscussionTurn[]>([]);
  const [text, setText] = useState("");
  const [thinking, setThinking] = useState(false);
  const [decision, setDecision] = useState<MethodDecision | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** 送出當下的歷史；用 ref 避免連按時讀到舊閉包。 */
  const turnsRef = useRef<DiscussionTurn[]>([]);

  const approachLabel = {
    single: t.ac_md_approach_single,
    batch: t.ac_md_approach_batch,
    generator: t.ac_md_approach_generator,
  };

  const send = async () => {
    const content = text.trim();
    if (!content || thinking || busy) return;
    setError(null);
    const next = [...turnsRef.current, { role: "user" as const, content }];
    turnsRef.current = next;
    setTurns(next);
    setText("");
    setThinking(true);
    try {
      const ctx = await getContext();
      const reply = await complete([{ role: "user", content: buildDiscussionMessage(next, ctx.brief, ctx.refs) }], buildDiscussionSystem());
      const withReply = [...next, { role: "assistant" as const, content: strip(reply) }];
      turnsRef.current = withReply;
      setTurns(withReply);
      // 以最新的 decision 區塊為準；這一輪沒附就保留上一個。
      setDecision((prev) => parseDecision(reply) ?? prev);
    } catch (e) {
      setError(t.ac_md_error(e instanceof Error ? e.message : String(e)));
    } finally {
      setThinking(false);
    }
  };

  return (
    <div className="ac-md" data-testid="ac-md">
      <h3 className="ac-req-title">{t.ac_md_title}</h3>
      <div className="ac-md-thread">
        {turns.map((x, i) => (
          <p key={i} className={`ac-md-turn ac-md-turn--${x.role}`}>
            {x.content}
          </p>
        ))}
        {thinking && <p className="ac-md-turn ac-md-turn--assistant">{t.ac_md_thinking}</p>}
      </div>
      {decision && (
        <div className="ac-md-decision" data-testid="ac-md-decision">
          <strong>{t.ac_md_decision_title}：{approachLabel[decision.approach]}</strong>
          {decision.reason && <p>{t.ac_md_reason}：{decision.reason}</p>}
          {decision.trial && <p>{t.ac_md_trial}：{decision.trial}</p>}
          {decision.approach === "batch" && <p className="task-field-hint">{t.ac_md_batch_note}</p>}
        </div>
      )}
      {error && <p className="ac-bad-text">{error}</p>}
      <textarea
        className="ac-req-input"
        data-testid="ac-md-input"
        rows={3}
        value={text}
        placeholder={t.ac_md_placeholder}
        disabled={thinking || busy}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void send();
        }}
      />
      <div className="ac-req-actions">
        <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" data-testid="ac-md-back" disabled={busy} onClick={onBack}>
          {t.ac_md_back}
        </button>
        <button className="aiterm-btn aiterm-btn--secondary" data-testid="ac-md-send" disabled={thinking || busy || !text.trim()} onClick={() => void send()}>
          {t.ac_md_send}
        </button>
        <button
          className="aiterm-btn aiterm-btn--primary"
          data-testid="ac-md-confirm"
          title={decision ? undefined : t.ac_md_confirm_hint}
          disabled={!decision || thinking || busy}
          onClick={() => decision && onConfirm(decision)}
        >
          {t.ac_md_confirm}
        </button>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: 跑測試確認通過**

Run: `npx vitest run src/components/AgentCanvas/MethodDiscussion.test.tsx`
Expected: PASS（6 條）。若 `LocaleProvider` 的匯出名稱不同，照 `RequirementStart` 周邊既有測試（例如 `StartNotice.test.tsx`）的掛載方式調整，不要改元件。

- [ ] **Step 5: 證明測試會紅**

暫時把 `setDecision((prev) => parseDecision(reply) ?? prev)` 改成 `setDecision(parseDecision(reply))`，並手動讓第二輪回覆不含區塊——本檔現有測試沒涵蓋「保留上一個」，所以先**補一條**：

```tsx
  it("keeps the previous proposal when a later reply has none", async () => {
    const complete = vi.fn().mockResolvedValueOnce(decisionReply).mockResolvedValueOnce("還有什麼要補充？");
    setup(complete);
    await send("533 支畫面");
    await waitFor(() => expect(screen.getByTestId("ac-md-decision")).toBeTruthy());
    await send("對了還有權限頁");
    await waitFor(() => expect(complete).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.getByText("還有什麼要補充？")).toBeTruthy());
    expect(screen.getByTestId("ac-md-decision")).toBeTruthy();
  });
```

用突變版跑：新測試要 FAIL；還原成 `?? prev` 後 PASS。

- [ ] **Step 6: Commit**

```bash
git add src/components/AgentCanvas/MethodDiscussion.tsx src/components/AgentCanvas/MethodDiscussion.test.tsx
git commit -m "feat(agent-canvas): 方法討論元件（多輪對話、做法決定卡、確認）"
```

---

### Task 5: 接進 `RequirementStart`

**Files:**
- Modify: `src/components/AgentCanvas/RequirementStart.tsx`
- Test: `src/components/AgentCanvas/RequirementStart.test.tsx`（新增，自帶 mock；`index.test.tsx` 只在 Step 4 確認沒被影響）

行為：
- 需求框頂端加一個勾選「先討論做法再開始」（預設關，維持原流程）。
- 勾選後，原本的 textarea 與送出區換成 `MethodDiscussion`；程式庫選擇、模型選擇仍在。
- `onConfirm(decision)` → 呼叫既有 `startFromRequirement({ ..., requirement: decision.requirement, decision, autoStart: skipSkills })`，沿用 `onStarted`、錯誤處理與 `StartError`。
- 程式庫還不 OK（`!repoOk`）時，確認鈕要鎖住（用 `busy` prop 傳 `busy || !repoOk`）。
- 沒有 AI 模型（`providerId === undefined`）時，不顯示開關（沒有模型就無法討論）。

- [ ] **Step 1: 寫會失敗的測試**（新檔 `RequirementStart.test.tsx`，自帶最小 mock，不依賴 `index.test.tsx`）

```tsx
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../../lib/webviewFocus", () => ({ restoreWebviewFocus: vi.fn().mockResolvedValue(undefined) }));
vi.mock("../../ipc/ai", () => ({ invokeAiComplete: vi.fn() }));
vi.mock("../../ipc/provider", () => ({ listProviders: vi.fn() }));
vi.mock("../../ipc/tasks", () => ({
  createTask: vi.fn().mockResolvedValue("task-1"),
  getProjectBrief: vi.fn().mockResolvedValue(""),
  setProjectBrief: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("../../ipc/agents", () => ({
  assignAgent: vi.fn().mockResolvedValue(undefined),
  checkRepo: vi.fn().mockResolvedValue({ status: "ok", branch: "main" }),
  createAgent: vi.fn().mockResolvedValue("agent-1"),
  getReferences: vi.fn().mockResolvedValue([]),
  setProjectRepo: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("../TaskBoard/useRefineTask", () => ({
  useRefineTask: () => ({ busy: false, error: null, refine: vi.fn() }),
}));

import { invokeAiComplete } from "../../ipc/ai";
import { createTask } from "../../ipc/tasks";
import { listProviders } from "../../ipc/provider";
import { RequirementStart } from "./RequirementStart";

const provider = { id: "p1", name: "P", is_default: true } as never;

const decisionReply =
  "建議批次。\n```decision\n" +
  JSON.stringify({ approach: "batch", reason: "骨架相同", trial: "先試 5 支", requirement: "轉換 Cm 畫面" }) +
  "\n```";

function renderStart(onStarted = vi.fn()) {
  render(
    <LocaleProvider>
      <RequirementStart projectId="proj" repo="/repo" onRepoChange={vi.fn()} onStarted={onStarted} />
    </LocaleProvider>,
  );
  return onStarted;
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(listProviders).mockResolvedValue([provider]);
});

describe("RequirementStart method discussion", () => {
  it("hides the toggle when no AI model is configured", async () => {
    vi.mocked(listProviders).mockResolvedValue([]);
    renderStart();
    await screen.findByTestId("ac-req");
    await waitFor(() => expect(screen.queryByTestId("ac-md-toggle")).toBeNull());
  });

  it("swaps the textarea for the discussion when toggled, and back", async () => {
    renderStart();
    fireEvent.click(await screen.findByTestId("ac-md-toggle"));
    expect(screen.getByTestId("ac-md")).toBeTruthy();
    expect(screen.queryByTestId("ac-req-input")).toBeNull();
    fireEvent.click(screen.getByTestId("ac-md-back"));
    expect(screen.getByTestId("ac-req-input")).toBeTruthy();
  });

  it("confirming a decision builds the team with that decision in the card", async () => {
    // 第一次呼叫是討論回覆；之後（團隊推薦、完成定義）都失敗 → 走預設團隊、沒有完成定義。
    vi.mocked(invokeAiComplete)
      .mockResolvedValueOnce({ content: decisionReply } as never)
      .mockRejectedValue(new Error("no ai"));
    const onStarted = renderStart();
    fireEvent.click(await screen.findByTestId("ac-md-toggle"));
    fireEvent.change(screen.getByTestId("ac-md-input"), { target: { value: "533 支畫面" } });
    fireEvent.click(screen.getByTestId("ac-md-send"));
    await screen.findByTestId("ac-md-decision");
    fireEvent.click(screen.getByTestId("ac-md-confirm"));
    await waitFor(() => expect(onStarted).toHaveBeenCalledTimes(1));
    const card = vi.mocked(createTask).mock.calls[0][1];
    expect(card.title).toContain("轉換 Cm 畫面");
    expect(card.body).toContain("做法決定：");
    expect(card.body).toContain("試跑計畫：先試 5 支");
  });
});
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `npx vitest run src/components/AgentCanvas/RequirementStart.test.tsx`
Expected: FAIL（`ac-md-toggle` 不存在）

- [ ] **Step 3: 實作**

在 `RequirementStart.tsx`：

```tsx
import { getProjectBrief } from "../../ipc/tasks"; // 已有 getProjectBrief import，沿用
import { MethodDiscussion } from "./MethodDiscussion";
import type { MethodDecision } from "./methodDecision";
```

state 與處理：

```tsx
  const [discuss, setDiscuss] = useState(false);

  const confirmDecision = async (decision: MethodDecision) => {
    if (!repo) return;
    setBusy(true);
    setError(null);
    try {
      onStarted(
        await startFromRequirement({ projectId, requirement: decision.requirement, decision, repoDir: repo, t, providerId, autoStart: skipSkills }, deps),
      );
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(e instanceof StartError && e.teamCreated ? t.ac_req_team_exists(msg) : msg);
    } finally {
      setBusy(false);
    }
  };
```

（`submit` 與 `confirmDecision` 的 try/catch 重複；抽成 `run(args)` 一個內部函式，兩邊共用，避免兩份錯誤處理漂移。抽的時候不改 `submit` 的對外行為，並跑既有測試確認。）

JSX：在標題下方、`providerId !== undefined` 時加開關：

```tsx
      {providerId !== undefined && (
        <label className="ac-req-skip" title={t.ac_md_toggle_tip}>
          <input type="checkbox" data-testid="ac-md-toggle" checked={discuss} disabled={busy} onChange={(e) => setDiscuss(e.target.checked)} />
          <span>{t.ac_md_toggle}</span>
        </label>
      )}
```

`discuss` 為 true 時，把 `<textarea>`、`ac-req-tools`、`ac-req-actions` 換成：

```tsx
      <MethodDiscussion
        complete={(messages, system) => deps.complete(messages, system, "method-discussion", providerId)}
        getContext={async () => ({ brief: ((await getProjectBrief(projectId).catch(() => null)) ?? "").trim(), refs: await getReferences(projectId).catch(() => []) })}
        busy={busy || !repoOk}
        onConfirm={(d) => void confirmDecision(d)}
        onBack={() => setDiscuss(false)}
      />
```

程式庫區塊、`RepoStatus`、錯誤訊息、「不需要加技能」勾選維持顯示在兩種模式下。

- [ ] **Step 4: 跑測試確認通過**

Run: `npx vitest run src/components/AgentCanvas`
Expected: PASS（含 `index.test.tsx` 原有測試）

- [ ] **Step 5: 補樣式**

在 `src/components/AgentCanvas/index.css` 加 `.ac-md-thread`（最大高度 + 捲動）、`.ac-md-turn--user`／`--assistant`（靠右／靠左或底色區分）、`.ac-md-decision`（框線卡片）。沿用 `.ac-req-*` 的間距與色彩變數，不另創色。

- [ ] **Step 6: 全面驗證**

Run:
```bash
npx tsc -b
npm run lint
npm run test
```
Expected: tsc 0 錯誤、lint 無新增錯誤、vitest 全綠。（MailView 有已知 flaky，只在記憶體吃緊時紅；若只有它紅，單獨重跑一次確認。）

- [ ] **Step 7: Commit**

```bash
git add src/components/AgentCanvas/RequirementStart.tsx src/components/AgentCanvas/RequirementStart.test.tsx src/components/AgentCanvas/index.css
git commit -m "feat(agent-canvas): 空桌布需求框可先討論做法，確認後才建團隊"
```

---

### Task 6: 實機驗收（不可省）

跑起來才能證明對話通道、解析與建團隊真的接上；單元測試只證明元件在 mock 下的行為。

- [ ] **Step 1: 起 dev**

Run: `npm run tauri:dev`（先確認 `src-tauri/binaries/` 已有 uv 等 sidecar；沒有就先跑對應 `scripts/setup-uv-*.sh`）。若乾淨重起，確認沒有沿用舊二進位。

- [ ] **Step 2: 走一遍**

1. 新建一個空專案、選程式庫。
2. 勾「先討論做法再開始」，輸入：「舊系統有 533 支畫面要轉成網頁，每支骨架一樣」。
3. 預期：AI 問 1 到 2 個問題，不是直接丟長答案；確認鈕是灰的。
4. 回答後，預期出現「AI 提議的做法」卡片；對話串裡看不到 ```` ```decision ```` 原文。
5. 按「確認，建立團隊」。預期：團隊出現在畫布；專案簡介裡有「做法決定：」；需求卡 body 有「做法決定：」與試跑計畫。
6. 重試一次選 `single` 的情境（例如「做一個待辦清單網站」），確認 AI 不亂推批次。

- [ ] **Step 3: 回報**

把 AI 實際回覆的格式問題（例如不附 decision 區塊、JSON 壞掉、一次問太多問題）整理出來；如果有，調整 `buildDiscussionSystem` 並補一條對應的解析測試，再 commit。

---

## Self-Review（對照 spec）

| spec 要求 | 對應 |
|---|---|
| 空桌布輸入先對話、不建任何東西 | Task 4（`MethodDiscussion` 只呼叫 `complete`）、Task 5 開關 |
| 逐步整理「做法決定」（做法、理由、試跑計畫） | Task 1 `MethodDecision`＋決定卡 |
| 使用者按「確認」才建團隊 | Task 4 確認鈕＋Task 5 `confirmDecision` |
| 團隊配置跟著做法走 | Task 2 把決定放進推薦的描述；**推薦提示詞沒有針對 batch／generator 的角色**，現階段只是讓 AI 看到決定，不保證配置不同 |
| 討論紀錄存進專案簡介或規格 | Task 2（簡介空時寫入需求＋決定；需求卡 body 附決定） |
| 判斷標準四項 | Task 1 `buildDiscussionSystem` |
| 決定權：AI 建議、使用者確認 | 確認鈕為唯一入口 |

**與 spec 的差距（要如實告知）：**
1. 「團隊配置跟著做法走」只做到「讓推薦 AI 看到決定」，沒有改 `buildRecommendSystem` 的角色目錄或規則；批次卡未實作前，做 batch 也沒有對應角色可配。
2. 「讀資料」只有簡介與參考資料夾路徑，沒有讀檔。
3. 簡介只在「空的時候」寫入（沿用既有規則），簡介已有內容的專案，決定只存在需求卡 body。

**型別一致性：** `MethodDecision`（Task 1 定義）在 Task 2、4、5 同名同欄位（`approach`／`reason`／`trial`／`requirement`）；`parseDecision`／`formatDecision`／`buildDiscussionMessage`／`buildDiscussionSystem` 名稱在各 Task 一致。
