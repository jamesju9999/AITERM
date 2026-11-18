import { render, screen, act } from "@testing-library/react";
import { describe, it, expect, vi, afterEach } from "vitest";
import { SettingsLoading } from "./SettingsLoading";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { translations, LOCALE_STORAGE_KEY } from "../../lib/i18n";

const t = translations["zh-TW"];

describe("SettingsLoading", () => {
  afterEach(() => vi.useRealTimers());

  it("shows the loading line at once and adds the macOS prompt hint only after a delay", () => {
    vi.useFakeTimers();
    localStorage.setItem(LOCALE_STORAGE_KEY, "zh-TW");
    render(<LocaleProvider><SettingsLoading /></LocaleProvider>);
    expect(screen.getByText(t.settings_loading)).toBeInTheDocument();
    expect(screen.queryByText(t.settings_loading_slow_hint)).toBeNull();
    act(() => { vi.advanceTimersByTime(1600); });
    expect(screen.getByText(t.settings_loading_slow_hint)).toBeInTheDocument();
  });
});
