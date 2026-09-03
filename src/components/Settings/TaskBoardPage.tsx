import { useCallback, useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { getTaskBoardConfig, setTaskBoardConfig, type TaskBoardConfig } from "../../ipc/tasks";

export function TaskBoardPage() {
  const { t } = useLocale();
  const [cfg, setCfg] = useState<TaskBoardConfig | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    void getTaskBoardConfig().then(setCfg);
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
      <p>{t.board_settings_desc}</p>

      <label>
        {t.board_settings_max_concurrent}
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
      </label>

      <label>
        {t.board_settings_claude_command}
        <input
          value={cfg.claude_command}
          onChange={(e) => {
            setSaved(false);
            setCfg({ ...cfg, claude_command: e.target.value });
          }}
        />
      </label>

      <div>
        <button onClick={() => void save()} disabled={saving}>
          {saved ? `${t.board_settings_saved} ✓` : t.board_save}
        </button>
      </div>
    </div>
  );
}
