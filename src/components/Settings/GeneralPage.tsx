import { useState, useEffect } from "react";
import { getConfig, setExecutionMode, setSubmitShortcut, setMaxAgentSteps, setDefaultTab } from "../../ipc/config";
import type { ExecutionMode, SubmitShortcut, DefaultTab } from "../../ipc/config";
import { useLocale } from "../../contexts/LocaleContext";
import type { Locale } from "../../lib/i18n";
import { THEMES, getActiveTheme, applyTheme, type ThemeId } from "../../lib/themes";
import "./GeneralPage.css";

const STEP_VALUES = [5, 10, 20, 50, 100, 0];
const DEFAULT_TAB_STORAGE_KEY = "aiterm_default_tab";
const FONT_SIZE_KEY = "aiterm-font-size";
const FONT_FAMILY_KEY = "aiterm-font-family";
const FONT_SIZE_OPTIONS = [8, 10, 11, 12, 13, 14, 16, 18, 20, 24];
const FONT_FAMILY_OPTIONS = [
  { label: "Cascadia Mono (default)", value: '"Cascadia Mono", Consolas, monospace' },
  { label: "JetBrains Mono", value: '"JetBrains Mono", monospace' },
  { label: "Fira Code", value: '"Fira Code", monospace' },
  { label: "Consolas", value: "Consolas, monospace" },
  { label: "monospace (system)", value: "monospace" },
];
const LOOP_TIMING_KEY = "loopTimingMode";

