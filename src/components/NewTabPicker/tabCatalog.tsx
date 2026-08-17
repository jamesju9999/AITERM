import type { Translations } from "../../lib/i18n";
import type { TabType } from "../TabBar";
import {
  TerminalIcon, DatabaseIcon, PaintbrushIcon, LinkIcon, BranchIcon,
  FileTextIcon, BookOpenIcon, RefreshIcon, CodeIcon, LibraryIcon, MailIcon, RobotIcon,
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
  hidden?: boolean;
  requiresBridge?: boolean;
  disabledHintKey?: StringTranslationKey;
}

const TAB_CATALOG_META: readonly TabCatalogMeta[] = [
  { id: "terminal",       type: "terminal",       icon: <TerminalIcon size={18} />,  labelKey: "terminal_tab",       descKey: "new_terminal_desc" },
  { id: "database",       type: "database",       icon: <DatabaseIcon size={18} />,  labelKey: "database_tab",       descKey: "new_database_desc" },
  { id: "design",         type: "design",         icon: <PaintbrushIcon size={18} />, labelKey: "design_tab",        descKey: "new_design_desc" },
  { id: "cross-db",       type: "cross-db",       icon: <LinkIcon size={18} />,      labelKey: "cross_db_tab",       descKey: "new_cross_db_desc" },
  { id: "vcs",            type: "vcs",            icon: <BranchIcon size={18} />,    labelKey: "vcs_tab",            descKey: "new_vcs_desc" },
  { id: "doc-converter",  type: "doc-converter",  icon: <FileTextIcon size={18} />,  labelKey: "doc_converter_tab",  descKey: "new_doc_converter_desc" },
  // api-docs 的入口在 commit 3547799 被刻意收起來（後端 ApiDocsView / api_docs /
  // ApiDocFetcher 全部保留），跟 mail 是同一種狀況：完整但不對使用者開放。
  { id: "api-docs",       type: "api-docs",       icon: <BookOpenIcon size={18} />,  labelKey: "api_docs_tab",       descKey: "new_api_docs_desc", hidden: true },
  { id: "loop-studio",    type: "loop-studio",    icon: <RefreshIcon size={18} />,   labelKey: "loop_studio_tab",    descKey: "new_loop_studio_desc" },
  { id: "code-assistant", type: "code-assistant", icon: <CodeIcon size={18} />,      labelKey: "code_assistant_tab", descKey: "new_code_assistant_desc" },
  { id: "knowledge-base", type: "knowledge-base", icon: <LibraryIcon size={18} />,   labelKey: "knowledge_base_tab", descKey: "new_knowledge_base_desc" },
  // The Mail entry is hidden while the feature is not being shipped to
  // users. The "mail" tab type, MailView, and its whole backend remain wired
  // up — restoring it means putting this line back, plus the matching button
  // in SettingsView, and re-adding the MailIcon import above. See the notes in
  // docs/superpowers/specs/2026-08-04-ai-mail-assistant-design.md.
  { id: "mail",           type: "mail",           icon: <MailIcon size={18} />,      labelKey: "mail_tab",           descKey: "new_mail_desc", hidden: true },
  // Claude Code 不是獨立的 TabType，而是「終端機分頁 + claudeBridge 選項」，
  // 且需要橋接 server 正在跑才能用（見 requiresBridge）。放在陣列最後，
  // 跟它在新增分頁選單裡的位置一致。
  { id: "claude-code",    type: "terminal",       opts: { claudeBridge: true }, icon: <RobotIcon size={18} />, labelKey: "bridge_new_tab", descKey: "bridge_new_tab_desc", requiresBridge: true, disabledHintKey: "bridge_new_tab_disabled_hint" },
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
    hidden: m.hidden,
    requiresBridge: m.requiresBridge,
    disabledHint: m.disabledHintKey ? t[m.disabledHintKey] : undefined,
  }));
}

export function visibleTabCatalog(t: Translations): TabCatalogEntry[] {
  return getTabCatalog(t).filter((e) => !e.hidden);
}
