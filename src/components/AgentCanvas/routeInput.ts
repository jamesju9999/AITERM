import type { AgentRow } from "../../ipc/agents";
import type { TaskWithAttachments } from "../../ipc/tasks";
import type { Translations } from "../../lib/i18n";
import { stripFence } from "../TaskBoard/refinePrompts";
import { bugCard, pickAssignee } from "./bugReport";
import { draftDoneCriteria, pickFirstStation, type StartDeps } from "./startFromRequirement";
import { withDoneCriteria } from "./doneCriteria";

/**
 * 常駐輸入框：什麼都往裡面丟，由 AI 判斷該怎麼處理——
 * 新需求走完整流程、小修改走快速路徑、bug 直接派工程師、問題直接回答（不建卡）。
 * 分類只是「建議」：畫面會先顯示打算怎麼做，使用者可以改類型再執行。
 */

export type InputKind = "requirement" | "fast" | "bug" | "question";
export const INPUT_KINDS: InputKind[] = ["requirement", "fast", "bug", "question"];

export interface RouteDecision {
  kind: InputKind;
  reason: string;
}

export function buildRouteSystem(): string {
  return [
    "你是 AITerm「Agent 協作」的分派員。使用者在輸入框打了一段話，你要判斷該怎麼處理，只能選一種：",
    "- requirement：新功能、需求變更、要做一個東西。要先寫規格再分工，走完整流程。",
    "- fast：明確、範圍很小的修改，改動大概在 1–3 個檔案內（改文字、調樣式、改一個數值、加一個小欄位）。不需要規格與審查。",
    "- bug：回報錯誤、壞掉、跟預期不一樣，要找原因並修正。",
    "- question：只是想了解現況或問問題（進度、某功能怎麼運作、為什麼這樣），不需要改程式。",
    "",
    "判斷原則：拿不準 fast 還是 requirement 時選 requirement（小修改誤判成大需求只是慢一點，反過來會繞過審查）。",
    "reason 用一句話說明為什麼，用使用者的語言書寫。",
    "",
    '只輸出一個 JSON 物件，不要其他文字：{"kind":"requirement|fast|bug|question","reason":"…"}',
  ].join("\n");
}

export function buildRouteMessage(text: string, brief: string): string {
  const parts = [`使用者輸入：\n${text.trim()}`];
  if (brief.trim()) parts.unshift(`專案簡介：\n${brief.trim()}`);
  return parts.join("\n\n");
}

/** 解析分類結果；不是預期格式回 null。 */
export function parseRoute(reply: string): RouteDecision | null {
  const text = stripFence(reply.trim()).trim();
  const start = text.indexOf("{");
  const end = text.lastIndexOf("}");
  if (start < 0 || end <= start) return null;
  try {
    const v = JSON.parse(text.slice(start, end + 1)) as { kind?: unknown; reason?: unknown };
    const kind = typeof v.kind === "string" ? v.kind.trim() : "";
    if (!INPUT_KINDS.includes(kind as InputKind)) return null;
    return { kind: kind as InputKind, reason: typeof v.reason === "string" ? v.reason.trim() : "" };
  } catch {
    return null;
  }
}

/** 分類。沒有 AI、AI 失敗或回得不對，一律退回「新需求」（最保守：走完整流程）。 */
export async function routeInput(
  d: Pick<StartDeps, "complete" | "getBrief">,
  projectId: string,
  text: string,
  providerId: string | undefined,
): Promise<{ decision: RouteDecision; usedDefault: boolean }> {
  const fallback = { decision: { kind: "requirement" as InputKind, reason: "" }, usedDefault: true };
  if (!providerId) return fallback;
  try {
    const brief = ((await d.getBrief(projectId).catch(() => null)) ?? "").trim();
    const reply = await d.complete([{ role: "user", content: buildRouteMessage(text, brief) }], buildRouteSystem(), "input-route", providerId);
    const decision = parseRoute(reply);
    return decision ? { decision, usedDefault: false } : fallback;
  } catch {
    return fallback;
  }
}

export interface DispatchEnv {
  projectId: string;
  repoDir: string;
  agents: AgentRow[];
  tasks: TaskWithAttachments[];
  t: Translations;
  providerId?: string;
}

