import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { LocaleProvider } from "./contexts/LocaleContext";
import { getActiveTheme, applyTheme } from "./lib/themes";

// Apply saved theme before first render to avoid flash
applyTheme(getActiveTheme());

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <LocaleProvider>
      <App />
    </LocaleProvider>
  </React.StrictMode>,
);
