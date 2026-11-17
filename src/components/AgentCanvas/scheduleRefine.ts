import { stripFence } from "../TaskBoard/refinePrompts";

/**
 * 定時器固定指令的「AI 潤飾」。這段文字每次到點都會原封不動交給同一位角色（工程師或驗收者），
 * 沒有人可以追問，所以要寫成一次就能照著做、做完怎麼算完成都講清楚的指令。
 */
export const SCHEDULE_REFINE_SYSTEM = [
  "你要把使用者草擬的內容，改寫成一份「定時任務的固定指令」。",
  "這份指令會在固定時間自動交給一位 AI 角色（工程師或驗收者）執行，沒有人在旁邊可以追問，所以必須一次說清楚。",
  "",
  "改寫原則：",
  "- 保留使用者的原意與所有具體資訊（名稱、路徑、指令、數字、專有名詞），不要發明使用者沒說的需求。",
  "- 先用一句話講這輪要達成什麼，再用簡短條列寫要做的步驟與範圍。",
  "- 要有明確的完成標準（做到什麼算完成），使用者沒說就只寫從原文看得出來的。",
  "- 每次都會重新執行，所以不要寫「今天」「這次」之類只對單次成立的字眼。",
  "- 精簡：能用一兩句講完的就不要拉長。",
  "- 用使用者草稿的語言書寫。只輸出指令本身：不要包成程式碼區塊，不要開場白或結語。",
].join("\n");

export function buildScheduleRefineMessage(draft: string): string {
  return `使用者草擬的定時任務指令：\n${draft}`;
}

/** 去掉模型可能包的程式碼區塊與頭尾空白；整份是空的回傳 null。 */
export function parseScheduleReply(reply: string): string | null {
  const text = stripFence(reply.trim()).trim();
  return text || null;
}
