import { useState, useEffect } from "react";
import { getConfig, setExecutionMode, setSubmitShortcut, setMaxAgentSteps, setDefaultTab } from "../../ipc/config";
import type { ExecutionMode, SubmitShortcut, DefaultTab } from "../../ipc/config";
import { useLocale } from "../../contexts/LocaleContext";
import type { Locale } from "../../lib/i18n";
import "./GeneralPage.css";

const STEP_VALUES = [5, 10, 20, 50, 100, 0];
const DEFAULT_TAB_STORAGE_KEY = "aiterm_default_tab";

export function GeneralPage() {
  const { t, locale, setLocale } = useLocale();
  const [mode, setMode] = useState<ExecutionMode>("always-confirm");
  const [shortcut, setShortcut] = useState<SubmitShortcut>("enter");
  const [maxSteps, setMaxSteps] = useState<number>(5);
  const [defaultTab, setDefaultTabState] = useState<DefaultTab>("terminal");
  const [saving, setSaving] = useState(false);

  const MODES: { value: ExecutionMode; label: string; desc: string }[] = [
    { value: "always-confirm", label: t.mode_always_confirm_label, desc: t.mode_always_confirm_desc },
    { value: "graded",         label: t.mode_graded_label,         desc: t.mode_graded_desc },
    { value: "full-auto",      label: t.mode_full_auto_label,      desc: t.mode_full_auto_desc },
  ];

  const SHORTCUT_MODES: { value: SubmitShortcut; label: string; desc: string }[] = [
    { value: "enter",       label: "Enter",           desc: t.shortcut_enter_desc },
    { value: "shift-enter", label: "Shift + Enter",   desc: t.shortcut_shift_enter_desc },
    { value: "ctrl-enter",  label: "Ctrl + Enter",    desc: t.shortcut_ctrl_enter_desc },
  ];

  useEffect(() => {
    getConfig().then((cfg) => {
      setMode(cfg.execution_mode);
      setShortcut(cfg.submit_shortcut);
      setMaxSteps(cfg.max_agent_steps ?? 5);
      setDefaultTabState(cfg.default_tab ?? "terminal");
    });
  }, []);

  const handleChange = async (newMode: ExecutionMode) => {
    setMode(newMode);
    setSaving(true);
    try { await setExecutionMode(newMode); } finally { setSaving(false); }
  };

  const handleShortcutChange = async (newShortcut: SubmitShortcut) => {
    setShortcut(newShortcut);
    setSaving(true);
    try { await setSubmitShortcut(newShortcut); } finally { setSaving(false); }
  };

  const handleMaxStepsChange = async (newSteps: number) => {
    setMaxSteps(newSteps);
    setSaving(true);
    try { await setMaxAgentSteps(newSteps); } finally { setSaving(false); }
  };

  const handleDefaultTabChange = async (tab: DefaultTab) => {
    setDefaultTabState(tab);
    // Cache synchronously in localStorage so TerminalApp reads it at next startup
    localStorage.setItem(DEFAULT_TAB_STORAGE_KEY, tab);
    setSaving(true);
    try { await setDefaultTab(tab); } finally { setSaving(false); }
  };

  return (
    <div className="general-page">
      <h2>{t.general_settings}</h2>

      <section className="settings-section">
        <h3>{t.language}</h3>
        <p className="section-desc">{t.language_desc}</p>
        <div className="mode-list">
          {(["zh-TW", "en"] as Locale[]).map((l) => (
            <label key={l} className="mode-option">
              <input
                type="radio"
                name="locale"
                value={l}
                checked={locale === l}
                onChange={() => setLocale(l)}
              />
              <div className="mode-text">
                <span className="mode-label">{l === "zh-TW" ? "繁體中文" : "English"}</span>
              </div>
            </label>
          ))}
        </div>
      </section>

      <section className="settings-section">
        <h3>{t.default_tab}</h3>
        <p className="section-desc">{t.default_tab_desc}</p>
        <div className="mode-list">
          {([
            { value: "terminal" as DefaultTab, label: t.terminal_tab },
            { value: "database" as DefaultTab, label: t.database_tab },
          ]).map((opt) => (
            <label key={opt.value} className="mode-option">
              <input
                type="radio"
                name="default_tab"
                value={opt.value}
                checked={defaultTab === opt.value}
                onChange={() => handleDefaultTabChange(opt.value)}
                disabled={saving}
              />
              <div className="mode-text">
                <span className="mode-label">{opt.label}</span>
              </div>
            </label>
          ))}
        </div>
      </section>

      <section className="settings-section">
        <h3>{t.execution_mode}</h3>
        <p className="section-desc">{t.execution_mode_desc}</p>
        <div className="mode-list">
          {MODES.map((m) => (
            <label key={m.value} className="mode-option">
              <input
                type="radio"
                name="execution_mode"
                value={m.value}
                checked={mode === m.value}
                onChange={() => handleChange(m.value)}
                disabled={saving}
              />
              <div className="mode-text">
                <span className="mode-label">{m.label}</span>
                <span className="mode-desc">{m.desc}</span>
              </div>
            </label>
          ))}
        </div>
      </section>

      <section className="settings-section">
        <h3>{t.agent_max_steps}</h3>
        <p className="section-desc">{t.agent_max_steps_desc}</p>
        <div className="step-select-row">
          <select
            className="step-select"
            value={maxSteps}
            onChange={(e) => handleMaxStepsChange(Number(e.target.value))}
            disabled={saving}
          >
            {STEP_VALUES.map((v) => (
              <option key={v} value={v}>
                {v === 0 ? t.steps_unlimited : t.steps_n(v)}
              </option>
            ))}
          </select>
          {maxSteps === 0 && (
            <span className="step-warning">{t.steps_unlimited_warning}</span>
          )}
        </div>
      </section>

      <section className="settings-section">
        <h3>{t.submit_shortcut}</h3>
        <p className="section-desc">{t.submit_shortcut_desc}</p>
        <div className="mode-list">
          {SHORTCUT_MODES.map((s) => (
            <label key={s.value} className="mode-option">
              <input
                type="radio"
                name="submit_shortcut"
                value={s.value}
                checked={shortcut === s.value}
                onChange={() => handleShortcutChange(s.value)}
                disabled={saving}
              />
              <div className="mode-text">
                <span className="mode-label">{s.label}</span>
                <span className="mode-desc">{s.desc}</span>
              </div>
            </label>
          ))}
        </div>
        {saving && <p className="saving-indicator">{t.saving_indicator}</p>}
      </section>
    </div>
  );
}