export interface DispatchResult {
  kind: InputKind;
  /** 建的卡（問題是一張唯讀的問答卡，答案在卡完成後才有）。 */
  taskId?: string;
  /** 交給誰。 */
  agentName?: string;
  /** 沒有可走快速路徑的工程師，退回完整流程。 */
  fellBack?: boolean;
}

export class NoStationError extends Error {}

/** 卡片附件（截圖）。由外面注入，測試不必 mock Tauri。 */
export type AddAttachment = (projectId: string, taskId: string, name: string, bytes: Uint8Array) => Promise<unknown>;
type DispatchDeps = Pick<StartDeps, "createTask" | "assignAgent" | "complete" | "getBrief"> & {
  addAttachment?: AddAttachment;
  /** 問問題：交給 Agent 讀程式庫回答。 */
  askAgent?: (projectId: string, question: string) => Promise<{ task_id: string; agent_name: string }>;
};

async function attachImages(d: DispatchDeps, projectId: string, taskId: string, images: File[]) {
  for (const file of images) {
    const bytes = new Uint8Array(await (file.arrayBuffer ? file.arrayBuffer() : new Response(file).arrayBuffer()));
    await d.addAttachment?.(projectId, taskId, file.name, bytes);
  }
}

const newCard = (d: Pick<StartDeps, "createTask">, env: DispatchEnv, title: string, body: string) =>
  d.createTask(env.projectId, {
    title,
    body,
    project_dir: env.repoDir,
    parallel_ok: true,
    interactive: false,
    use_bridge: false,
    bridge_tiers: null,
    label: null,
    isolate_worktree: null,
  });

/** 照分類的結果執行。失敗就丟例外（建好的卡不會重複建：建卡之後才派工，派工失敗時卡片留在計畫中）。 */
export async function dispatchInput(
  kind: InputKind,
  text: string,
  env: DispatchEnv,
  d: DispatchDeps,
  /** 使用者附的圖（可以沒有）：問問題以外都附到卡片上。 */
  images: File[] = [],
): Promise<DispatchResult> {
  const { projectId, agents, tasks, t } = env;
  const nameOf = (id: string) => agents.find((a) => a.id === id)?.name ?? id;

  if (kind === "question") {
    // 交給 Agent 讀程式庫回答，不是用簡介猜：簡介裡沒有的細節（用了哪個 API、某功能怎麼運作）只有看程式碼才答得出來。
    if (!d.askAgent) throw new Error("askAgent missing");
    const r = await d.askAgent(projectId, text.trim());
    return { kind, taskId: r.task_id, agentName: r.agent_name };
  }

  if (kind === "fast" || kind === "bug") {
    const assignee = pickAssignee(agents, tasks, null);
    if (assignee) {
      const shots = images.length > 0;
      const instructions = kind === "bug" ? (shots ? t.ac_bug_instructions : t.ac_bug_instructions_noshot) : shots ? t.ac_cmd_image_note : "";
      const { title, body } = bugCard(text, instructions, null);
      const taskId = await newCard(d, env, title, body);
      await attachImages(d, projectId, taskId, images);
      await d.assignAgent(projectId, taskId, assignee.id, kind === "fast");
      return { kind, taskId, agentName: nameOf(assignee.id) };
    }
    // 只有規格站（或沒有工程師）：小修改沒地方走快速路徑，退回完整流程。
    const result = await dispatchInput("requirement", text, env, d, images);
    return { ...result, fellBack: true };
  }

  const station = pickFirstStation(agents);
  if (!station) throw new NoStationError(t.ac_cmd_no_coder);
  const { title, body } = bugCard(text, images.length > 0 ? t.ac_cmd_image_note : "", null);
  const brief = ((await d.getBrief(projectId).catch(() => null)) ?? "").trim();
  const criteria = await draftDoneCriteria(d, text, brief, env.providerId);
  const taskId = await newCard(d, env, title, withDoneCriteria(body, criteria));
  await attachImages(d, projectId, taskId, images);
  await d.assignAgent(projectId, taskId, station);
  return { kind: "requirement", taskId, agentName: nameOf(station) };
}
