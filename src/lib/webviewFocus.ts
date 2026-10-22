import { getCurrentWebview } from "@tauri-apps/api/webview";

/**
 * 原生對話框（確認、選資料夾）是蓋在視窗上的系統視窗；關掉之後 macOS 的網頁檢視可能沒有拿回鍵盤焦點：
 * 文字框看起來有焦點（外框、游標都在），按鍵卻送不進去。要明確把焦點還給網頁檢視。
 * 不在 Tauri 裡（測試）或沒有權限時什麼都不做，不丟錯。
 */
export async function restoreWebviewFocus(): Promise<void> {
  try {
    await getCurrentWebview().setFocus();
  } catch {
    // 不在 Tauri 裡，或這個平台不需要。
  }
}
