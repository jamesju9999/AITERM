import type { Translations } from "../../lib/i18n";
import type { Tab, TabType } from "../TabBar";
import {
  TerminalIcon, DatabaseIcon, PaintbrushIcon, LinkIcon, BranchIcon,
  FileTextIcon, BookOpenIcon, RefreshIcon, CodeIcon, LibraryIcon, MailIcon, RobotIcon, EyeIcon,
} from "../Icons";

/** 建立分頁時要一併傳給 onSelect/onOpenTab 的選項。NewTabPicker、HomeView、
 *  TerminalApp 的 handler 型別都指到這一個型別，多一個欄位時漏改的消費端
 *  會直接編譯錯誤，而不是靜默把選項丟掉。 */
export interface TabOpenOpts {
  claudeBridge?: boolean;
  /** 終端機分頁的起始目錄。首頁的「最近的專案目錄」用它開在指定位置。
   *  這跟 Tab.cwd（分頁目前實際所在的目錄）是兩件事，不要混用。 */
  initialCwd?: string;
  /** 建立分頁時就掛上 agent 任務。首頁的自然語言輸入框用它。 */
  initialMission?: { goal: string; maxSteps: number };
}

export interface TabCatalogEntry {
  /** React key 與識別用。Claude Code 與一般終端機的 type 都是 "terminal"，
   *  type 不足以區分，所以另外給 id。 */
  id: string;
  type: TabType;
  /** 建立分頁時要一併傳給 onSelect 的選項。 */
  opts?: TabOpenOpts;
  icon: React.ReactNode;
  label: string;
  desc: string;
  /** 分頁類型專屬色（hex），套用在首頁「開始工作」卡片的圖示與左邊框。
   *  必填而非 optional：新增 TabType 時，下面的 `_exhaustive` 型別窮盡性
   *  檢查已經會擋掉漏列的 entry，但那只保證「有沒有這一筆」，保證不了
   *  「這一筆有沒有顏色」。設成必填，漏寫顏色會直接編譯失敗，而不是
   *  默默生出一張沒有顏色的卡片。 */
  color: string;
  /** 後端完整但尚未對使用者開放。隱藏的理由集中記在這裡，不要散在各個呼叫端。 */
  hidden?: boolean;
  /** 需要橋接 server 正在執行才能使用。呼叫端自行決定沒在跑時要停用還是隱藏。 */
  requiresBridge?: boolean;
  /** requiresBridge 為 true 且橋接沒在跑時，取代 desc 顯示的提示文字。 */
  disabledHint?: string;
}

/** Translations 裡值型別是 string 的鍵——排除函式型與陣列型的翻譯項
 *  （例如 `(n) => string`），因為 label/desc 只能是字串。 */
type StringTranslationKey = { [K in keyof Translations]: Translations[K] extends string ? K : never }[keyof Translations];

/** getTabCatalog 的原始資料。label/desc 是 i18n 字串鍵，不是字串本身——
 *  這一份不需要 Translations 就能存在，只有 getTabCatalog(t) 組出完整
 *  entry 時才需要把鍵換成當下語系的字串。
 *  icon 不吃 t，所以直接放在這裡建一次，不必每次呼叫 getTabCatalog 都重建。 */
interface TabCatalogMeta {
  id: string;
  type: TabType;
  opts?: TabOpenOpts;
  icon: React.ReactNode;
  labelKey: StringTranslationKey;
  descKey: StringTranslationKey;
  /** 見 TabCatalogEntry.color 的註解。 */
  color: string;
  hidden?: boolean;
  requiresBridge?: boolean;
  disabledHintKey?: StringTranslationKey;
}

