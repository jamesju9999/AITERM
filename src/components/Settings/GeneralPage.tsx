import { useState, useEffect } from "react";
import { getConfig, setExecutionMode } from "../../ipc/config";
import type { ExecutionMode } from "../../ipc/config";
import "./GeneralPage.css";

const MODES: { value: ExecutionMode; label: string; desc: string }[] = [
  {
    value: "always-confirm",
    label: "一律確認",
    desc: "所有 AI 產出的命令都顯示預覽框，需明確確認才執行（預設）。",
  },
  {
    value: "graded",
    label: "分級自動",
    desc: "Safe 命令自動執行；NeedsConfirm / Dangerous 仍顯示確認框。",
  },
  {
    value: "full-auto",
    label: "全自動 Agent",
    desc: "Safe 與 NeedsConfirm 自動執行；Dangerous 仍強制確認。",
  },
];

export function GeneralPage() {
  const [mode, setMode] = useState<ExecutionMode>("always-confirm");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    getConfig().then((cfg) => setMode(cfg.execution_mode));
  }, []);

  const handleChange = async (newMode: ExecutionMode) => {
    setMode(newMode);
    setSaving(true);
    try {
      await setExecutionMode(newMode);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="general-page">
      <h2>一般設定</h2>

      <section className="settings-section">
        <h3>執行模式</h3>
        <p className="section-desc">決定 AI 產出的命令如何被執行。</p>
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
        {saving && <p className="saving-indicator">儲存中…</p>}
      </section>
    </div>
  );
}
