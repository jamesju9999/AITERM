/**
 * 使用者用中文／日文等輸入法選字時，按 Enter 只是要「確認候選字」，但瀏覽器
 * 一樣會送出一個 keydown，且 e.key === "Enter"。吃純 Enter 送出的文字輸入若
 * 沒有分辨組字階段，就會把確認候選字誤當成送出指令。
 *
 * 所有「純 Enter 觸發送出／選取」的處理器都應在開頭呼叫這個函式擋掉組字中的事件。
 * 需要 Ctrl/Cmd/Shift 修飾鍵才送出的處理器不受影響（IME 確認是純 Enter）。
 *
 * isComposing 是標準屬性；keyCode === 229 是舊版 WebKit（含部分 WKWebView 狀態）
 * 在組字期間的既有行為，一起判斷比較保險。
 *
 * 同時接受 React 合成事件（透過 nativeEvent）與原生 KeyboardEvent（window 層監聽）。
 */
export function isImeComposing(e: {
  isComposing?: boolean;
  keyCode?: number;
  nativeEvent?: { isComposing?: boolean };
}): boolean {
  return Boolean(e.nativeEvent?.isComposing ?? e.isComposing) || e.keyCode === 229;
}
