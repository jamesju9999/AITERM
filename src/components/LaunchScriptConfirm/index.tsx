import { useEffect, useRef } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import "./index.css";

// 對話框出現後這段時間內不接受「執行」。多個腳本排隊時，下一個對話框會在同一個
// 位置、同樣的版面重新掛載，雙擊的第二下（約 100ms 後）會落在新對話框的「執行」上，
// 等於核准了一個使用者根本沒讀過的腳本；對話框出現在一個已經按下去的點擊底下也是
// 同一個問題。預設焦點放在「跳過」只保護得了鍵盤。
// 600ms：Windows／macOS 預設的雙擊間隔是 500ms，再留一點餘裕給「第一下點擊」到
// 「新對話框 commit」之間的幾毫秒。
const RUN_SHIELD_MS = 600;

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
  // 掛載時間記在 effect 裡而不是 render 期間（performance.now() 不是純的）。用
  // performance.now() 而不是 Date.now()：前者是單調時鐘，系統時間被往回撥（NTP、VM
  // 休眠還原）時，「執行」才不會被鎖到系統時間追回來為止。effect 還沒跑之前是 null，
  // 一律當作還在保護期內（fail closed）。只擋「執行」：跳過與 Esc 是安全的那一邊，
  // 不設限。
  const shownAtRef = useRef<number | null>(null);
  useEffect(() => {
    shownAtRef.current = performance.now();
  }, []);
  const handleRun = () => {
    const shownAt = shownAtRef.current;
    if (shownAt === null || performance.now() - shownAt < RUN_SHIELD_MS) return;
    onRun();
  };
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
            onClick={handleRun}
          >
            {t.launch_script_run}
          </button>
        </div>
      </div>
    </div>
  );
}
