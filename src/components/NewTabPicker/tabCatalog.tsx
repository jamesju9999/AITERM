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

/** 分頁類型的唯一清單。NewTabPicker、首頁大圖入口、AI 路由提示詞都用這一份。 */
export function getTabCatalog(t: Translations): TabCatalogEntry[] {
  return [
    { id: "terminal",       type: "terminal",       icon: <TerminalIcon size={18} />,  label: t.terminal_tab,       desc: t.new_terminal_desc },
    { id: "database",       type: "database",       icon: <DatabaseIcon size={18} />,  label: t.database_tab,       desc: t.new_database_desc },
    { id: "design",         type: "design",         icon: <PaintbrushIcon size={18} />, label: t.design_tab,        desc: t.new_design_desc },
    { id: "cross-db",       type: "cross-db",       icon: <LinkIcon size={18} />,      label: t.cross_db_tab,       desc: t.new_cross_db_desc },
    { id: "vcs",            type: "vcs",            icon: <BranchIcon size={18} />,    label: t.vcs_tab,            desc: t.new_vcs_desc },
    { id: "doc-converter",  type: "doc-converter",  icon: <FileTextIcon size={18} />,  label: t.doc_converter_tab,  desc: t.new_doc_converter_desc },
    // api-docs 的入口在 commit 3547799 被刻意收起來（後端 ApiDocsView / api_docs /
    // ApiDocFetcher 全部保留），跟 mail 是同一種狀況：完整但不對使用者開放。
    { id: "api-docs",       type: "api-docs",       icon: <BookOpenIcon size={18} />,  label: t.api_docs_tab,       desc: t.new_api_docs_desc, hidden: true },
    { id: "loop-studio",    type: "loop-studio",    icon: <RefreshIcon size={18} />,   label: t.loop_studio_tab,    desc: t.new_loop_studio_desc },
    { id: "code-assistant", type: "code-assistant", icon: <CodeIcon size={18} />,      label: t.code_assistant_tab, desc: t.new_code_assistant_desc },
    { id: "knowledge-base", type: "knowledge-base", icon: <LibraryIcon size={18} />,   label: t.knowledge_base_tab, desc: t.new_knowledge_base_desc },
    // The Mail entry is hidden while the feature is not being shipped to
    // users. The "mail" tab type, MailView, and its whole backend remain wired
    // up — restoring it means putting this line back, plus the matching button
    // in SettingsView, and re-adding the MailIcon import above. See the notes in
    // docs/superpowers/specs/2026-08-04-ai-mail-assistant-design.md.
    { id: "mail",           type: "mail",           icon: <MailIcon size={18} />,      label: t.mail_tab,           desc: t.new_mail_desc, hidden: true },
    // Claude Code 不是獨立的 TabType，而是「終端機分頁 + claudeBridge 選項」，
    // 且需要橋接 server 正在跑才能用（見 requiresBridge）。放在陣列最後，
    // 跟它在新增分頁選單裡的位置一致。
    { id: "claude-code",    type: "terminal",       opts: { claudeBridge: true }, icon: <RobotIcon size={18} />, label: t.bridge_new_tab, desc: t.bridge_new_tab_desc, requiresBridge: true, disabledHint: t.bridge_new_tab_disabled_hint },
  ];
}

export function visibleTabCatalog(t: Translations): TabCatalogEntry[] {
  return getTabCatalog(t).filter((e) => !e.hidden);
}
