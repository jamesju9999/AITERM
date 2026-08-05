import { useCallback, useEffect, useState } from "react";
import { MemoryRouter, Routes, Route, useNavigate, useLocation } from "react-router-dom";
import { TerminalApp } from "./components/TerminalApp";
import { SettingsView } from "./components/Settings/SettingsView";
import { OnboardingWizard } from "./components/Onboarding/OnboardingWizard";
import { UpdateModal } from "./components/UpdateModal";
import { AppImageIntegrationPrompt } from "./components/AppImageIntegrationPrompt";
import { ClaudeNotifPrompt } from "./components/ClaudeNotifPrompt";
import { useUpdaterContext } from "./contexts/UpdaterContext";
import { isOnboardingDone } from "./ipc/config";
import "./App.css";

export function AppRoutes() {
  const navigate = useNavigate();
  const location = useLocation();
  const [ready, setReady] = useState(false);
  const { hasUpdate } = useUpdaterContext();
  const [claudeSeen, setClaudeSeen] = useState(false);
  const [appImageOffering, setAppImageOffering] = useState(false);
  // hasUpdate（有沒有可用更新）不等於「更新卡片正在畫面上」——使用者按過
  // 「稍後」之後 hasUpdate 仍是 true（見 useUpdater.ts），所以改看 UpdateModal
  // 自己回報的實際顯示狀態，避免我們的卡片被一個已經不在畫面上的更新卡永久擋住。
  const [updateVisible, setUpdateVisible] = useState(false);
  // 三張角落卡片固定在右下角同一個位置，同時出現會完全重疊。
  // 優先序：更新 > AppImage > Claude 通知。
  const onClaudeDetected = useCallback(() => setClaudeSeen(true), []);

  useEffect(() => {
    // On mount: check if this is the first launch.
    isOnboardingDone()
      .then((done) => {
        if (!done) navigate("/onboarding", { replace: true });
      })
      .catch(() => {})
      .finally(() => setReady(true));
  }, [navigate]);

  // Keyboard shortcut: Ctrl+, → settings
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === ",") {
        e.preventDefault();
        navigate("/settings", hasUpdate ? { state: { tab: "about" } } : undefined);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
    // hasUpdate must stay in deps — the handler is registered once via
    // addEventListener, and without it the closure would keep seeing the
    // initial false value even after the async version-check resolves.
  }, [navigate, hasUpdate]);

  if (!ready) return null;

  const isTerminal = location.pathname === "/";

  return (
    <div style={{ position: "relative", width: "100%", height: "100%", overflow: "hidden" }}>
      {/* 
        TerminalApp sits permanently in the background. 
        This is critical so React Router doesn't unmount it, which would destroy 
        the active PTY sessions, WebGL canvas contexts, tabs, and layout markers.
      */}
      <div 
        style={{ 
          position: "absolute", inset: 0, zIndex: 0,
          visibility: isTerminal ? "visible" : "hidden",
          pointerEvents: isTerminal ? "auto" : "none"
        }}
      >
        <TerminalApp hasUpdate={hasUpdate} onClaudeDetected={onClaudeDetected} />
      </div>

      {/* Settings / Onboarding Overlays */}
      {!isTerminal && (
        <div style={{ position: "absolute", inset: 0, zIndex: 10, backgroundColor: "#0c0c0c", pointerEvents: "auto" }}>
          <Routes>
            <Route path="/settings/*" element={<SettingsView />} />
            <Route path="/onboarding" element={<OnboardingWizard />} />
          </Routes>
        </div>
      )}
      <UpdateModal onVisibleChange={setUpdateVisible} />
      <AppImageIntegrationPrompt hasUpdate={hasUpdate} onOfferingChange={setAppImageOffering} />
      <ClaudeNotifPrompt claudeSeen={claudeSeen} blocked={updateVisible || appImageOffering} />
    </div>
  );
}

function App() {
  return (
    <MemoryRouter initialEntries={["/"]}>
      <AppRoutes />
    </MemoryRouter>
  );
}

export default App;
