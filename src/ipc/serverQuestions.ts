import { invoke } from "@tauri-apps/api/core";

/** 共用專案裡 Agent 提的問題（server 的 `ask_human`）。`options` 是 JSON 陣列字串。 */
export interface ServerQuestion {
  id: number;
  project_id: string;
  card_id: string;
  agent_id: string | null;
  question: string;
  options: string;
  recommended: string | null;
  context: string | null;
  status: "open" | "answered" | "cancelled";
  answer: string | null;
  answered_by: string | null;
  answered_via: string | null;
  created_at: number;
  answered_at: number | null;
}

export interface ServerQuestions {
  open: ServerQuestion[];
  closed: ServerQuestion[];
}

export const listServerQuestions = (projectId: string): Promise<ServerQuestions> => invoke("server_questions", { projectId });
/** 回答一個問題（任何成員）。 */
export const answerServerQuestion = (id: number, answer: string): Promise<void> => invoke("server_question_answer", { id, answer });
/** 取消一個問題（管理員限定）。 */
export const cancelServerQuestion = (id: number): Promise<void> => invoke("server_question_cancel", { id });
/** 通知信箱（管理員限定）。 */
export const getAskNotifyEmail = (): Promise<string> => invoke("server_ask_settings");
export const setAskNotifyEmail = (notifyEmail: string): Promise<void> => invoke("server_ask_settings_set", { notifyEmail });

export const questionOptions = (q: ServerQuestion): string[] => {
  try {
    const v: unknown = JSON.parse(q.options);
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
};
