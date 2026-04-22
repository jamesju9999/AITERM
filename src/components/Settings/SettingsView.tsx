import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { ProvidersPage } from "./ProvidersPage";
import { GeneralPage } from "./GeneralPage";
import { DatabaseConnectionsPage } from "./DatabaseConnectionsPage";
import { AboutPage } from "./AboutPage";
import { useLocale } from "../../contexts/LocaleContext";
import "./SettingsView.css";

type SettingsTab = "general" | "providers" | "databases" | "about";

export function SettingsView() {
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
        {tab === "about" && <AboutPage />}
      </main>
    </div>
  );
}
