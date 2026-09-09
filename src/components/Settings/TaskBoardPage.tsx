import { useCallback, useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { getTaskBoardConfig, setTaskBoardConfig, type TaskBoardConfig } from "../../ipc/tasks";
import { getTelegramConfig } from "../../ipc/telegram";
import "./TaskBoardPage.css";

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

  return (
    <div className="task-board-page">
      <h2>{t.board_settings_title}</h2>
      <p className="task-board-desc">{t.board_settings_desc}</p>

      <section className="task-board-section">
        <label className="task-board-field">
          <span>{t.board_settings_max_concurrent}</span>
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
          <span>{t.board_settings_claude_command}</span>
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

        <label className="task-board-field">
          <span>{t.board_settings_stuck_timeout}</span>
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
          <span className="task-board-hint">{t.board_settings_stuck_timeout_hint}</span>
        </label>

        <label className="task-board-field task-board-field--checkbox">
          <input
            type="checkbox"
            className="task-board-checkbox"
            checked={cfg.auto_close_finished_tabs}
            onChange={(e) => {
              setSaved(false);
              setCfg({ ...cfg, auto_close_finished_tabs: e.target.checked });
            }}
          />
          <span>{t.board_settings_auto_close}</span>
          <span className="task-board-hint">{t.board_settings_auto_close_hint}</span>
        </label>

        <label className="task-board-field task-board-field--checkbox">
          <input
            type="checkbox"
            className="task-board-checkbox"
            checked={cfg.notify_desktop_on_finish}
            onChange={(e) => {
              setSaved(false);
              setCfg({ ...cfg, notify_desktop_on_finish: e.target.checked });
            }}
          />
          <span>{t.board_settings_notify_desktop}</span>
          <span className="task-board-hint">{t.board_settings_notify_desktop_hint}</span>
        </label>

        {telegramConfigured && (
          <label className="task-board-field task-board-field--checkbox">
            <input
              type="checkbox"
              className="task-board-checkbox"
              checked={cfg.notify_telegram_on_finish}
              onChange={(e) => {
                setSaved(false);
                setCfg({ ...cfg, notify_telegram_on_finish: e.target.checked });
              }}
            />
            <span>{t.board_settings_notify_telegram}</span>
            <span className="task-board-hint">{t.board_settings_notify_telegram_hint}</span>
          </label>
        )}
      </section>

      <div className="task-board-actions">
        <button onClick={() => void save()} disabled={saving}>
          {saved ? `${t.board_settings_saved} ✓` : t.board_save}
        </button>
      </div>
    </div>
  );
}
