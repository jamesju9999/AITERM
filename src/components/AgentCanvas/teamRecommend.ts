import type { Reference } from "../../ipc/agents";
import type { Translations } from "../../lib/i18n";
import { stripFence } from "../TaskBoard/refinePrompts";
import { assembleTeam, builtinTemplates, type TeamCoder, type TeamTemplate } from "./teamTemplates";

/**
 * 「依專案簡介推薦」團隊：AI 只能從這份角色目錄挑（提示詞是調整過的，不讓 AI 自己寫），
 * 可以給角色取貼近專案的名稱；結果一律再用固定規則檢查、修正並註明。
 */

interface CoderRole {
  key: string;
  /** 給 AI 看的一句用途。 */
  use: string;
  role: (t: Translations) => string;
  prefix: (t: Translations) => string;
  spec?: boolean;
}

const CODER_ROLES: CoderRole[] = [
  { key: "analyst", use: "需求分析師（產出規格）：釐清需求（會問使用者）、技術盤點、寫規格並拆成多項工作；團隊最前面", role: (t) => t.ac_role_analyst, prefix: (t) => t.ac_role_analyst_desc, spec: true },
  { key: "spec", use: "規格撰寫師（產出規格）：依卡片需求寫規格文件再拆工作；需求已經清楚、只是工作量大時用", role: (t) => t.ac_role_spec, prefix: (t) => t.ac_role_spec_desc, spec: true },
  { key: "coder", use: "程式碼工程師：一般實作", role: (t) => t.ls_preset_coder, prefix: (t) => t.ls_preset_coder_desc },
  { key: "frontend", use: "前端工程師：畫面、互動、樣式", role: (t) => t.ac_role_frontend, prefix: (t) => t.ac_role_frontend_desc },
  { key: "tester", use: "測試工程師：補測試、重現問題", role: (t) => t.ls_preset_tester, prefix: (t) => t.ls_preset_tester_desc },
  { key: "bugfix", use: "除錯工程師：找原因並修正 bug", role: (t) => t.ac_role_bugfix, prefix: (t) => t.ac_role_bugfix_desc },
  { key: "refactorer", use: "重構工程師：不改行為整理程式", role: (t) => t.ls_preset_refactorer, prefix: (t) => t.ls_preset_refactorer_desc },
  { key: "devops", use: "DevOps 工程師：建置、部署、CI、環境", role: (t) => t.ls_preset_devops, prefix: (t) => t.ls_preset_devops_desc },
  { key: "docs", use: "文件撰寫員：寫使用說明、API 文件", role: (t) => t.ls_preset_docs, prefix: (t) => t.ls_preset_docs_desc },
  { key: "architect", use: "架構師：架構設計與技術決策", role: (t) => t.ls_preset_architect, prefix: (t) => t.ls_preset_architect_desc },
  { key: "migration", use: "C/S 移植工程師：把舊的 Client-Server 模組移植成網頁系統", role: (t) => t.ac_role_migration, prefix: (t) => t.ac_role_migration_desc },
  { key: "inventory_researcher", use: "研究工程師（盤點）：只讀參考資料夾裡的舊系統、只寫盤點文件，結論附來源路徑", role: (t) => t.ls_preset_researcher, prefix: (t) => t.ac_tpl_inventory_researcher },
  { key: "inventory_synthesizer", use: "彙整工程師（盤點）：把各模組的盤點文件整合成總覽與風險清單", role: (t) => t.ls_preset_docs, prefix: (t) => t.ac_tpl_inventory_synthesizer },
];

