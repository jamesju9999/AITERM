import { useLocale } from "../../contexts/LocaleContext";
import "./index.css";

interface Props {
  scriptPath: string;
  onRun: () => void;
  onSkip: () => void;
}

/**
 * 雙擊 `.command`／`.sh` 等於執行任意程式，所以執行前要使用者確認，並且
 * 顯示完整路徑。**初始焦點刻意放在「只開啟資料夾」而不是「執行」**：啟動請求
 * 可能在使用者正於別的分頁打字時到達，焦點會跳進來，下一個 Space／Enter 若落在
 * 「執行」上就等於替使用者核准了任意腳本（`TerminalView` 在焦點是 BUTTON 時
 * 不會把按鍵導回終端機）。Escape 等同跳過。
 * **必須掛在 `TerminalApp` 層**：非作用中的分頁是
 * `visibility: hidden` + `pointer-events: none`，掛在分頁裡會點不到。
 * 不用 `window.confirm`（Tauri 內有已知問題）。
 */
export function LaunchScriptConfirm({ scriptPath, onRun, onSkip }: Props) {
  const { t } = useLocale();
  return (
    <div className="aiterm-launch-confirm__backdrop">
      <div
        className="aiterm-launch-confirm"
        role="dialog"
        aria-modal="true"
        onKeyDown={(e) => {
          if (e.key === "Escape") onSkip();
        }}
      >
        <div className="aiterm-launch-confirm__title">{t.launch_script_title}</div>
        <div className="aiterm-launch-confirm__body">{t.launch_script_body(scriptPath)}</div>
        <div className="aiterm-launch-confirm__actions">
          <button
            className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
            data-testid="launch-script-skip"
            autoFocus
            onClick={onSkip}
          >
            {t.launch_script_skip}
          </button>
          <button
            className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
            data-testid="launch-script-run"
            onClick={onRun}
          >
            {t.launch_script_run}
          </button>
        </div>
      </div>
    </div>
  );
}
