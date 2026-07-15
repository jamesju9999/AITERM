import { LOCALE_STORAGE_KEY, translations } from "./i18n";

function getT() {
  const loc = (localStorage.getItem(LOCALE_STORAGE_KEY) || "zh-TW") as "zh-TW" | "en";
  return translations[loc] || translations["zh-TW"];
}

export function formatTime(ms: number): string {
  const loc = localStorage.getItem(LOCALE_STORAGE_KEY) || "zh-TW";
  return new Date(ms).toLocaleTimeString(loc, { hour12: false });
}

export function formatDuration(startMs: number, endMs: number): string {
  const s = Math.max(0, Math.round((endMs - startMs) / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  const rem = s % 60;
  const t = getT();
  return rem === 0 ? `${m}${t.time_unit_min}` : t.time_unit_min_sec(m, rem);
}
