/**
 * 空桌布的「方法討論」：先跟 AI 多輪討論這批工作的做法，確認後才建團隊。
 * 對話在真的 Claude Code session 裡進行；AI 把決定寫進 JSON 檔，這裡負責提示詞與解析。
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

const APPROACHES: readonly Approach[] = ["single", "batch", "generator"];

const APPROACH_LABEL: Record<Approach, string> = {
  single: "單張卡（每項工作各一張模組卡）",
  batch: "批次卡（同一份做法 × 一份清單）",
  generator: "先做產生器（一張卡寫並驗證產生器，再由它大量輸出）",
};

/** 後端會把它換成決定檔的真實路徑。 */
export const DECISION_PATH_PLACEHOLDER = "{{DECISION_PATH}}";

/** 驗證並整理決定；不合格式回 null。`parseDecisionJson` 只是它的 JSON 入口。 */
function toDecision(raw: unknown): MethodDecision | null {
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

/** 決定檔可能寫到一半或還沒有：任何讀不懂的內容都回 null（當作還在討論）。 */
export function parseDecisionJson(text: string): MethodDecision | null {
  if (!text.trim()) return null;
  try {
    return toDecision(JSON.parse(text));
  } catch {
    return null;
  }
}

/** 開給 Claude Code session 的第一則提示：角色、判斷標準、決定檔格式。 */
export function buildDiscussionPrompt(opening: string): string {
  return [
    "你在協助使用者決定 AITerm「Agent 協作」這批工作該怎麼做，先討論、不要動手改任何程式。",
    "你可以也應該直接讀程式庫與參考資料夾（先讀它們的 CLAUDE.md／README，再看實際檔案）、跑唯讀指令做統計，再提建議；不要說你讀不到。",
    "",
    "可選的做法：",
    ...APPROACHES.map((a) => `- ${a}：${APPROACH_LABEL[a]}`),
    "",
    "批次只在四項全符合時才建議：有明確清單且通常 20 項以上；每項用同一份 template；驗證方式相同且可機械判斷；項目之間沒有依賴。",
    "能被產生器涵蓋的部分，優先建議先做產生器；批次只處理產生器處理不了的例外項目。",
    "不確定就先看資料、再問使用者，一次問一到兩個問題。",
    "",
    `當你有把握提案時，用 Write 工具把下面格式的 JSON 寫到 ${DECISION_PATH_PLACEHOLDER}（整份覆寫），然後用使用者的語言簡短說明，並請使用者回到 AITerm 的 Agent 協作畫面按「確認」：`,
    '{"approach":"single|batch|generator","reason":"為什麼，要引用你看到的實際資料","trial":"試跑計畫與驗證方式，沒有就留空字串","requirement":"整理後的完整需求"}',
    "使用者想調整時繼續討論，之後覆寫同一個檔案；以最後一次寫入為準。",
    "",
    `使用者的開場說明：\n${opening.trim()}`,
  ].join("\n");
}

/** 寫進需求卡與專案簡介的文字；讓後面的角色看得到決定與理由。 */
export function formatDecision(d: MethodDecision): string {
  const lines = [`做法決定：${APPROACH_LABEL[d.approach]}`];
  if (d.reason) lines.push(`理由：${d.reason}`);
  if (d.trial) lines.push(`試跑計畫：${d.trial}`);
  return lines.join("\n");
}
