import { useEffect, useState } from "react";
import { MemoryRouter, Routes, Route, useNavigate } from "react-router-dom";
import { TerminalView } from "./components/TerminalView";
import { SettingsView } from "./components/Settings/SettingsView";
import { OnboardingWizard } from "./components/Onboarding/OnboardingWizard";
import { isOnboardingDone } from "./ipc/config";
import "./App.css";

function AppRoutes() {
  const navigate = useNavigate();
  const [ready, setReady] = useState(false);

  useEffect(() => {
    // On mount: check if this is the first launch.
    isOnboardingDone()
      .then((done) => {
        if (!done) navigate("/onboarding", { replace: true });
      })
      .catch(() => {
        // If the IPC call fails (e.g. during development without Tauri),
        // just show the terminal normally.
      })
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

  return (
    <Routes>
      <Route path="/" element={<TerminalView />} />
      <Route path="/settings/*" element={<SettingsView />} />
      <Route path="/onboarding" element={<OnboardingWizard />} />
    </Routes>
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
