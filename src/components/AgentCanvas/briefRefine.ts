import { stripFence } from "../TaskBoard/refinePrompts";

/**
 * 專案簡介的「AI 潤飾」。這份簡介會原封不動放進每一張 Agent 卡的提示（見
 * src-tauri/src/tasks/context.rs），所以目標是讓所有 Agent 都需要知道的事一目了然，
 * 而且要短——每張卡都會帶著它。團隊分工是系統自動附的，簡介裡不要重複。
 */
export const BRIEF_REFINE_SYSTEM = [
  "你要把使用者草擬的專案說明，改寫成一份「專案簡介」。",
  "這份簡介會放在每一位 AI 工程師（Claude Code／Codex）開工前的提示裡，讓平行工作、各自只負責一個模組的 Agent 有共同的全貌。",
  "",
  "改寫原則：",
  "- 保留使用者的原意與所有具體資訊（名稱、技術、版本、路徑、數字、專有名詞），不要發明使用者沒說的需求或技術選型。",
  "- 依序整理成這些段落，使用者沒提到的段落整段省略：目標、使用者與情境、技術選型、架構與目錄慣例、共同約定（命名、程式風格、介面語言、測試）、不做的事。",
  "- 每段用一行「段落名：」開頭，內容用簡短條列。",
  "- 不要列出模組分工或誰負責什麼：系統會另外自動附上團隊分工。",
  "- 精簡：只寫所有 Agent 都需要知道的事，總長盡量在 800 字以內。",
  "- 用使用者草稿的語言書寫。只輸出簡介本身：不要包成程式碼區塊，不要開場白或結語。",
].join("\n");

export function buildBriefRefineMessage(draft: string): string {
  return `使用者草擬的專案說明：\n${draft}`;
}

/** 去掉模型可能包的程式碼區塊與頭尾空白；整份是空的回傳 null。 */
export function parseBriefReply(reply: string): string | null {
  const text = stripFence(reply.trim()).trim();
  return text || null;
}
