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
  architecture: "arch_tab",
};

/**
 * 分頁標題是建立當下的語言存下來的；切換語言後，還是預設標題的分頁跟著換成現在的語言，
 * 使用者自己改過的標題照原樣顯示。
 *
 * Agent 協作分頁還沒改過名時帶上看板專案名稱：`short`（側邊欄，旁邊已經有圖示）只顯示
 * 專案名，其他地方顯示「Agent 協作 · 專案名」，開兩個分頁才分得出來。
 */
export function displayTabTitle(
  tab: Pick<Tab, "type" | "title" | "agentCanvasProjectName">,
  t: Translations,
  variant: "short" | "long" = "long",
): string {
  const key = DEFAULT_TITLE_KEY[tab.type];
  if (!key) return tab.title;
  const isDefault = Object.values(translations).some((loc) => loc[key] === tab.title);
  if (!isDefault) return tab.title;
  const base = t[key] as string;
  const project = tab.type === "agent-canvas" ? tab.agentCanvasProjectName : undefined;
  if (!project) return base;
  return variant === "short" ? project : `${base} · ${project}`;
}
