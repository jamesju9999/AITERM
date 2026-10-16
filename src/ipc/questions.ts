import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ── Types（鏡射 Rust commands/questions.rs 的 QuestionView）─────────────────

/** Agent 用 ask_human 提出、還沒回答的問題（收件匣的一列）。 */
export interface QuestionView {
  project_id: string;
  project_name: string;
  task_title: string;
  agent_name: string | null;
  id: number;
  task_id: string;
  agent_id: string | null;
  question: string;
  /** 選項的 JSON 陣列字串。 */
  options: string;
  recommended: string | null;
  context: string | null;
  status: "open" | "answered" | "cancelled";
  created_at: number;
}

export function optionList(q: Pick<QuestionView, "options">): string[] {
  try {
    const v: unknown = JSON.parse(q.options);
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}

// ── Commands ────────────────────────────────────────────────────────────────

/** 所有已開啟專案裡還沒回答的問題（新的在前）。 */
export const listQuestions = (): Promise<QuestionView[]> => invoke("questions_list");

/** 回答一題：答案會打進提問的 Agent 的分頁，它接著做。 */
export const answerQuestion = (projectId: string, id: number, answer: string): Promise<void> =>
  invoke("questions_answer", { projectId, id, answer });

/** 有新問題、回答、或卡片結束讓問題作廢時。 */
export const onQuestionsUpdated = (cb: () => void): Promise<UnlistenFn> => listen("questions-updated", () => cb());
