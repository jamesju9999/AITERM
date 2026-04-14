import { useState, useEffect } from "react";
import { getConfig, setExecutionMode, setSubmitShortcut } from "../../ipc/config";
import type { ExecutionMode, SubmitShortcut } from "../../ipc/config";
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

const SHORTCUT_MODES: { value: SubmitShortcut; label: string; desc: string }[] = [
  { value: "enter", label: "Enter", desc: "按下 Enter 鍵送出指令，Shift+Enter 換行。" },
  { value: "shift-enter", label: "Shift + Enter", desc: "按下 Shift+Enter 送出指令，直接按 Enter 換行。" },
  { value: "ctrl-enter", label: "Ctrl + Enter", desc: "按下 Ctrl+Enter 送出指令，直接按 Enter 換行。" },
];

export function GeneralPage() {
  const [mode, setMode] = useState<ExecutionMode>("always-confirm");
  const [shortcut, setShortcut] = useState<SubmitShortcut>("enter");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    getConfig().then((cfg) => {
      setMode(cfg.execution_mode);
      setShortcut(cfg.submit_shortcut);
    });
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

  const handleShortcutChange = async (newShortcut: SubmitShortcut) => {
    setShortcut(newShortcut);
    setSaving(true);
    try {
      await setSubmitShortcut(newShortcut);
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
      </section>

      <section className="settings-section">
        <h3>輸入組合鍵</h3>
        <p className="section-desc">決定底部輸入框以哪個組合鍵送出指令。</p>
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
        {saving && <p className="saving-indicator">儲存中…</p>}
      </section>
    </div>
  );
}
