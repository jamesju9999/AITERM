import { useEffect, useState } from "react";
import { MemoryRouter, Routes, Route, useNavigate, useLocation } from "react-router-dom";
import { TerminalApp } from "./components/TerminalApp";
import { SettingsView } from "./components/Settings/SettingsView";
import { OnboardingWizard } from "./components/Onboarding/OnboardingWizard";
import { isOnboardingDone } from "./ipc/config";
import "./App.css";

function AppRoutes() {
  const navigate = useNavigate();
  const location = useLocation();
  const [ready, setReady] = useState(false);

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
        navigate("/settings");
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [navigate]);

  if (!ready) return null;

  const isTerminal = location.pathname === "/";

  return (
    <div style={{ position: "relative", width: "100vw", height: "100vh", overflow: "hidden" }}>
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
        <TerminalApp />
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
