import { useCallback, useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { codexEnvStatus, getTaskBoardConfig, setCodexEnv, setTaskBoardConfig, type TaskBoardConfig } from "../../ipc/tasks";
import { getTelegramConfig } from "../../ipc/telegram";
import "./TaskBoardPage.css";

type ToggleKey =
  | "isolate_with_worktree"
  | "lean_agent_dispatch"
  | "auto_close_finished_tabs"
  | "notify_desktop_on_finish"
  | "notify_telegram_on_finish";

export function TaskBoardPage() {
  const { t } = useLocale();
  const [cfg, setCfg] = useState<TaskBoardConfig | null>(null);
  const [telegramConfigured, setTelegramConfigured] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    void getTaskBoardConfig().then(setCfg);
    void getTelegramConfig().then((tc) =>
      setTelegramConfigured(Boolean(tc.bot_token) && Boolean(tc.chat_id)),
    );
  }, []);

  const save = useCallback(async () => {
    if (!cfg) return;
    setSaving(true);
    try {
      const max = Number.isNaN(cfg.max_concurrent) ? 1 : cfg.max_concurrent;
      await setTaskBoardConfig({ ...cfg, max_concurrent: max });
      setSaved(true);
    } finally {
      setSaving(false);
    }
  }, [cfg]);

  if (!cfg) return <div className="task-board-page" />;

  const toggle = (key: ToggleKey, label: string, hint: string, checked = Boolean(cfg[key])) => (
    <label className="task-board-toggle">
      <input
        type="checkbox"
        className="task-board-checkbox"
        checked={checked}
        onChange={(e) => {
          setSaved(false);
          setCfg({ ...cfg, [key]: e.target.checked });
        }}
      />
      <span className="task-board-toggle-text">
        <span className="task-board-toggle-label">{label}</span>
        <span className="task-board-hint">{hint}</span>
      </span>
    </label>
  );

  return (
    <div className="task-board-page">
      <h2>{t.board_settings_title}</h2>
      <p className="task-board-desc">{t.board_settings_desc}</p>

      <section className="task-board-section">
        <h3>{t.board_settings_group_dispatch}</h3>
        <div className="task-board-grid">
          <label className="task-board-field">
            <span className="task-board-label">{t.board_settings_max_concurrent}</span>
            <input
              type="number"
              min={1}
              max={16}
              value={Number.isNaN(cfg.max_concurrent) ? "" : cfg.max_concurrent}
              onChange={(e) => {
                setSaved(false);
                setCfg({ ...cfg, max_concurrent: e.target.valueAsNumber });
              }}
            />
            <span className="task-board-hint">{t.board_settings_max_concurrent_hint}</span>
          </label>

          <label className="task-board-field">
            <span className="task-board-label">{t.board_settings_stuck_timeout}</span>
            <span className="task-board-unit">
              <input
                type="number"
                min={1}
                max={360}
                value={Number.isNaN(cfg.stuck_timeout_secs) ? "" : Math.round(cfg.stuck_timeout_secs / 60)}
                onChange={(e) => {
                  setSaved(false);
                  setCfg({ ...cfg, stuck_timeout_secs: Math.round(e.target.valueAsNumber * 60) });
                }}
              />
              {t.board_settings_minutes}
            </span>
            <span className="task-board-hint">{t.board_settings_stuck_timeout_hint}</span>
          </label>
        </div>

        <label className="task-board-field">
          <span className="task-board-label">{t.board_settings_claude_command}</span>
          <input
            type="text"
            value={cfg.claude_command}
            onChange={(e) => {
              setSaved(false);
              setCfg({ ...cfg, claude_command: e.target.value });
            }}
          />
          <span className="task-board-hint">{t.board_settings_claude_command_hint}</span>
        </label>
      </section>

      <section className="task-board-section">
        <h3>{t.board_settings_group_env}</h3>
        <div className="task-board-toggles">
          {toggle("isolate_with_worktree", t.board_settings_isolate, t.board_settings_isolate_hint)}
          {toggle("lean_agent_dispatch", t.board_settings_lean, t.board_settings_lean_hint, cfg.lean_agent_dispatch ?? true)}
        </div>
        <CodexEnvField />
        {/* 關掉隔離之後，多張卡片會在同一個工作目錄互相覆蓋——Agent 會自己
            管 git，不代表兩個 Agent 同時改同一份檔案不會打架。只提醒，不代
            使用者改 max_concurrent。 */}
        {!cfg.isolate_with_worktree && cfg.max_concurrent > 1 && (
          <p className="task-board-warning">{t.board_settings_isolate_concurrent_warning}</p>
        )}
      </section>

      <section className="task-board-section">
        <h3>{t.board_settings_group_finish}</h3>
        <div className="task-board-toggles">
          {toggle("auto_close_finished_tabs", t.board_settings_auto_close, t.board_settings_auto_close_hint)}
          {toggle("notify_desktop_on_finish", t.board_settings_notify_desktop, t.board_settings_notify_desktop_hint)}
          {telegramConfigured &&
            toggle("notify_telegram_on_finish", t.board_settings_notify_telegram, t.board_settings_notify_telegram_hint)}
        </div>
      </section>

      <div className="task-board-actions">
        <button className="aiterm-btn aiterm-btn--primary" onClick={() => void save()} disabled={saving}>
          {saved ? `${t.board_settings_saved} ✓` : t.board_save}
        </button>
      </div>
    </div>
  );
}

/**
 * 派給 Codex 的 Agent 卡片要帶的環境變數（例如本機 oMLX 的 OMLX_API_KEY）。存在系統鑰匙圈，
 * 內容不回顯；有自己的儲存鈕，跟上面的設定分開存。
 */
function CodexEnvField() {
  const { t } = useLocale();
  const [isSet, setIsSet] = useState(false);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  useEffect(() => {
    void codexEnvStatus().then(setIsSet).catch(() => setIsSet(false));
  }, []);

  const save = async (value: string) => {
    setBusy(true);
    setMsg(null);
    try {
      await setCodexEnv(value);
      setIsSet(!!value.trim());
      setText("");
      setMsg(value.trim() ? t.board_settings_codex_env_saved : t.board_settings_codex_env_cleared);
    } catch (e) {
      setMsg(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="task-board-field task-board-codex-env">
      <span className="task-board-label">{t.board_settings_codex_env}</span>
      <textarea
        aria-label={t.board_settings_codex_env}
        rows={3}
        spellCheck={false}
        value={text}
        placeholder={isSet ? t.board_settings_codex_env_set_placeholder : "OMLX_API_KEY=..."}
        onChange={(e) => setText(e.target.value)}
      />
      <span className="task-board-codex-env-row">
        <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" disabled={busy || !text.trim()} onClick={() => void save(text)}>
          {t.board_settings_codex_env_save}
        </button>
        {isSet && (
          <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" disabled={busy} onClick={() => void save("")}>
            {t.board_settings_codex_env_clear}
          </button>
        )}
        {msg && <span className="task-board-hint">{msg}</span>}
      </span>
      <span className="task-board-hint">{t.board_settings_codex_env_hint}</span>
    </div>
  );
}