const TAB_CATALOG_META: readonly TabCatalogMeta[] = [
  { id: "terminal",       type: "terminal",       icon: <TerminalIcon size={18} />,  labelKey: "terminal_tab",       descKey: "new_terminal_desc",       color: "#4ade80" },
  { id: "database",       type: "database",       icon: <DatabaseIcon size={18} />,  labelKey: "database_tab",       descKey: "new_database_desc",       color: "#60a5fa" },
  { id: "design",         type: "design",         icon: <PaintbrushIcon size={18} />, labelKey: "design_tab",        descKey: "new_design_desc",         color: "#a855f7" },
  { id: "cross-db",       type: "cross-db",       icon: <LinkIcon size={18} />,      labelKey: "cross_db_tab",       descKey: "new_cross_db_desc",       color: "#22d3ee" },
  { id: "vcs",            type: "vcs",            icon: <BranchIcon size={18} />,    labelKey: "vcs_tab",            descKey: "new_vcs_desc",            color: "#fb923c" },
  { id: "doc-converter",  type: "doc-converter",  icon: <FileTextIcon size={18} />,  labelKey: "doc_converter_tab",  descKey: "new_doc_converter_desc",  color: "#94a3b8" },
  // api-docs 的入口在 commit 3547799 被刻意收起來（後端 ApiDocsView / api_docs /
  // ApiDocFetcher 全部保留），跟 mail 是同一種狀況：完整但不對使用者開放。
  { id: "api-docs",       type: "api-docs",       icon: <BookOpenIcon size={18} />,  labelKey: "api_docs_tab",       descKey: "new_api_docs_desc",       color: "#38bdf8", hidden: true },
  { id: "loop-studio",    type: "loop-studio",    icon: <RefreshIcon size={18} />,   labelKey: "loop_studio_tab",    descKey: "new_loop_studio_desc",    color: "#f472b6" },
  { id: "code-assistant", type: "code-assistant", icon: <CodeIcon size={18} />,      labelKey: "code_assistant_tab", descKey: "new_code_assistant_desc", color: "#facc15" },
  { id: "knowledge-base", type: "knowledge-base", icon: <LibraryIcon size={18} />,   labelKey: "knowledge_base_tab", descKey: "new_knowledge_base_desc", color: "#818cf8" },
  // The Mail entry is hidden while the feature is not being shipped to
  // users. The "mail" tab type, MailView, and its whole backend remain wired
  // up — restoring it means putting this line back, plus the matching button
  // in SettingsView, and re-adding the MailIcon import above. See the notes in
  // docs/superpowers/specs/2026-08-04-ai-mail-assistant-design.md.
  { id: "mail",           type: "mail",           icon: <MailIcon size={18} />,      labelKey: "mail_tab",           descKey: "new_mail_desc",           color: "#f87171", hidden: true },
  // 2B-2a 只做遠端終端機的畫面，還沒有「連線到同事的終端機」那個入口
  // （分享按鈕、同意視窗、連線對話框都是 2B-2b）——跟 mail、api-docs 同一種
  // 狀況：分頁型別跟畫面都完整，只是還不對使用者開放，用 hidden 標記。
  { id: "remote-terminal", type: "remote-terminal", icon: <EyeIcon size={18} />,      labelKey: "remote_terminal_tab", descKey: "new_remote_terminal_desc", color: "#2dd4bf", hidden: true },
  // Claude Code 不是獨立的 TabType，而是「終端機分頁 + claudeBridge 選項」，
  // 且需要橋接 server 正在跑才能用（見 requiresBridge）。放在陣列最後，
  // 跟它在新增分頁選單裡的位置一致。顏色刻意跟 vcs 的橙拉開，用 Anthropic 的
  // 品牌陶土色。
  { id: "claude-code",    type: "terminal",       opts: { claudeBridge: true }, icon: <RobotIcon size={18} />, labelKey: "bridge_new_tab", descKey: "bridge_new_tab_desc", color: "#d97757", requiresBridge: true, disabledHintKey: "bridge_new_tab_disabled_hint" },
];

/** 可路由的分頁類型：從 TAB_CATALOG_META 過濾掉 hidden 之後推導，不需要
 *  Translations。給 AI 路由（routeIntent.ts）判斷 AI 回應是否落在允許清單內
 *  用——AI 路由不能是 hidden 分頁（mail、api-docs）的後門，所以這裡要用
 *  visibleTabCatalog 同一份過濾邏輯，而不是另外維護一份型別清單。
 *  用 Set 去重，因為 claude-code 這一筆的 type 也是 "terminal"。 */
export const VISIBLE_TAB_TYPES: readonly TabType[] = Array.from(
  new Set(TAB_CATALOG_META.filter((m) => !m.hidden).map((m) => m.type)),
);

/** 分頁類型的唯一清單。NewTabPicker、首頁大圖入口、AI 路由提示詞都用這一份。 */
export function getTabCatalog(t: Translations): TabCatalogEntry[] {
  return TAB_CATALOG_META.map((m) => ({
    id: m.id,
    type: m.type,
    opts: m.opts,
    icon: m.icon,
    label: t[m.labelKey],
    desc: t[m.descKey],
    color: m.color,
    hidden: m.hidden,
    requiresBridge: m.requiresBridge,
    disabledHint: m.disabledHintKey ? t[m.disabledHintKey] : undefined,
  }));
}

export function visibleTabCatalog(t: Translations): TabCatalogEntry[] {
  return getTabCatalog(t).filter((e) => !e.hidden);
}

/** 分頁類型對應的專屬色。只吃 type 與 claudeBridge 兩個欄位，不吃整個 Tab——
 *  呼叫端（例如 ResumeSection）手上通常已經有一個 Tab，但這裡刻意收窄參數，
 *  之後 Tab 多長欄位不會牽動這個函式，測試也只需要造兩個值。
 *  "terminal" 這個 type 同時對應一般終端機與 Claude Code 兩筆 entry，光用
 *  type 去 find 只會拿到陣列裡第一筆（一般終端機），把 Claude Code 分頁誤判
 *  成綠色——所以要先用 claudeBridge 分流到 claude-code 那筆，其餘型別才用
 *  type 找。 */
export function colorForTab(type: TabType, claudeBridge?: Tab["claudeBridge"]): string {
  if (type === "terminal" && claudeBridge) {
    return TAB_CATALOG_META.find((m) => m.id === "claude-code")!.color;
  }
  return TAB_CATALOG_META.find((m) => m.type === type)!.color;
}
