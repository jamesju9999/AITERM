import { useLocale } from "../../contexts/LocaleContext";
import "./index.css";

interface Props {
  scriptPath: string;
  onRun: () => void;
  onSkip: () => void;
}

/**
 * 雙擊 `.command`／`.sh` 等於執行任意程式，所以執行前要使用者確認，並且
 * 顯示完整路徑。**必須掛在 `TerminalApp` 層**：非作用中的分頁是
 * `visibility: hidden` + `pointer-events: none`，掛在分頁裡會點不到。
 * 不用 `window.confirm`（Tauri 內有已知問題）。
 */
export function LaunchScriptConfirm({ scriptPath, onRun, onSkip }: Props) {
  const { t } = useLocale();
  return (
    <div className="aiterm-launch-confirm__backdrop">
      <div className="aiterm-launch-confirm" role="dialog" aria-modal="true">
        <div className="aiterm-launch-confirm__title">{t.launch_script_title}</div>
        <div className="aiterm-launch-confirm__body">{t.launch_script_body(scriptPath)}</div>
        <div className="aiterm-launch-confirm__actions">
          <button
            className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
            data-testid="launch-script-skip"
            onClick={onSkip}
          >
            {t.launch_script_skip}
          </button>
          <button
            className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
            data-testid="launch-script-run"
            autoFocus
            onClick={onRun}
          >
            {t.launch_script_run}
          </button>
        </div>
      </div>
    </div>
  );
}