const REVIEWER_ROLES: { key: string; use: string; role: (t: Translations) => string; prefix: (t: Translations) => string }[] = [
  { key: "general", use: "一般審核者：程式碼＋是否符合規格", role: (t) => t.ac_role_general_reviewer, prefix: (t) => t.ac_role_general_reviewer_desc },
  { key: "combined", use: "綜合審核者：一般＋資安（金流、個資、權限、對外服務）", role: (t) => t.ac_role_combined_reviewer, prefix: (t) => t.ac_role_combined_reviewer_desc },
  { key: "security", use: "安全審計員：只看資安", role: (t) => t.ls_preset_security, prefix: (t) => t.ac_role_security_reviewer_desc },
  { key: "docs", use: "文件審核者：產出是文件（例如盤點）時用，檢查來源與涵蓋項目", role: (t) => t.ac_tpl_inventory_reviewer_role, prefix: (t) => t.ac_tpl_inventory_reviewer },
];

/** 工程師最多幾位（同時派工太多，協調與合併的成本比省下的時間高）。 */
export const MAX_RECOMMENDED_CODERS = 6;
/** 提示裡要求的角色名稱長度；超過 MAX_NAME 才截斷（留一點餘裕，不切在詞中間）。 */
const NAME_HINT = 12;
const MAX_NAME = 24;

export function buildRecommendSystem(t: Translations): string {
  const tpls = builtinTemplates(t)
    .map((x) => `- ${x.id}（${x.name}）：${x.description}`)
    .join("\n");
  return [
    "你要依使用者的專案簡介，為 AITerm 的「Agent 協作」推薦一組 AI 團隊。",
    "流程固定：工程師（Coder）各自在自己的分支做模組 → 審核者審查 → 協調者合併 → 驗收者實際操作驗收。",
    "有「產出規格」的工程師時，他先寫規格，協調者照規格把工作拆給其他工程師。",
    "",
    "可以參考的內建範本：",
    tpls,
    "",
    "工程師角色（role 只能用這些 key）：",
    ...CODER_ROLES.map((r) => `- ${r.key}：${r.use}`),
    "",
    "審核者角色（只能用這些 key）：",
    ...REVIEWER_ROLES.map((r) => `- ${r.key}：${r.use}`),
    "",
    "規則：",
    `- 工程師總數 1–${MAX_RECOMMENDED_CODERS} 位；產出規格的（analyst／spec）最多一位，有的話一定要有協調者。`,
    "- 有驗收者就一定要有協調者。產出只有文件、不會執行程式時不需要驗收者。",
    "- 需求還不清楚、或要先摸清楚舊系統時，前面放 analyst。",
    `- 可以用 name 給角色取更貼近專案的名稱（例如「EGL 研究工程師」，${NAME_HINT} 字以內），不需要就省略；角色的工作內容仍由 key 決定。`,
    "- 每一項附一句 reason，說明為什麼需要（根據簡介裡的哪一點）。",
    "- reason、summary、name 用專案簡介的語言書寫。",
    "",
    "只輸出一個 JSON 物件，不要其他文字：",
    '{"base":"範本 id","summary":"一句話說明這組團隊","coders":[{"role":"key","name":"可省略","count":1,"reason":"…"}],' +
      '"reviewer":{"role":"key","reason":"…"},"orchestrator":{"include":true,"reason":"…"},"verifier":{"include":false,"reason":"…"}}',
  ].join("\n");
}

export function buildRecommendMessage(brief: string, refs: Reference[]): string {
  const parts = [`專案簡介：\n${brief.trim()}`];
  parts.push(
    refs.length
      ? `參考資料夾（整個團隊唯讀，例如舊系統原始碼）：\n${refs.map((r) => `- ${r.path}${r.note ? `：${r.note}` : ""}`).join("\n")}`
      : "參考資料夾：沒有",
  );
  return parts.join("\n\n");
}

/** 推薦結果：可以直接套用的範本，加上每個角色的理由與系統修正過的地方。 */
export interface Recommendation {
  template: TeamTemplate;
  summary: string;
  /** 角色名稱（跟範本卡片上顯示的一樣）→ 理由。 */
  reasons: { label: string; reason: string }[];
  /** 不合規則、系統自動修正的地方。 */
  notes: string[];
}

const str = (v: unknown): string => (typeof v === "string" ? v.trim() : "");

