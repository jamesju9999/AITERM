import { useEffect, useState } from "react";
import { MemoryRouter, Routes, Route, useNavigate, useLocation } from "react-router-dom";
import { TerminalApp } from "./components/TerminalApp";
import { SettingsView } from "./components/Settings/SettingsView";
import { OnboardingWizard } from "./components/Onboarding/OnboardingWizard";
import { UpdateModal } from "./components/UpdateModal";
import { AppImageIntegrationPrompt } from "./components/AppImageIntegrationPrompt";
import { useUpdaterContext } from "./contexts/UpdaterContext";
import { isOnboardingDone } from "./ipc/config";
import "./App.css";

export function AppRoutes() {
  const navigate = useNavigate();
  const location = useLocation();
  const [ready, setReady] = useState(false);
  const { hasUpdate } = useUpdaterContext();

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
        <TerminalApp hasUpdate={hasUpdate} />
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
      <UpdateModal />
      <AppImageIntegrationPrompt hasUpdate={hasUpdate} />
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
