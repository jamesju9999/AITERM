import { isPermissionGranted, requestPermission } from "@tauri-apps/plugin-notification";

// 模組層級的單一 promise，跨所有呼叫端共用。每個通知來源各自記憶化的話，
// 一次啟動就會疊出多個並行的 requestPermission() 提示，並且重複打擾
// 已經拒絕過的使用者——這正是這個記憶化存在的理由。
let pending: Promise<boolean> | null = null;

/**
 * 取得通知權限，整個 app 生命週期內只實際請求一次。
 *
 * 桌面端注意事項（tauri-plugin-notification 2.3.3，desktop.rs:61-66）：
 * permission_state() 與 request_permission() 在桌面端一律回傳 Granted，
 * 完全不會去問 OS。這段保留是因為它是外掛的正式 API 且在行動端是必要的，
 * 但在 macOS / Windows / Linux 上它永遠 resolve true。
 */
export function ensureNotificationPermission(): Promise<boolean> {
  pending ??= isPermissionGranted()
    .then((granted) => granted || requestPermission().then((p) => p === "granted"))
    .catch((err) => {
      console.error("[notify] notification permission check failed:", err);
      // 不要把一次暫時性的 IPC 失敗記憶成「被拒絕」。
      pending = null;
      return false;
    });
  return pending;
}

/**
 * 只給測試用：清掉快取的權限結果。
 *
 * 這個快取是模組層級的，所以在同一個測試檔內會跨 test 存活——一個先跑的
 * test 快取了「已授權」，後面驗證「被拒絕」路徑的 test 就永遠不會再走到
 * requestPermission()。每個 test 都必須從乾淨狀態開始。
 */
export function resetNotificationPermissionForTests(): void {
  pending = null;
}
