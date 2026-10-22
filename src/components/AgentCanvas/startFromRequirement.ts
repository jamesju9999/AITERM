import type { AgentKind, NewAgentArgs, Reference } from "../../ipc/agents";
import type { Translations } from "../../lib/i18n";
import { bugCard } from "./bugReport";
import { buildDoneMessage, buildDoneSystem, parseDoneCriteria, withDoneCriteria } from "./doneCriteria";
import { buildRecommendMessage, buildRecommendSystem, parseRecommendation } from "./teamRecommend";
import { builtinTemplates, toCreate, type TeamTemplate } from "./teamTemplates";

/**
 * 需求先行：空桌布上直接打「你想做什麼」，依需求組好團隊並把需求派給第一站。
 * 推薦、套用都沿用範本對話框那一套（teamRecommend／toCreate），這裡只負責串起來。
 */

/** 外部呼叫都從這裡注入，測試不必 mock Tauri。 */
export interface StartDeps {
  getBrief: (projectId: string) => Promise<string | null>;
  setBrief: (projectId: string, brief: string) => Promise<void>;
  getReferences: (projectId: string) => Promise<Reference[]>;
  complete: (messages: { role: "user"; content: string }[], system: string, streamId: string, providerId?: string) => Promise<string>;
  createAgent: (projectId: string, args: NewAgentArgs) => Promise<string>;
  createTask: (
    projectId: string,
    args: {
      title: string;
      body: string;
      project_dir: string;
      parallel_ok: boolean;
      interactive: boolean;
      use_bridge: boolean;
      bridge_tiers: string | null;
      label: string | null;
      isolate_worktree: boolean | null;
    },
  ) => Promise<string>;
  assignAgent: (projectId: string, taskId: string, agentId: string, fast?: boolean) => Promise<void>;
}

export interface StartArgs {
  projectId: string;
  requirement: string;
  repoDir: string;
  t: Translations;
  /** 沒有供應商 = 不問 AI，直接用預設團隊。 */
  providerId?: string;
  /** AI 用不了時的團隊；省略 = 內建「標準」。 */
  fallback?: TeamTemplate;
  /**
   * 建好團隊後要不要立刻把需求派出去。預設 true（維持原本的行為）；
   * 需求框預設傳 false：使用者可能想先把資料庫、自訂技能掛到工程師身上，準備好再按「開始執行」。
   */
  autoStart?: boolean;
}

export interface StartResult {
  /** 給使用者看的一句話：為什麼是這支團隊。 */
  summary: string;
  reasons: { label: string; reason: string }[];
  /** AI 沒能推薦，用了預設團隊。 */
  usedDefault: boolean;
  /** 寫進卡片的「完成定義」；沒有 AI 或失敗時是空的。 */
  criteria: string[];
  /** 還沒派出去的需求卡（沒有自動開始時才有）：按「開始執行」就派給 `stationId`。 */
  pending?: { taskId: string; stationId: string };
}

/** 請 AI 把需求寫成完成定義。任何失敗都回空陣列：少了它只是少一道把關，不該擋下建卡。 */
export async function draftDoneCriteria(
  d: Pick<StartDeps, "complete">,
  requirement: string,
  brief: string,
  providerId: string | undefined,
): Promise<string[]> {
  if (!providerId) return [];
  try {
    const reply = await d.complete([{ role: "user", content: buildDoneMessage(requirement, brief) }], buildDoneSystem(), "done-criteria", providerId);
    return parseDoneCriteria(reply);
  } catch {
    return [];
  }
}

/** 失敗時告訴畫面：團隊是不是已經建好了（建好了就不能再讓人重送一次）。 */
export class StartError extends Error {
  teamCreated: boolean;
  constructor(message: string, teamCreated: boolean) {
    super(message);
    this.teamCreated = teamCreated;
  }
}

/** 需求交給誰：先寫規格的人（需求分析師／規格撰寫師）優先，否則第一位一般工程師。 */
export function pickFirstStation(agents: { id: string; kind: AgentKind; produces_spec: boolean }[]): string | null {
  const coders = agents.filter((a) => a.kind === "coder");
  return (coders.find((a) => a.produces_spec) ?? coders[0])?.id ?? null;
}

const msg = (e: unknown) => (e instanceof Error ? e.message : typeof e === "string" ? e : JSON.stringify(e));

export async function startFromRequirement(args: StartArgs, d: StartDeps): Promise<StartResult> {
  const { projectId, repoDir, t } = args;
  const requirement = args.requirement.trim();

  const brief = ((await d.getBrief(projectId).catch(() => null)) ?? "").trim();
  // 簡介是空的才寫入：簡介是專案的長期背景，之後的需求不該把它越寫越亂。
  if (!brief) await d.setBrief(projectId, requirement).catch(() => {});

  let tpl: TeamTemplate | null = null;
  let summary = "";
  let reasons: { label: string; reason: string }[] = [];
  if (args.providerId) {
    try {
      const refs = await d.getReferences(projectId).catch(() => []);
      const described = brief ? `${brief}\n\n本次需求：\n${requirement}` : requirement;
      const reply = await d.complete(
        [{ role: "user", content: buildRecommendMessage(described, refs) }],
        buildRecommendSystem(t),
        "team-recommend",
        args.providerId,
      );
      const rec = parseRecommendation(reply, t);
      if (rec) {
        tpl = rec.template;
        summary = rec.summary;
        reasons = rec.reasons;
      }
    } catch {
      // 退回預設團隊
    }
  }
  const usedDefault = !tpl;
  if (!tpl) {
    tpl = args.fallback ?? builtinTemplates(t).find((x) => x.id === "standard")!;
    summary = tpl.name;
  }

  const created: { id: string; kind: AgentKind; produces_spec: boolean }[] = [];
  const toMake = toCreate(tpl, [], repoDir, 40, 40);
  try {
    for (const a of toMake) {
      const id = await d.createAgent(projectId, a);
      created.push({ id, kind: a.kind, produces_spec: !!a.produces_spec });
    }
  } catch (e) {
    throw new StartError(msg(e), created.length > 0);
  }

  try {
    const station = pickFirstStation(created);
    if (!station) throw new Error("no coder");
    const { title, body } = bugCard(requirement, "", null);
    const criteria = await draftDoneCriteria(d, requirement, brief, args.providerId);
    const taskId = await d.createTask(projectId, {
      title,
      body: withDoneCriteria(body, criteria),
      project_dir: repoDir,
      parallel_ok: true,
      interactive: false,
      use_bridge: false,
      bridge_tiers: null,
      label: null,
      isolate_worktree: null,
    });
    if (args.autoStart === false) return { summary, reasons, usedDefault, criteria, pending: { taskId, stationId: station } };
    await d.assignAgent(projectId, taskId, station);
    return { summary, reasons, usedDefault, criteria };
  } catch (e) {
    throw new StartError(msg(e), true);
  }
}
