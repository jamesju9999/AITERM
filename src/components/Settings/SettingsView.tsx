import { useState, type ReactNode } from "react";
import { useNavigate, useLocation } from "react-router-dom";
import { ProvidersPage } from "./ProvidersPage";
import { GeneralPage } from "./GeneralPage";
import { DatabaseConnectionsPage } from "./DatabaseConnectionsPage";
import { SharedServerPage } from "./SharedServerPage";
import { VcsConnectionsPage } from "./VcsConnectionsPage";
import { MailAccountsPage } from "./MailAccountsPage";
import { EnterprisePage } from "./EnterprisePage";
import { AboutPage } from "./AboutPage";
import { McpServersPage } from "./McpServersPage";
import { ClaudeBridgePage } from "./ClaudeBridgePage";
import { McpToolServerPage } from "./McpToolServerPage";
import { OpenAiServerPage } from "./OpenAiServerPage";
import { ComputerUsePage } from "./ComputerUsePage";
import { TaskBoardPage } from "./TaskBoardPage";
import { UsagePage } from "./UsagePage";
import { useLocale } from "../../contexts/LocaleContext";
import {
  SettingsIcon,
  RobotIcon,
  DatabaseIcon,
  BranchIcon,
  WrenchIcon,
  InfoIcon,
  LinkIcon,
  ZapIcon,
  SparklesIcon,
  ClipboardListIcon,
  MonitorIcon,
  MailIcon,
} from "../Icons";
import type { Translations } from "../../lib/i18n";
import "./SettingsView.css";

type SettingsTab = "general" | "providers" | "databases" | "vcs" | "enterprise" | "about" | "mcp" | "mail" | "bridge" | "usage" | "mcpToolServer" | "openaiServer" | "computerUse" | "taskBoard" | "sharedServer";

type SidebarEntry =
  | { heading: string }
  | { tab: SettingsTab; icon: ReactNode; label: string };

function SIDEBAR_ITEMS(t: Translations): SidebarEntry[] {
  const item = (tab: SettingsTab, icon: ReactNode, label: string): SidebarEntry => ({ tab, icon, label });
  return [
    item("general", <SettingsIcon size={16} />, t.general),
    { heading: t.settings_group_ai },
    item("providers", <RobotIcon size={16} />, t.ai_providers),
    item("bridge", <LinkIcon size={16} />, t.bridge_title),
    item("openaiServer", <LinkIcon size={16} />, t.openai_server_title),
    item("usage", <ZapIcon size={16} />, t.usage_quota_title),
    { heading: t.settings_group_connections },
    item("sharedServer", <LinkIcon size={16} />, t.settings_tab_sharedServer),
    item("databases", <DatabaseIcon size={16} />, t.db_connections),
    item("vcs", <BranchIcon size={16} />, t.vcs_connections),
    item("mail", <MailIcon size={16} />, t.mail_accounts_settings_title),
    item("mcp", <WrenchIcon size={16} />, t.mcp_servers),
    item("mcpToolServer", <SparklesIcon size={16} />, t.mcp_tool_server_title),
    { heading: t.settings_group_automation },
    item("taskBoard", <ClipboardListIcon size={16} />, t.settings_tab_taskBoard),
    item("computerUse", <MonitorIcon size={16} />, t.settings_tab_computerUse),
    { heading: "" },
    item("about", <InfoIcon size={16} />, t.about),
  ];
}

export function SettingsView() {
  const navigate = useNavigate();
  const location = useLocation();
  const initialTab = (location.state as { tab?: SettingsTab } | null)?.tab ?? "general";
  const [tab, setTab] = useState<SettingsTab>(initialTab);
  const { t } = useLocale();

  return (
    <div className="settings-view">
      {/*
        Sidebar. Doubles as the window drag region: this view is an overlay on
        top of TerminalApp, which is visibility:hidden while it shows — so the
        TitleBar and TabBar that normally carry data-tauri-drag-region are not
        reachable here. Without this the window cannot be moved at all until
        the user goes back to the terminal.

        Same placement as .aiterm-tabbar: the attribute sits on the container,
        and the buttons inside still receive their own clicks because Tauri
        only starts a drag when the event target itself is a drag region.
        Deliberately NOT on .settings-content — that scrolls, and a drag
        region there would fight with scrolling and text selection.
      */}
      <nav className="settings-sidebar" data-tauri-drag-region>
        <div className="settings-sidebar-title" data-tauri-drag-region>{t.settings_title}</div>

        {/*
          分組：一般置頂、關於置底，中間依用途分成三組。企業版分頁在側邊欄是隱藏的
          （分頁、頁面、下方的 render 分支都還在，要恢復只要在這裡加回一項）。
        */}
        <div className="sidebar-scroll" data-tauri-drag-region>
        {SIDEBAR_ITEMS(t).map((entry, i) =>
          "heading" in entry ? (
            entry.heading
              ? <div key={`h${i}`} className="sidebar-group-heading">{entry.heading}</div>
              : <div key={`h${i}`} className="sidebar-group-divider" />
          ) : (
            <button
              key={entry.tab}
              className={`sidebar-item ${tab === entry.tab ? "sidebar-item--active" : ""}`}
              onClick={() => setTab(entry.tab)}
            >
              {entry.icon} {entry.label}
            </button>
          ),
        )}

        {/*
          flex:1 — 這個 spacer 吃掉側邊欄所有剩餘的垂直空間，也就是畫面上那
          一大片空白。拖曳屬性不會從 <nav> 繼承下來，少了這行就等於整片最好
          抓的區域都不能拖。
        */}
        <div className="sidebar-spacer" data-tauri-drag-region />
        </div>

        <button className="sidebar-back" onClick={() => navigate("/")}>
          {t.back_to_terminal}
        </button>
      </nav>

      {/*
        Content. 頂端那條拖曳條是 .settings-content 的兄弟、不是子元素：它不
        跟著捲動，所以不會跟捲動或選字打架（上面說不放在 .settings-content
        的理由）。高度正好取代原本的 padding-top，畫面不變。
      */}
      <div className="settings-main">
      <div className="settings-content-drag" data-tauri-drag-region />
      <main className="settings-content">
        {tab === "general" && <GeneralPage />}
        {tab === "providers" && <ProvidersPage />}
        {tab === "sharedServer" && <SharedServerPage />}
        {tab === "databases" && <DatabaseConnectionsPage />}
        {tab === "vcs" && <VcsConnectionsPage />}
        {tab === "mail" && <MailAccountsPage />}
        {tab === "mcp" && <McpServersPage />}
        {tab === "bridge" && <ClaudeBridgePage />}
        {tab === "mcpToolServer" && <McpToolServerPage />}
        {tab === "openaiServer" && <OpenAiServerPage />}
        {tab === "computerUse" && <ComputerUsePage />}
        {tab === "taskBoard" && <TaskBoardPage />}
        {tab === "usage" && <UsagePage />}
        {tab === "enterprise" && <EnterprisePage />}
        {tab === "about" && <AboutPage />}
      </main>
      </div>
    </div>
  );
}
