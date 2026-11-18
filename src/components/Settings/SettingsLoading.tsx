import { useEffect, useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";

/** 超過這個時間還沒讀完，就補一句「可能在等系統授權」。 */
const SLOW_MS = 1500;

/**
 * 設定頁在讀取資料（供應商金鑰、系統權限等）時顯示，避免使用者面對一片空白。
 * 讀取可能卡在 macOS 的鑰匙圈／權限對話框上，所以拖久了會多提示一句。
 */
export function SettingsLoading() {
  const { t } = useLocale();
  const [slow, setSlow] = useState(false);
  useEffect(() => {
    const id = setTimeout(() => setSlow(true), SLOW_MS);
    return () => clearTimeout(id);
  }, []);
  return (
    <div role="status" aria-live="polite" style={{ padding: "24px 0", color: "#888", fontSize: 13 }}>
      <div>{t.settings_loading}</div>
      {slow && <div style={{ marginTop: 6, fontSize: 12 }}>{t.settings_loading_slow_hint}</div>}
    </div>
  );
}
