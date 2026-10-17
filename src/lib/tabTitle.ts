import { translations, type Translations } from "./i18n";
import type { Tab } from "../components/TabBar";

// 分頁類型 → 建立時用的預設標題字串（跟 TerminalApp 開新分頁時一致）。
const DEFAULT_TITLE_KEY: Partial<Record<Tab["type"], keyof Translations>> = {
  terminal: "terminal_tab",
  database: "database_tab",
  design: "design_tab",
  "cross-db": "cross_db_tab",
  vcs: "vcs_tab",
  "doc-converter": "doc_converter_tab",
  "api-docs": "api_docs_tab",
  "loop-studio": "loop_studio_tab",
  "code-assistant": "code_assistant_tab",
  "knowledge-base": "knowledge_base_tab",
  mail: "mail_tab",
  "agent-canvas": "agent_canvas_tab",
};

/**
 * 分頁標題是建立當下的語言存下來的；切換語言後，還是預設標題的分頁跟著換成現在的語言，
 * 使用者自己改過的標題照原樣顯示。
 */
export function displayTabTitle(tab: Pick<Tab, "type" | "title">, t: Translations): string {
  const key = DEFAULT_TITLE_KEY[tab.type];
  if (!key) return tab.title;
  const isDefault = Object.values(translations).some((loc) => loc[key] === tab.title);
  return isDefault ? (t[key] as string) : tab.title;
}