/** 從回覆裡取出 JSON 物件（容忍程式碼區塊與前後多餘的字）。 */
function extractJson(reply: string): Record<string, unknown> | null {
  const text = stripFence(reply.trim()).trim();
  const start = text.indexOf("{");
  const end = text.lastIndexOf("}");
  if (start < 0 || end <= start) return null;
  try {
    const v: unknown = JSON.parse(text.slice(start, end + 1));
    return v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

/** 解析並檢查 AI 的推薦；完全用不了（不是 JSON、沒有任何合法的工程師）回 null。 */
export function parseRecommendation(reply: string, t: Translations): Recommendation | null {
  const raw = extractJson(reply);
  if (!raw) return null;
  const notes: string[] = [];
  const reasons: { label: string; reason: string }[] = [];
  const coders: TeamCoder[] = [];
  let specKept = false;

  for (const item of Array.isArray(raw.coders) ? raw.coders : []) {
    const o = (item ?? {}) as Record<string, unknown>;
    const key = str(o.role);
    const def = CODER_ROLES.find((r) => r.key === key);
    if (!def) {
      notes.push(t.ac_rec_note_unknown_role(key || "?"));
      continue;
    }
    if (def.spec && specKept) {
      notes.push(t.ac_rec_note_one_spec);
      continue;
    }
    const label = str(o.name).slice(0, MAX_NAME) || def.role(t);
    let count = Math.floor(Number(o.count ?? 1));
    if (!Number.isFinite(count) || count < 1) count = 1;
    if (def.spec) count = 1;
    for (let i = 0; i < count; i++) {
      if (coders.length >= MAX_RECOMMENDED_CODERS) break;
      coders.push({ role: label, prefix: def.prefix(t), produces_spec: !!def.spec });
    }
    if (def.spec) specKept = true;
    const reason = str(o.reason);
    if (reason && !reasons.some((r) => r.label === label)) reasons.push({ label, reason });
  }
  const asked = (Array.isArray(raw.coders) ? raw.coders : []).reduce<number>((n, item) => {
    const c = Math.floor(Number(((item ?? {}) as Record<string, unknown>).count ?? 1));
    return n + (Number.isFinite(c) && c > 0 ? c : 1);
  }, 0);
  if (asked > MAX_RECOMMENDED_CODERS && coders.length === MAX_RECOMMENDED_CODERS) notes.push(t.ac_rec_note_max_coders(MAX_RECOMMENDED_CODERS));
  if (coders.length === 0) return null;

  const rv = (raw.reviewer ?? {}) as Record<string, unknown>;
  let rdef = REVIEWER_ROLES.find((r) => r.key === str(rv.role));
  if (!rdef) {
    if (str(rv.role)) notes.push(t.ac_rec_note_unknown_role(str(rv.role)));
    rdef = REVIEWER_ROLES[0];
  }
  const reviewer = { role: rdef.role(t), prefix: rdef.prefix(t) };
  if (str(rv.reason)) reasons.push({ label: reviewer.role, reason: str(rv.reason) });

  const orch = (raw.orchestrator ?? {}) as Record<string, unknown>;
  const ver = (raw.verifier ?? {}) as Record<string, unknown>;
  const withVerifier = ver.include === true;
  let withOrch = orch.include === true;
  if (!withOrch && withVerifier) {
    withOrch = true;
    notes.push(t.ac_rec_note_orch_for_verifier);
  }
  if (!withOrch && specKept) {
    withOrch = true;
    notes.push(t.ac_rec_note_orch_for_spec);
  }
  if (withOrch && str(orch.reason)) reasons.push({ label: t.ac_role_integrator, reason: str(orch.reason) });
  if (withVerifier && str(ver.reason)) reasons.push({ label: t.ac_role_verifier, reason: str(ver.reason) });

  return {
    template: { name: t.ac_rec_name, agents: assembleTeam(t, coders, reviewer, withOrch, withVerifier) },
    summary: str(raw.summary),
    reasons,
    notes,
  };
}
