import type { TierMapping } from "../../ipc/bridge";

export interface BridgeProfile {
  id: string;
  name: string;
  opus: TierMapping | null;
  sonnet: TierMapping | null;
  haiku: TierMapping | null;
}

const STORAGE_KEY = "aiterm.bridgeProfiles";

/** localStorage 壞掉、內容損毀或被瀏覽器擋下時一律視為「沒有存過」，不讓設定頁掛掉。 */
export function loadBridgeProfiles(): BridgeProfile[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed) ? (parsed as BridgeProfile[]) : [];
  } catch {
    return [];
  }
}

/** 寫入失敗（例如私密模式關閉 storage）不拋出——頂多這次操作不會持久化。 */
export function saveBridgeProfiles(profiles: BridgeProfile[]): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(profiles));
  } catch {
    // 忽略：呼叫端已經更新了記憶體中的 state，使用者這次 session 還是看得到。
  }
}

type TierTriple = Pick<BridgeProfile, "opus" | "sonnet" | "haiku">;

function tierEqual(a: TierMapping | null, b: TierMapping | null): boolean {
  if (a === null || b === null) return a === b;
  return a.provider_id === b.provider_id && a.model === b.model;
}

/** 「使用中」徽章的判斷依據：目前表格的三個 tier 是否跟某個 profile 逐一相符。 */
export function tiersEqual(a: TierTriple, b: TierTriple): boolean {
  return tierEqual(a.opus, b.opus) && tierEqual(a.sonnet, b.sonnet) && tierEqual(a.haiku, b.haiku);
}
