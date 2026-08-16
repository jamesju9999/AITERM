import type { TabType } from "../TabBar";
import { VISIBLE_TAB_TYPES } from "../NewTabPicker/tabCatalog";

/** 首頁自然語言輸入框的路由結果。純函式的輸出，不含任何 IPC/UI 邏輯。 */
export interface RouteResult {
  type: TabType;
  /** 只有 terminal 會用到：把使用者原句當成 agent mission 的目標。 */
  mission?: string;
  /** 使用者原本打的那句話。換一種分頁類型時要用同一句話重開。 */
  userText: string;
  /** true 代表這是降級結果，不是 AI 判斷出來的。UI 據此決定要不要顯示提示。 */
  fallback: boolean;
}

/** 降級路徑：AI 沒設定、網路失敗、超時、回應無法解析，一律退回這裡——
 *  開終端機分頁，並把使用者整句話當成 agent 任務目標。輸入框必須永遠有
 *  反應，不能因為 AI 掛掉就卡住。 */
export function fallbackRoute(userText: string): RouteResult {
  return { type: "terminal", mission: userText, userText, fallback: true };
}

/** 從可能夾雜說明文字或包在 markdown 圍籬裡的回應中，撈出第一個 JSON 物件。
 *  取「第一個 `{` 到最後一個 `}`」——夠用是因為路由提示詞只要求 AI 回一個
 *  單一 JSON 物件，不會有巢狀的第二個物件跟在後面。撈不到或解析失敗回 null，
 *  呼叫端一律走降級，不對呼叫端丟例外。 */
function extractJsonObject(raw: string): unknown {
  const start = raw.indexOf("{");
  const end = raw.lastIndexOf("}");
  if (start === -1 || end === -1 || end < start) return null;
  try {
    return JSON.parse(raw.slice(start, end + 1));
  } catch {
    // 模型輸出被截斷、圍籬裡夾了非 JSON 文字等都會讓 JSON.parse 丟例外——
    // 這是預期會發生的情況，不是程式錯誤，降級處理即可，不能讓例外往上炸。
    return null;
  }
}

/** 解析 AI 路由回應，並套用防呆。任何一步不符合預期都降級（fallbackRoute），
 *  絕不丟例外、絕不放行清單外的分頁類型。 */
export function parseRouteReply(raw: string, userText: string): RouteResult {
  const parsed = extractJsonObject(raw);

  // 不是物件（null、陣列、基本型別）就沒有 .type 可讀，降級。
  if (typeof parsed !== "object" || parsed === null) {
    return fallbackRoute(userText);
  }

  const type = (parsed as Record<string, unknown>).type;

  // AI 沒回 type，或 type 不是字串——回應形狀不對，不能亂猜，降級。
  if (typeof type !== "string") {
    return fallbackRoute(userText);
  }

  // 核心防呆：AI 只能回既有分頁類型裡「可見」的一種。清單外的值（幻覺出來的
  // 類型）以及 hidden 的類型（mail、api-docs——後端完整但入口刻意收起來）
  // 都不可以照單全收去開分頁，否則 AI 路由會變成已下架功能的後門。
  if (!VISIBLE_TAB_TYPES.includes(type as TabType)) {
    return fallbackRoute(userText);
  }

  const result: RouteResult = { type: type as TabType, userText, fallback: false };
  // 只有 terminal 需要把使用者原句當成 agent mission 的目標；其他分頁類型
  // 沒有「任務目標」這個概念。
  if (type === "terminal") {
    result.mission = userText;
  }
  return result;
}
