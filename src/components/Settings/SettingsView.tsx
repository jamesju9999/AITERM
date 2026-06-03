import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { ProvidersPage } from "./ProvidersPage";
import { GeneralPage } from "./GeneralPage";
import { DatabaseConnectionsPage } from "./DatabaseConnectionsPage";
import { VcsConnectionsPage } from "./VcsConnectionsPage";
import { AboutPage } from "./AboutPage";
import { EnterprisePage } from "./EnterprisePage";
import { useLocale } from "../../contexts/LocaleContext";
import "./SettingsView.css";

type SettingsTab = "general" | "providers" | "databases" | "vcs" | "enterprise" | "about";

interface UpdateInfo {
  hasUpdate: boolean;
  latestVersion: string;
}

export function SettingsView({ updateInfo }: { updateInfo?: UpdateInfo }) {
  const navigate = useNavigate();
  const [tab, setTab] = useState<SettingsTab>("general");
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
          ⚙️ {t.general}
        </button>
        <button
          className={`sidebar-item ${tab === "providers" ? "sidebar-item--active" : ""}`}
          onClick={() => setTab("providers")}
        >
          🤖 {t.ai_providers}
        </button>
        <button
          className={`sidebar-item ${tab === "databases" ? "sidebar-item--active" : ""}`}
          onClick={() => setTab("databases")}
        >
          🗄️ {t.db_connections}
        </button>
        <button
          className={`sidebar-item ${tab === "vcs" ? "sidebar-item--active" : ""}`}
          onClick={() => setTab("vcs")}
        >
          🔀 {t.vcs_connections}
        </button>
        <button
          className={`sidebar-item ${tab === "enterprise" ? "sidebar-item--active" : ""}`}
          onClick={() => setTab("enterprise")}
        >
          🏢 Enterprise
        </button>
        <button
          className={`sidebar-item ${tab === "about" ? "sidebar-item--active" : ""}`}
          onClick={() => setTab("about")}
        >
          ℹ️ {t.about}
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
        {tab === "enterprise" && <EnterprisePage />}
        {tab === "about" && (
          <AboutPage initialLatestVersion={updateInfo?.latestVersion} />
        )}
      </main>
    </div>
  );
}
