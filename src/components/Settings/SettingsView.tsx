import { useState } from "react";
import { useNavigate, useLocation } from "react-router-dom";
import { ProvidersPage } from "./ProvidersPage";
import { GeneralPage } from "./GeneralPage";
import { DatabaseConnectionsPage } from "./DatabaseConnectionsPage";
import { VcsConnectionsPage } from "./VcsConnectionsPage";
import { MailAccountsPage } from "./MailAccountsPage";
import { EnterprisePage } from "./EnterprisePage";
import { AboutPage } from "./AboutPage";
import { McpServersPage } from "./McpServersPage";
import { ClaudeBridgePage } from "./ClaudeBridgePage";
import { useLocale } from "../../contexts/LocaleContext";
import {
  SettingsIcon,
  RobotIcon,
  DatabaseIcon,
  BranchIcon,
  WrenchIcon,
  InfoIcon,
  LinkIcon
} from "../Icons";
import "./SettingsView.css";

type SettingsTab = "general" | "providers" | "databases" | "vcs" | "enterprise" | "about" | "mcp" | "mail" | "bridge";

export function SettingsView() {
  const navigate = useNavigate();
  const location = useLocation();
  const initialTab = (location.state as { tab?: SettingsTab } | null)?.tab ?? "general";
  const [tab, setTab] = useState<SettingsTab>(initialTab);
  const { t } = useLocale();

  return (
    <div className="settings-view">
      {/* Sidebar */}
      <nav className="settings-sidebar">
        <div className="settings-sidebar-title">{t.settings_title}</div>

        <button
          className={`sidebar-item ${tab === "general" ? "sidebar-item--active" : ""}`}
          onClick={() => setTab("general")}
        >
          <SettingsIcon size={16} /> {t.general}
        </button>
        <button
          className={`sidebar-item ${tab === "providers" ? "sidebar-item--active" : ""}`}
          onClick={() => setTab("providers")}
        >
          <RobotIcon size={16} /> {t.ai_providers}
        </button>
        <button
          className={`sidebar-item ${tab === "databases" ? "sidebar-item--active" : ""}`}
          onClick={() => setTab("databases")}
        >
          <DatabaseIcon size={16} /> {t.db_connections}
        </button>
        <button
          className={`sidebar-item ${tab === "vcs" ? "sidebar-item--active" : ""}`}
          onClick={() => setTab("vcs")}
        >
          <BranchIcon size={16} /> {t.vcs_connections}
        </button>
        {/*
          The Mail entry is hidden from the sidebar while the feature is not
          being shipped to users. The tab, its page and the render branch below
          are all still wired up, exactly like the Enterprise entry — restoring
          it means putting this button back, plus the matching item in
          NewTabPicker, and re-adding the MailIcon import above. Nothing
          else was removed. See the hand-off notes in
          docs/superpowers/specs/2026-08-04-ai-mail-assistant-design.md.

          <button
            className={`sidebar-item ${tab === "mail" ? "sidebar-item--active" : ""}`}
            onClick={() => setTab("mail")}
          >
            <MailIcon size={16} /> {t.mail_accounts_settings_title}
          </button>
        */}
        <button
          className={`sidebar-item ${tab === "mcp" ? "sidebar-item--active" : ""}`}
          onClick={() => setTab("mcp")}
        >
          <WrenchIcon size={16} /> {t.mcp_servers}
        </button>
        <button
          className={`sidebar-item ${tab === "bridge" ? "sidebar-item--active" : ""}`}
          onClick={() => setTab("bridge")}
        >
          <LinkIcon size={16} /> {t.bridge_title}
        </button>
        {/*
          The Enterprise entry is hidden from the sidebar. The tab, its page and
          the render branch below are all still wired up, so restoring it means
          putting this button back — nothing else was removed.
        */}
        <button
          className={`sidebar-item ${tab === "about" ? "sidebar-item--active" : ""}`}
          onClick={() => setTab("about")}
        >
          <InfoIcon size={16} /> {t.about}
        </button>

        <div className="sidebar-spacer" />

        <button className="sidebar-back" onClick={() => navigate("/")}>
          {t.back_to_terminal}
        </button>
      </nav>

      {/* Content */}
      <main className="settings-content">
        {tab === "general" && <GeneralPage />}
        {tab === "providers" && <ProvidersPage />}
        {tab === "databases" && <DatabaseConnectionsPage />}
        {tab === "vcs" && <VcsConnectionsPage />}
        {tab === "mail" && <MailAccountsPage />}
        {tab === "mcp" && <McpServersPage />}
        {tab === "bridge" && <ClaudeBridgePage />}
        {tab === "enterprise" && <EnterprisePage />}
        {tab === "about" && <AboutPage />}
      </main>
    </div>
  );
}
