import { homeDir } from "@tauri-apps/api/path";

/** 快取抓到的家目錄路徑（正規化為 "/" 分隔、去掉結尾斜線）。
 *
 *  首頁的「最近的專案目錄」要把父路徑縮寫成 "~/..."，但 `homeDir()` 是
 *  非同步的，而首頁每次顯示都會重新掛載——若在元件內用 useEffect 等它回來，
 *  會讓區塊變成「先顯示完整路徑、下一刻跳成縮寫」的閃爍，且讓元件多一層
 *  非同步狀態。改用模組層級快取：第一次呼叫時背景抓一次，之後任何一次
 *  掛載（包含首頁重新掛載）都能同步拿到已快取的值；拿不到就原樣顯示完整
 *  路徑，不是致命錯誤。抓取寫法照抄 TerminalView 既有的
 *  `homeDir().then().catch()`。 */
let cached: string | null = null;
let started = false;

export function getCachedHomeDir(): string | null {
  if (!started) {
    started = true;
    try {
      homeDir()
        .then((h) => { cached = h.replace(/\\/g, "/").replace(/\/$/, ""); })
        .catch(() => {});
    } catch {
      // 非 Tauri 環境（例如測試）沒有 IPC 橋接可用，放棄快取即可。
    }
  }
  return cached;
}

/** 把路徑開頭的家目錄換成 "~"。拿不到家目錄，或路徑不在家目錄底下時原樣返回。 */
export function abbreviateHome(path: string): string {
  const home = getCachedHomeDir();
  const normalized = path.replace(/\\/g, "/");
  if (home && (normalized === home || normalized.startsWith(home + "/"))) {
    return "~" + normalized.slice(home.length);
  }
  return normalized;
}
