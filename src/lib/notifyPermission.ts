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
 *
 * 永不 reject；失敗一律 resolve false。呼叫端（例如 useMailSync.ts 裡
 * `listen` 的 async callback）是 `if (await ensureNotificationPermission())`，
 * 沒有包 try/catch——把 catch 改成 rethrow 會在事件 callback 裡變成一個
 * 沒有測試會抓到的 unhandled rejection。
 *
 * 這不是 app 裡唯一取得通知權限的入口。
 * `src/components/Settings/MailAccountsPage.tsx` 的
 * `promptForNotificationPermission` 刻意繞過這個模組，直接呼叫
 * isPermissionGranted() / requestPermission()：它要在使用者剛新增完
 * mail 帳號、確定正在看著 app 的那一刻主動跳出授權對話框，如果改走這裡的
 * 記憶化，一旦 `pending` 已經是 true，就會靜默地永遠不再跳出提示。因此
 * `pending` 為真並不代表「這個 session 已經問過使用者」——它只代表「這個
 * 模組問過」。
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
 *
 * 更危險的方向不是變紅，是變成假陽性：如果先跑的 test 把 `granted = true`
 * 快取下來，後面任何斷言「有發出通知」的 test 都會通過——即使權限處理邏輯
 * 本身已經壞掉也一樣。任何會觸發通知的測試檔都必須在 beforeEach 呼叫這個
 * reset，否則它的斷言可能是因為錯的理由才通過。
 */
export function resetNotificationPermissionForTests(): void {
  pending = null;
}
