import type { ITheme } from "@xterm/xterm";

export type ThemeId = "dark" | "light" | "nord" | "dracula";

export interface AppTheme {
  id: ThemeId;
  label: string;
  xterm: ITheme;
  /** CSS variables applied to :root */
  css: Record<string, string>;
}

export const THEMES: AppTheme[] = [
  {
    id: "dark",
    label: "Dark (default)",
    xterm: {
      background: "#0c0c0c",
      foreground: "#e6e6e6",
      cursor: "#e6e6e6",
      selectionBackground: "#2a4a3a",
      black: "#000000",
      red: "#e35757",
      green: "#34d399",
      yellow: "#fbbf24",
      blue: "#60a5fa",
      magenta: "#a78bfa",
      cyan: "#22d3ee",
      white: "#e6e6e6",
      brightBlack: "#555",
      brightRed: "#f87171",
      brightGreen: "#6ee7b7",
      brightYellow: "#fde68a",
      brightBlue: "#93c5fd",
      brightMagenta: "#c4b5fd",
      brightCyan: "#67e8f9",
      brightWhite: "#ffffff",
    },
    css: {
      "--bg-primary": "#0c0c0c",
      "--bg-secondary": "#111",
      "--bg-tertiary": "#1a1a1a",
      "--bg-surface": "#222",
      "--border-color": "#2a2a2a",
      "--border-subtle": "#333",
      "--text-primary": "#e6e6e6",
      "--text-secondary": "#aaa",
      "--text-muted": "#666",
      "--accent": "#34d399",
      "--accent-dim": "#0f2e23",
    },
  },
  {
    id: "light",
    label: "Light",
    xterm: {
      background: "#f8f8f8",
      foreground: "#1e1e1e",
      cursor: "#1e1e1e",
      selectionBackground: "#b5d5ff",
      black: "#1e1e1e",
      red: "#d73737",
      green: "#1a8a1a",
      yellow: "#b86100",
      blue: "#0060c7",
      magenta: "#6b2fb6",
      cyan: "#0077aa",
      white: "#666",
      brightBlack: "#444",
      brightRed: "#e03030",
      brightGreen: "#207820",
      brightYellow: "#c07000",
      brightBlue: "#1e7ed8",
      brightMagenta: "#8045c8",
      brightCyan: "#0085c7",
      brightWhite: "#111",
    },
    css: {
      "--bg-primary": "#f8f8f8",
      "--bg-secondary": "#efefef",
      "--bg-tertiary": "#e8e8e8",
      "--bg-surface": "#fff",
      "--border-color": "#d5d5d5",
      "--border-subtle": "#ccc",
      "--text-primary": "#1e1e1e",
      "--text-secondary": "#555",
      "--text-muted": "#999",
      "--accent": "#1a8a1a",
      "--accent-dim": "#d8f0d8",
    },
  },
  {
    id: "nord",
    label: "Nord",
    xterm: {
      background: "#2e3440",
      foreground: "#d8dee9",
      cursor: "#d8dee9",
      selectionBackground: "#434c5e",
      black: "#3b4252",
      red: "#bf616a",
      green: "#a3be8c",
      yellow: "#ebcb8b",
      blue: "#81a1c1",
      magenta: "#b48ead",
      cyan: "#88c0d0",
      white: "#e5e9f0",
      brightBlack: "#4c566a",
      brightRed: "#bf616a",
      brightGreen: "#a3be8c",
      brightYellow: "#ebcb8b",
      brightBlue: "#81a1c1",
      brightMagenta: "#b48ead",
      brightCyan: "#8fbcbb",
      brightWhite: "#eceff4",
    },
    css: {
      "--bg-primary": "#2e3440",
      "--bg-secondary": "#3b4252",
      "--bg-tertiary": "#434c5e",
      "--bg-surface": "#4c566a",
      "--border-color": "#434c5e",
      "--border-subtle": "#4c566a",
      "--text-primary": "#d8dee9",
      "--text-secondary": "#81a1c1",
      "--text-muted": "#616e88",
      "--accent": "#88c0d0",
      "--accent-dim": "#3b4f5c",
    },
  },
  {
    id: "dracula",
    label: "Dracula",
    xterm: {
      background: "#282a36",
      foreground: "#f8f8f2",
      cursor: "#f8f8f2",
      selectionBackground: "#44475a",
      black: "#21222c",
      red: "#ff5555",
      green: "#50fa7b",
      yellow: "#f1fa8c",
      blue: "#bd93f9",
      magenta: "#ff79c6",
      cyan: "#8be9fd",
      white: "#f8f8f2",
      brightBlack: "#6272a4",
      brightRed: "#ff6e6e",
      brightGreen: "#69ff94",
      brightYellow: "#ffffa5",
      brightBlue: "#d6acff",
      brightMagenta: "#ff92df",
      brightCyan: "#a4ffff",
      brightWhite: "#ffffff",
    },
    css: {
      "--bg-primary": "#282a36",
      "--bg-secondary": "#21222c",
      "--bg-tertiary": "#343746",
      "--bg-surface": "#44475a",
      "--border-color": "#44475a",
      "--border-subtle": "#3d3f4f",
      "--text-primary": "#f8f8f2",
      "--text-secondary": "#bd93f9",
      "--text-muted": "#6272a4",
      "--accent": "#50fa7b",
      "--accent-dim": "#1a3a24",
    },
  },
];

export const THEME_STORAGE_KEY = "aiterm-theme";

export function getActiveTheme(): AppTheme {
  const id = localStorage.getItem(THEME_STORAGE_KEY) as ThemeId | null;
  return THEMES.find((t) => t.id === id) ?? THEMES[0];
}

export function applyTheme(theme: AppTheme) {
  const root = document.documentElement;
  for (const [k, v] of Object.entries(theme.css)) {
    root.style.setProperty(k, v);
  }
  root.setAttribute("data-theme", theme.id);
  localStorage.setItem(THEME_STORAGE_KEY, theme.id);
  window.dispatchEvent(new CustomEvent("aiterm:theme-changed", { detail: { theme } }));
}