export function GeneralPage() {
  const { t, locale, setLocale } = useLocale();
  const [mode, setMode] = useState<ExecutionMode>("always-confirm");
  const [shortcut, setShortcut] = useState<SubmitShortcut>("enter");
  const [maxSteps, setMaxSteps] = useState<number>(5);
  const [defaultTab, setDefaultTabState] = useState<DefaultTab>("terminal");
  const [fontSize, setFontSizeState] = useState<number>(() =>
    parseInt(localStorage.getItem(FONT_SIZE_KEY) ?? "14", 10) || 14
  );
  const [fontFamily, setFontFamilyState] = useState<string>(
    () => localStorage.getItem(FONT_FAMILY_KEY) ?? FONT_FAMILY_OPTIONS[0].value
  );
  const [activeTheme, setActiveTheme] = useState<ThemeId>(() => getActiveTheme().id);
  const [telegramToken, setTelegramToken] = useState("");
  const [telegramChatId, setTelegramChatId] = useState("");
  const [saving, setSaving] = useState(false);
  const [policyControlled, setPolicyControlled] = useState<{ execution_mode?: boolean; max_agent_steps?: boolean }>({});
  const [timingMode, setTimingMode] = useState<"full" | "compact">(
    () => (localStorage.getItem(LOOP_TIMING_KEY) as "full" | "compact" | null) ?? "compact"
  );

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
      if (cfg.enterprise_policy) {
        setPolicyControlled({
          execution_mode: !!cfg.enterprise_policy.execution_mode,
          max_agent_steps: !!cfg.enterprise_policy.max_agent_steps,
        });
      }
    });
    import("../../ipc/telegram").then(({ getTelegramConfig }) => {
      getTelegramConfig().then(cfg => {
        if (cfg.bot_token) setTelegramToken(cfg.bot_token);
        if (cfg.chat_id) setTelegramChatId(cfg.chat_id);
      });
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

  const handleThemeChange = (id: ThemeId) => {
    setActiveTheme(id);
    const theme = THEMES.find((t) => t.id === id)!;
    applyTheme(theme);
  };

  const handleFontSizeChange = (size: number) => {
    setFontSizeState(size);
    localStorage.setItem(FONT_SIZE_KEY, String(size));
    window.dispatchEvent(new CustomEvent("aiterm:font-changed", { detail: { fontSize: size, fontFamily } }));
  };

  const handleFontFamilyChange = (family: string) => {
    setFontFamilyState(family);
    localStorage.setItem(FONT_FAMILY_KEY, family);
    window.dispatchEvent(new CustomEvent("aiterm:font-changed", { detail: { fontSize, fontFamily: family } }));
  };

  const handleTimingModeChange = (newMode: "full" | "compact") => {
    setTimingMode(newMode);
    localStorage.setItem(LOOP_TIMING_KEY, newMode);
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
        <h3 style={{ display: "flex", alignItems: "center", gap: 8 }}>
          {t.execution_mode}
          {policyControlled.execution_mode && (
            <span style={{ fontSize: 11, color: "#888", fontWeight: 400 }}>{t.managed_by_admin}</span>
          )}
        </h3>
        <p className="section-desc">{t.execution_mode_desc}</p>
        <div className="mode-list" style={{ opacity: policyControlled.execution_mode ? 0.5 : 1 }}>
          {MODES.map((m) => (
            <label key={m.value} className="mode-option">
              <input
                type="radio"
                name="execution_mode"
                value={m.value}
                checked={mode === m.value}
                onChange={() => handleChange(m.value)}
                disabled={saving || !!policyControlled.execution_mode}
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
        <h3 style={{ display: "flex", alignItems: "center", gap: 8 }}>
          {t.agent_max_steps}
          {policyControlled.max_agent_steps && (
            <span style={{ fontSize: 11, color: "#888", fontWeight: 400 }}>{t.managed_by_admin}</span>
          )}
        </h3>
        <p className="section-desc">{t.agent_max_steps_desc}</p>
        <div className="step-select-row" style={{ opacity: policyControlled.max_agent_steps ? 0.5 : 1 }}>
          <select
            className="step-select"
            value={maxSteps}
            onChange={(e) => handleMaxStepsChange(Number(e.target.value))}
            disabled={saving || !!policyControlled.max_agent_steps}
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
      </section>

      <section className="settings-section">
        <h3>{t.appearance}</h3>
        <div style={{ display: "flex", flexDirection: "column", gap: "20px" }}>
          <div>
            <label style={{ display: "block", marginBottom: "6px", fontSize: "13px", color: "#aaa" }}>
              {t.settings_theme}
            </label>
            <div style={{ display: "flex", flexWrap: "wrap", gap: "8px" }}>
              {THEMES.map((theme) => (
                <button
                  key={theme.id}
                  onClick={() => handleThemeChange(theme.id)}
                  style={{
                    padding: "6px 14px", borderRadius: 5, cursor: "pointer", fontSize: 12,
                    border: activeTheme === theme.id ? "1px solid #34d399" : "1px solid #333",
                    background: theme.css["--bg-tertiary"] ?? "#1a1a1a",
                    color: theme.css["--text-primary"] ?? "#e6e6e6",
                    fontWeight: activeTheme === theme.id ? 600 : 400,
                  }}
                >
                  {theme.label}
                </button>
              ))}
            </div>
          </div>
          <div>
            <label style={{ display: "block", marginBottom: "6px", fontSize: "13px", color: "#aaa" }}>
              {t.font_size}
            </label>
            <p className="section-desc" style={{ margin: "0 0 8px 0" }}>{t.font_size_desc}</p>
            <div style={{ display: "flex", flexWrap: "wrap", gap: "6px" }}>
              {FONT_SIZE_OPTIONS.map((s) => (
                <button
                  key={s}
                  onClick={() => handleFontSizeChange(s)}
                  style={{
                    padding: "4px 10px", borderRadius: 4, cursor: "pointer", fontSize: 13,
                    border: fontSize === s ? "1px solid #34d399" : "1px solid #333",
                    background: fontSize === s ? "#0f2e23" : "#1a1a1a",
                    color: fontSize === s ? "#34d399" : "#ccc",
                  }}
                >
                  {s}
                </button>
              ))}
            </div>
          </div>
          <div>
            <label style={{ display: "block", marginBottom: "6px", fontSize: "13px", color: "#aaa" }}>
              {t.font_family}
            </label>
            <p className="section-desc" style={{ margin: "0 0 8px 0" }}>{t.font_family_desc}</p>
            <select
              className="step-select"
              value={fontFamily}
              onChange={(e) => handleFontFamilyChange(e.target.value)}
            >
              {FONT_FAMILY_OPTIONS.map((f) => (
                <option key={f.value} value={f.value}>{f.label}</option>
              ))}
            </select>
          </div>
        </div>
      </section>

      <section className="settings-section">
        <h3>{t.telegram_integration_title}</h3>
        <p className="section-desc">{t.telegram_integration_desc}</p>

        <div style={{ marginTop: "12px", background: "#111", border: "1px solid #333", borderRadius: "6px", padding: "12px", fontSize: "13px", color: "#ccc" }}>
          <h4 style={{ margin: "0 0 8px 0", color: "#eee" }}>{t.telegram_steps_title}</h4>
          <ol style={{ margin: 0, paddingLeft: "20px", display: "flex", flexDirection: "column", gap: "6px" }}>
            <li>{t.telegram_step_1}</li>
            <li>{t.telegram_step_2}</li>
            <li>{t.telegram_step_3}</li>
            <li>{t.telegram_step_4}</li>
          </ol>
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: "12px", marginTop: "16px" }}>
          <div>
            <label style={{ display: "block", marginBottom: "4px", fontSize: "13px", color: "#aaa" }}>{t.telegram_bot_token}</label>
            <input 
              type="password" 
              className="settings-input"
              style={{ width: "100%", padding: "8px", borderRadius: "4px", border: "1px solid #333", background: "#1e1e1e", color: "#eee" }}
              placeholder={t.telegram_bot_token_placeholder}
              value={telegramToken}
              onChange={async (e) => {
                const val = e.target.value;
                setTelegramToken(val);
                setSaving(true);
                try {
                  const { getTelegramConfig, setTelegramConfig } = await import("../../ipc/telegram");
                  const cfg = await getTelegramConfig();
                  await setTelegramConfig({ ...cfg, bot_token: val || null });
                } finally { setSaving(false); }
              }}
            />
          </div>
          <div>
            <label style={{ display: "block", marginBottom: "4px", fontSize: "13px", color: "#aaa" }}>{t.telegram_chat_id}</label>
            <input 
              type="text" 
              className="settings-input"
              style={{ width: "100%", padding: "8px", borderRadius: "4px", border: "1px solid #333", background: "#1e1e1e", color: "#eee" }}
              placeholder={t.telegram_chat_id_placeholder}
              value={telegramChatId}
              onChange={async (e) => {
                const val = e.target.value;
                setTelegramChatId(val);
                setSaving(true);
                try {
                  const { getTelegramConfig, setTelegramConfig } = await import("../../ipc/telegram");
                  const cfg = await getTelegramConfig();
                  await setTelegramConfig({ ...cfg, chat_id: val || null });
                } finally { setSaving(false); }
              }}
            />
          </div>
        </div>
        {saving && <p className="saving-indicator">{t.saving_indicator}</p>}
      </section>

      <section className="settings-section">
        <h3>{t.loop_timing_label}</h3>
        <p className="section-desc">{t.loop_timing_desc}</p>
        <div className="mode-list">
          {([
            { value: "compact" as const, label: t.loop_timing_compact_label, desc: t.loop_timing_compact_desc },
            { value: "full"    as const, label: t.loop_timing_full_label, desc: t.loop_timing_full_desc },
          ]).map((opt) => (
            <label key={opt.value} className="mode-option">
              <input
                type="radio"
                name="loopTimingMode"
                value={opt.value}
                checked={timingMode === opt.value}
                onChange={() => handleTimingModeChange(opt.value)}
              />
              <div className="mode-text">
                <span className="mode-label">{opt.label}</span>
                <span className="mode-desc">{opt.desc}</span>
              </div>
            </label>
          ))}
        </div>
      </section>
    </div>
  );
}
