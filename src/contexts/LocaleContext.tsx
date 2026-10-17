import { createContext, useContext, useState, type ReactNode } from "react";
import { type Locale, type Translations, translations, LOCALE_STORAGE_KEY } from "../lib/i18n";

interface LocaleContextValue {
  locale: Locale;
  t: Translations;
  setLocale: (l: Locale) => void;
}

// 開發模式改 i18n.ts 會熱更新這個模組；每次都 createContext 的話，main.tsx 手上舊的
// Provider 跟新的 useLocale 就對不上，setLocale 掉進下面的空函式（點語言沒反應）。
// 掛在 globalThis 上，熱更新前後用同一個 Context。
const g = globalThis as { __aitermLocaleContext?: React.Context<LocaleContextValue | null> };
const LocaleContext = (g.__aitermLocaleContext ??= createContext<LocaleContextValue | null>(null));

export function LocaleProvider({ children }: { children: ReactNode }) {
  const [locale, setLocaleState] = useState<Locale>(() => {
    const stored = localStorage.getItem(LOCALE_STORAGE_KEY);
    return stored === "en" ? "en" : "zh-TW";
  });

  const setLocale = (l: Locale) => {
    localStorage.setItem(LOCALE_STORAGE_KEY, l);
    setLocaleState(l);
  };

  return (
    <LocaleContext.Provider value={{ locale, t: translations[locale], setLocale }}>
      {children}
    </LocaleContext.Provider>
  );
}

export function useLocale(): LocaleContextValue {
  const ctx = useContext(LocaleContext);
  if (!ctx) {
    const stored = localStorage.getItem(LOCALE_STORAGE_KEY);
    const locale: Locale = stored === "en" ? "en" : "zh-TW";
    return {
      locale,
      t: translations[locale],
      setLocale: () => {},
    };
  }
  return ctx;
}
