import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { ProvidersPage } from "./ProvidersPage";
import { GeneralPage } from "./GeneralPage";
import { DatabaseConnectionsPage } from "./DatabaseConnectionsPage";
import "./SettingsView.css";

type SettingsTab = "providers" | "general" | "databases";

export function SettingsView() {
  const navigate = useNavigate();
  const [tab, setTab] = useState<SettingsTab>("providers");

  return (
    <div className="settings-view">
      {/* Sidebar */}
      <nav className="settings-sidebar">
        <div className="settings-sidebar-title">設定</div>

        <button
          className={`sidebar-item ${tab === "providers" ? "sidebar-item--active" : ""}`}
          onClick={() => setTab("providers")}
        >
          AI Providers
        </button>
        <button
          className={`sidebar-item ${tab === "general" ? "sidebar-item--active" : ""}`}
          onClick={() => setTab("general")}
        >
          一般
        </button>
        <button
          className={`sidebar-item ${tab === "databases" ? "sidebar-item--active" : ""}`}
          onClick={() => setTab("databases")}
        >
          🗄️ 資料庫連線
        </button>

        <div className="sidebar-spacer" />

        <button className="sidebar-back" onClick={() => navigate("/")}>
          ← 回到終端機
        </button>
      </nav>

      {/* Content */}
      <main className="settings-content">
        {tab === "providers" && <ProvidersPage />}
        {tab === "general" && <GeneralPage />}
        {tab === "databases" && <DatabaseConnectionsPage />}
      </main>
    </div>
  );
}
