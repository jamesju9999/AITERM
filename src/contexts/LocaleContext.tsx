import { createContext, useContext, useState, type ReactNode } from "react";
import { type Locale, type Translations, translations, LOCALE_STORAGE_KEY } from "../lib/i18n";

interface LocaleContextValue {
  locale: Locale;
  t: Translations;
  setLocale: (l: Locale) => void;
}

const LocaleContext = createContext<LocaleContextValue | null>(null);

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
