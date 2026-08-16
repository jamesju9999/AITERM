import type { Translations } from "../../lib/i18n";
import type { TabType } from "../TabBar";
import {
  TerminalIcon, DatabaseIcon, PaintbrushIcon, LinkIcon, BranchIcon,
  FileTextIcon, BookOpenIcon, RefreshIcon, CodeIcon, LibraryIcon, MailIcon,
} from "../Icons";

export interface TabCatalogEntry {
  type: TabType;
  icon: React.ReactNode;
  label: string;
  desc: string;
  /** 後端完整但尚未對使用者開放。隱藏的理由集中記在這裡，不要散在各個呼叫端。 */
  hidden?: boolean;
}

/** 分頁類型的唯一清單。NewTabPicker、首頁大圖入口、AI 路由提示詞都用這一份。 */
export function getTabCatalog(t: Translations): TabCatalogEntry[] {
  return [
    { type: "terminal",       icon: <TerminalIcon size={18} />,  label: t.terminal_tab,       desc: t.new_terminal_desc },
    { type: "database",       icon: <DatabaseIcon size={18} />,  label: t.database_tab,       desc: t.new_database_desc },
    { type: "design",         icon: <PaintbrushIcon size={18} />, label: t.design_tab,        desc: t.new_design_desc },
    { type: "cross-db",       icon: <LinkIcon size={18} />,      label: t.cross_db_tab,       desc: t.new_cross_db_desc },
    { type: "vcs",            icon: <BranchIcon size={18} />,    label: t.vcs_tab,            desc: t.new_vcs_desc },
    { type: "doc-converter",  icon: <FileTextIcon size={18} />,  label: t.doc_converter_tab,  desc: t.new_doc_converter_desc },
    { type: "api-docs",       icon: <BookOpenIcon size={18} />,  label: t.api_docs_tab,       desc: t.new_api_docs_desc },
    { type: "loop-studio",    icon: <RefreshIcon size={18} />,   label: t.loop_studio_tab,    desc: t.new_loop_studio_desc },
    { type: "code-assistant", icon: <CodeIcon size={18} />,      label: t.code_assistant_tab, desc: t.new_code_assistant_desc },
    { type: "knowledge-base", icon: <LibraryIcon size={18} />,   label: t.knowledge_base_tab, desc: t.new_knowledge_base_desc },
    { type: "mail",           icon: <MailIcon size={18} />,      label: t.mail_tab,           desc: t.new_mail_desc, hidden: true },
  ];
}

export function visibleTabCatalog(t: Translations): TabCatalogEntry[] {
  return getTabCatalog(t).filter((e) => !e.hidden);
}
