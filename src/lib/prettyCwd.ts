import { homeDir } from "@tauri-apps/api/path";

/** `\` → `/`，並去掉結尾的 `/`（根目錄 `/` 保留）。 */
function norm(p: string): string {
  const s = p.replace(/\\/g, "/");
  return s.length > 1 ? s.replace(/\/+$/, "") : s;
}

/**
 * 路徑落在家目錄底下（或就是家目錄）時，回傳家目錄之後的部分（"" 表示家目錄本身）；
 * 否則回傳 null。比對要在路徑邊界上（/Users/jamesju2 不算 /Users/jamesju 底下），
 * 且 Windows 的磁碟機代號開頭的路徑不分大小寫（C:\Users 與 c:\users 是同一處）。
 */
function underHome(cwd: string, home: string): string | null {
  const ci = /^[a-zA-Z]:/.test(home);
  const a = ci ? cwd.toLowerCase() : cwd;
  const h = ci ? home.toLowerCase() : home;
  if (a === h) return "";
  if (a.startsWith(h + "/")) return cwd.slice(home.length);
  return null;
}

/** 完整路徑，家目錄換成 `~`；三個平台一致（Windows 的 `\` 也轉成 `/`）。 */
export function tildePath(cwd: string, home: string | null | undefined): string {
  const c = norm(cwd);
  if (!home) return c;
  const rest = underHome(c, norm(home));
  return rest === null ? c : `~${rest}`;
}

/** 指令區塊標頭用：家目錄底下顯示 `~/…`，其餘路徑只留最後兩段並以 `.../` 開頭。 */
export function blockCwdLabel(cwd: string | undefined, home: string | null | undefined): string {
  if (!cwd) return "";
  const c = norm(cwd);
  const h = home ? norm(home) : null;
  if (h && underHome(c, h) !== null) return tildePath(cwd, home);
  const parts = c.split("/");
  return parts.length > 2 ? `.../${parts.slice(-2).join("/")}` : c;
}

let cached: string | null = null;
let pending: Promise<string | null> | null = null;

/** 家目錄（正規化後）；整個 App 只問一次。取不到時回 null，呼叫端照原樣顯示。 */
export function getHomeDir(): Promise<string | null> {
  if (cached !== null) return Promise.resolve(cached);
  pending ??= homeDir()
    .then((h) => { cached = norm(h); return cached; })
    .catch(() => null)
    .finally(() => { pending = null; });
  return pending;
}

/** 測試用：清掉快取。 */
export function _resetHomeDirCache(): void { cached = null; pending = null; }
