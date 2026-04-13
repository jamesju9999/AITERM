import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { ProvidersPage } from "./ProvidersPage";
import { GeneralPage } from "./GeneralPage";
import "./SettingsView.css";

type SettingsTab = "providers" | "general";

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

        <div className="sidebar-spacer" />

        <button className="sidebar-back" onClick={() => navigate("/")}>
          ← 回到終端機
        </button>
      </nav>

      {/* Content */}
      <main className="settings-content">
        {tab === "providers" && <ProvidersPage />}
        {tab === "general" && <GeneralPage />}
      </main>
    </div>
  );
}
