/**
 * 模型常見的失誤：回「讓我先取得 UI 快照。」就結束這一輪，卻沒真的呼叫工具。
 * 工具迴圈只在「這一輪有工具呼叫」時繼續，所以任務就停在半路。
 *
 * 這裡判斷一則純文字回覆是不是「宣告了要做、卻沒做」。寧可漏判也不要誤判：
 * 誤判會多催一次模型，漏判只是維持現狀（使用者自己再說一次）。所以條件很窄——
 * 短、以宣告動作的句型開頭或在句中出現、結尾不是問句。
 */

const MAX_LENGTH = 200;

// 「讓我／我來／我先／我會／我將／接下來」與英文的 let me／I'll／I will／I am going to。
const INTENT = /(讓我(?!知道)|我來|我先|我會|我將|接下來|let me(?! know)|i'll |i will |i am going to |i'm going to )/i;

export function looksLikeUnfulfilledIntent(text: string): boolean {
  const t = text.trim();
  if (!t || t.length > MAX_LENGTH) return false;
  if (/[?？]$/.test(t)) return false; // 在問使用者，不是在宣告
  return INTENT.test(t);
}

/** 催促模型直接動手的訊息（只送給模型，不顯示在對話裡）。 */
export function intentNudge(locale: string): string {
  return locale === "zh-TW"
    ? "請直接呼叫工具來執行，不要只描述你打算做什麼。任務完成或需要我提供資訊時，才用文字回覆。"
    : "Call the tool directly instead of describing what you plan to do. Reply in plain text only when the task is complete or you need information from me.";
}
